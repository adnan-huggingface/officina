//! Tracked changes, comments, and bookmarks.
//!
//! These are in the model from the first chunk rather than from the one that
//! makes them editable, and that is a deliberate ordering. A reader that
//! flattens a tracked deletion has destroyed it: the text is gone, the author is
//! gone, and no later chunk can bring either back. Preservation has to be built
//! before the feature, not after — the same reasoning as the Preservation Vault
//! itself.
//!
//! Three shapes appear, and they are genuinely different:
//!
//! - **A wrapper.** `<w:ins>` and `<w:del>` contain the runs they are about, so
//!   an insertion is a property of a span of content.
//! - **A property change.** `<w:rPrChange>` and `<w:pPrChange>` sit *inside* the
//!   properties they changed and hold the *previous* ones, so rejecting the
//!   change is putting them back.
//! - **A mark.** `<w:commentRangeStart>` and `<w:bookmarkStart>` are empty
//!   elements at a position, with a matching end somewhere later — possibly in
//!   a different paragraph, a different table cell, or a different section.
//!   Nothing guarantees they nest.

use std::sync::Arc;

use crate::prop::{ParaProps, RunProps};

/// Who made a change and when.
///
/// The date is kept as the file's own text rather than parsed into a timestamp.
/// It is an ISO 8601 string, it is written back verbatim, and a document whose
/// date is `0001-01-01T00:00:00Z` — which Word writes when the privacy setting
/// strips them — must come back saying exactly that rather than a date we chose
/// to represent "none" with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    /// `w:id`. Unique within its own kind and no further; an insertion and a
    /// comment may share the number 3.
    pub id: u32,
    pub author: Arc<str>,
    pub date: Option<Arc<str>>,
}

impl Mark {
    pub fn new(id: u32, author: impl Into<Arc<str>>) -> Mark {
        Mark {
            id,
            author: author.into(),
            date: None,
        }
    }
}

/// A tracked change wrapping a span of content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Revision {
    /// `<w:ins>` — content added. Shown unless changes are hidden; removed by
    /// rejecting.
    Inserted(Mark),
    /// `<w:del>` — content removed. **The text is still there**, in `<w:delText>`
    /// rather than `<w:t>`, and it is drawn struck through until the change is
    /// accepted. A reader that treats `delText` as ordinary text un-deletes it;
    /// one that skips it loses the ability to reject.
    Deleted(Mark),
    /// `<w:moveFrom>` — the origin of a move, paired by name with a
    /// `<w:moveTo>`. Word draws it as a deletion in a different colour and
    /// accepts both halves together.
    MovedFrom { mark: Mark, name: Arc<str> },
    /// `<w:moveTo>` — the destination.
    MovedTo { mark: Mark, name: Arc<str> },
}

impl Revision {
    pub fn mark(&self) -> &Mark {
        match self {
            Revision::Inserted(mark)
            | Revision::Deleted(mark)
            | Revision::MovedFrom { mark, .. }
            | Revision::MovedTo { mark, .. } => mark,
        }
    }

    /// Whether the content inside is present in the document as it stands —
    /// i.e. before anything is accepted or rejected.
    ///
    /// A deletion's content is *drawn*, struck through, but it is not part of
    /// the text: a word count, a search, and the flowed length all have to skip
    /// it, and every one of those is a place this question gets asked.
    pub const fn is_present(&self) -> bool {
        matches!(self, Revision::Inserted(_) | Revision::MovedTo { .. })
    }

    /// Whether accepting this change keeps the content.
    pub const fn survives_accept(&self) -> bool {
        self.is_present()
    }

    /// Whether rejecting it keeps the content.
    pub const fn survives_reject(&self) -> bool {
        !self.is_present()
    }
}

/// A change to formatting rather than to content: `<w:rPrChange>`,
/// `<w:pPrChange>` and their table equivalents.
///
/// The properties held are the **previous** ones. That is the whole design of
/// the element and it is counter-intuitive enough to be worth the type: the
/// current formatting is where it always was, and this is what rejecting puts
/// back.
#[derive(Debug, Clone, PartialEq)]
pub struct PropChange {
    pub mark: Mark,
    pub previous: PreviousProps,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PreviousProps {
    Run(Box<RunProps>),
    Paragraph(Box<ParaProps>),
    /// A table, row or cell property change. Not modelled in detail — the
    /// element is preserved by the writer and this records that one was there,
    /// so accepting or rejecting can refuse rather than silently doing nothing.
    Table,
}

/// One `<w:comment>` — the note itself, which lives in `comments.xml` rather
/// than in the document.
///
/// The document holds only the anchors. That separation is why a comment can
/// span a range: the range is marked in the text and the prose is elsewhere.
#[derive(Debug, Clone)]
pub struct Comment {
    pub id: u32,
    pub author: Arc<str>,
    /// `w:initials` — what the balloon is labelled with.
    pub initials: Option<Arc<str>>,
    pub date: Option<Arc<str>>,
    /// `w:done` — modern Word marks a comment resolved rather than deleting it,
    /// in `commentsExtended.xml`. Read here so a resolved comment is not drawn
    /// as an open one.
    pub done: bool,
    /// The comment's own body: paragraphs, and occasionally a table.
    pub content: Vec<crate::doc::Block>,
}

impl Comment {
    pub fn new(id: u32, author: impl Into<Arc<str>>) -> Comment {
        Comment {
            id,
            author: author.into(),
            initials: None,
            date: None,
            done: false,
            content: Vec::new(),
        }
    }

    /// The comment's text, for a list or a search.
    pub fn text(&self) -> String {
        crate::doc::text_of(&self.content)
    }
}

/// An empty element that marks a position in the text.
///
/// These do not nest and are not guaranteed to be balanced. A `<w:bookmarkEnd>`
/// may appear before its start in document order in a file that has been edited
/// enough times, and Word opens it. Modelling them as a tree would mean either
/// rejecting such a document or inventing structure it does not have; they are
/// kept as what they are, points in a sequence, and matched up by id when
/// something needs the range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anchor {
    /// `<w:bookmarkStart w:id="0" w:name="_GoBack"/>`. The name is the identity
    /// a cross-reference uses; the id only pairs the start with its end.
    BookmarkStart {
        id: u32,
        name: Arc<str>,
    },
    BookmarkEnd {
        id: u32,
    },
    CommentStart {
        id: u32,
    },
    CommentEnd {
        id: u32,
    },
    /// `<w:commentReference>` — where the balloon's line points. Inside a run,
    /// unlike the range marks, and carrying the run's formatting.
    CommentReference {
        id: u32,
    },
    /// `<w:permStart>` / `<w:permEnd>` — an editable region inside a protected
    /// document. Preserved rather than enforced; enforcing it without also
    /// implementing document protection would be a lock with no door.
    PermissionStart {
        id: Arc<str>,
        editor: Option<Arc<str>>,
    },
    PermissionEnd {
        id: Arc<str>,
    },
}

impl Anchor {
    /// Whether this marks the beginning of a range.
    pub const fn is_start(&self) -> bool {
        matches!(
            self,
            Anchor::BookmarkStart { .. }
                | Anchor::CommentStart { .. }
                | Anchor::PermissionStart { .. }
        )
    }
}

/// Everything a document knows about who has been in it.
///
/// `people.xml` — the part that appears beside a document with tracked changes —
/// maps an author's name to a presence provider id. Not interpreted; carried so
/// the writer can put it back, and so an author list can be built without
/// walking the whole document.
#[derive(Debug, Clone, Default)]
pub struct People {
    pub authors: Vec<Arc<str>>,
}

impl People {
    pub fn record(&mut self, author: &Arc<str>) {
        if !self.authors.iter().any(|known| known == author) {
            self.authors.push(author.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deletion_is_drawn_but_is_not_part_of_the_text() {
        let deleted = Revision::Deleted(Mark::new(1, "Adnan Khan"));
        assert!(!deleted.is_present());
        assert!(!deleted.survives_accept());
        assert!(deleted.survives_reject());
    }

    #[test]
    fn an_insertion_survives_accepting_and_a_deletion_survives_rejecting() {
        let inserted = Revision::Inserted(Mark::new(0, "Adnan Khan"));
        assert!(inserted.is_present());
        assert!(inserted.survives_accept());
        assert!(!inserted.survives_reject());
    }

    #[test]
    fn a_move_is_a_deletion_and_an_insertion_that_know_each_others_name() {
        let from = Revision::MovedFrom {
            mark: Mark::new(4, "Adnan Khan"),
            name: "move1".into(),
        };
        let to = Revision::MovedTo {
            mark: Mark::new(5, "Adnan Khan"),
            name: "move1".into(),
        };
        assert!(!from.is_present());
        assert!(to.is_present());
        assert_eq!(from.mark().author, to.mark().author);
    }

    #[test]
    fn a_date_the_file_stripped_comes_back_as_the_file_wrote_it() {
        // Word's "remove personal information" writes this exact string rather
        // than dropping the attribute, and a model that parsed dates would have
        // to invent something to put back.
        let mut mark = Mark::new(1, "Author");
        mark.date = Some("0001-01-01T00:00:00Z".into());
        assert_eq!(mark.date.as_deref(), Some("0001-01-01T00:00:00Z"));
    }

    #[test]
    fn a_property_change_holds_what_the_formatting_used_to_be() {
        let change = PropChange {
            mark: Mark::new(2, "Adnan Khan"),
            previous: PreviousProps::Run(Box::default()),
        };
        match &change.previous {
            PreviousProps::Run(props) => assert!(props.is_empty()),
            other => panic!("wrong kind: {other:?}"),
        }
    }

    #[test]
    fn anchors_are_points_rather_than_a_tree() {
        let marks = [
            Anchor::BookmarkStart {
                id: 0,
                name: "_GoBack".into(),
            },
            Anchor::CommentStart { id: 1 },
            Anchor::BookmarkEnd { id: 0 },
            Anchor::CommentEnd { id: 1 },
        ];
        // These overlap and are legal. Nothing here objects.
        assert!(marks[0].is_start());
        assert!(marks[1].is_start());
        assert!(!marks[2].is_start());
    }

    #[test]
    fn the_author_list_does_not_repeat_itself() {
        let mut people = People::default();
        people.record(&"Adnan Khan".into());
        people.record(&"Adnan Khan".into());
        people.record(&"Someone Else".into());
        assert_eq!(people.authors.len(), 2);
    }
}
