//! What a field evaluates to, and the two passes that make a page number right.
//!
//! **A page number cannot be known before the page exists.** `{ PAGE }` is on
//! some page, and which page it is on depends on how tall everything before it
//! turned out to be — including the page numbers themselves, if they are wide
//! enough to change a line break. Word lays the document out, updates the
//! fields, and lays it out again; so does this, and it stops after the second
//! pass rather than iterating to a fixed point, because a document where a page
//! number changes the pagination that changes the page number is a document
//! whose answer is a matter of taste.
//!
//! The first pass draws whatever the file cached, which is what Word shows for a
//! field it cannot recompute either.

use std::collections::HashMap;
use std::sync::Arc;

use wp_model::field::Kind;

/// Which field a laid-out fragment is the result of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FieldMark {
    /// The paragraph's index in `Document::paragraphs`.
    pub paragraph: usize,
    /// Which field of that paragraph, counted from the `begin` so that a field
    /// nested inside another keeps a stable number.
    pub ordinal: usize,
    /// Which band this field was laid out in: `None` in the document body,
    /// `Some(page)` in the header or footer drawn on that page.
    ///
    /// A header is laid out again for every page it appears on, from the same
    /// paragraphs — so without this, the `{ PAGE }` in a footer would be one
    /// field asked the same question on every page, and every page would show
    /// the number of the last one. The paragraph indices would collide with the
    /// body's besides, headers not being part of `Document::paragraphs`.
    pub band: Option<u32>,
    pub kind: Kind,
}

impl FieldMark {
    fn key(&self) -> (usize, usize, Option<u32>) {
        (self.paragraph, self.ordinal, self.band)
    }
}

/// Which paragraphs lie inside the result of a table of contents.
///
/// **A contents field opens in one paragraph and closes forty paragraphs
/// later**, so nothing looking at a paragraph on its own can tell that it is
/// one of the entries. This is walked once over the document, the way the note
/// marks are, and consulted while a paragraph is laid out.
///
/// The paragraph a contents field ends in is counted as inside it, which is
/// generous by however much of that paragraph follows the field. Nothing
/// follows it in practice: Word ends a contents field on a paragraph of its
/// own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contents {
    spans: Vec<std::ops::RangeInclusive<usize>>,
}

impl Contents {
    pub fn of(document: &wp_model::Document) -> Contents {
        use wp_model::doc::Piece;
        let mut spans = Vec::new();
        // One entry per open field, so a `HYPERLINK` inside the contents does
        // not close the contents when it ends. The value is where a contents
        // field's result began, and `None` for every other kind of field.
        let mut open: Vec<Option<usize>> = Vec::new();
        let mut instruction = String::new();
        for (index, paragraph) in document.paragraphs().iter().enumerate() {
            for piece in paragraph.runs().iter().flat_map(|run| run.content.iter()) {
                match piece {
                    Piece::FieldStart { .. } => {
                        open.push(None);
                        instruction.clear();
                    }
                    Piece::Instruction(text) => instruction.push_str(text),
                    Piece::FieldSeparate => {
                        let toc = wp_model::Field::parse(&instruction)
                            .is_some_and(|field| field.kind() == Kind::Toc);
                        if let Some(slot) = open.last_mut() {
                            *slot = toc.then_some(index);
                        }
                        instruction.clear();
                    }
                    Piece::FieldEnd => {
                        if let Some(Some(from)) = open.pop() {
                            spans.push(from..=index);
                        }
                    }
                    _ => {}
                }
            }
        }
        // A field left open at the end of the document runs to the end of it,
        // which is what Word draws for a document whose contents field was
        // damaged.
        let last = document.paragraphs().len().saturating_sub(1);
        for from in open.into_iter().flatten() {
            spans.push(from..=last);
        }
        Contents { spans }
    }

    /// Whether this paragraph is one of a table of contents' entries.
    pub fn holds(&self, paragraph: usize) -> bool {
        self.spans.iter().any(|span| span.contains(&paragraph))
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }
}

/// What each field evaluates to.
#[derive(Debug, Clone, Default)]
pub struct FieldValues {
    values: HashMap<(usize, usize, Option<u32>), Arc<str>>,
    /// What `{ DATE }` and `{ TIME }` show. Supplied by the application rather
    /// than read here: a layout that read the clock could not be tested, and a
    /// document laid out twice would differ for no reason the user caused.
    pub today: Option<Arc<str>>,
    pub now: Option<Arc<str>>,
    /// What `{ FILENAME }`, `{ AUTHOR }` and `{ TITLE }` show.
    pub file_name: Option<Arc<str>>,
    pub author: Option<Arc<str>>,
    pub title: Option<Arc<str>>,
}

impl FieldValues {
    pub fn new() -> FieldValues {
        FieldValues::default()
    }

    /// A fresh set of values keeping only what does not depend on pagination —
    /// the date, the file name and the rest, which the application supplied and
    /// a second pass must not lose.
    pub fn carrying(from: &FieldValues) -> FieldValues {
        FieldValues {
            values: HashMap::new(),
            today: from.today.clone(),
            now: from.now.clone(),
            file_name: from.file_name.clone(),
            author: from.author.clone(),
            title: from.title.clone(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn set(&mut self, mark: FieldMark, value: impl Into<Arc<str>>) {
        self.values.insert(mark.key(), value.into());
    }

    /// The text to draw instead of the cached result, if there is one.
    pub fn get(&self, mark: FieldMark) -> Option<&str> {
        if let Some(value) = self.values.get(&mark.key()) {
            return Some(value);
        }
        // The fields whose answer does not depend on the page they are on can
        // be answered without a second pass at all.
        match mark.kind {
            Kind::Date => self.today.as_deref(),
            Kind::Time => self.now.as_deref(),
            Kind::FileName => self.file_name.as_deref(),
            Kind::Author => self.author.as_deref(),
            Kind::Title => self.title.as_deref(),
            _ => None,
        }
    }

    /// Whether this field's answer needs the document to have been paginated.
    pub fn needs_pages(kind: Kind) -> bool {
        matches!(kind, Kind::Page | Kind::NumPages | Kind::SectionPages)
    }
}

impl FieldValues {
    /// Whether these are the values the layout was already given.
    ///
    /// Only the field results matter — the application's own strings are
    /// carried through unchanged and cannot differ between the two passes.
    pub fn same_as(&self, other: &FieldValues) -> bool {
        self.values.len() == other.values.len()
            && self
                .values
                .iter()
                .all(|(key, value)| other.values.get(key) == Some(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(ordinal: usize, kind: Kind) -> FieldMark {
        FieldMark {
            paragraph: 0,
            ordinal,
            band: None,
            kind,
        }
    }

    /// A paragraph of pieces, as a document's body.
    fn body(pieces: Vec<Vec<wp_model::doc::Piece>>) -> wp_model::Document {
        use wp_model::doc::{Block, Inline, Paragraph, Run};
        let mut document = wp_model::Document::new();
        document.body = pieces
            .into_iter()
            .map(|content| {
                let mut run = Run::new();
                run.content = content;
                let mut paragraph = Paragraph::new();
                paragraph.content = vec![Inline::Run(run)];
                Block::Paragraph(paragraph)
            })
            .collect();
        document
    }

    #[test]
    fn a_contents_field_holds_every_paragraph_between_its_ends() {
        use wp_model::doc::Piece;
        // A contents field opens in one paragraph, sets its entries in the
        // next forty, and closes in the last. Nothing looking at one paragraph
        // can tell that the middle ones are inside it.
        let document = body(vec![
            vec![Piece::Text("before".into())],
            vec![
                Piece::FieldStart {
                    dirty: false,
                    lock: false,
                },
                Piece::Instruction(r#" TOC \o "1-3" \h "#.into()),
                Piece::FieldSeparate,
                Piece::Text("first entry".into()),
            ],
            vec![Piece::Text("second entry".into())],
            vec![Piece::Text("third entry".into()), Piece::FieldEnd],
            vec![Piece::Text("after".into())],
        ]);
        let contents = Contents::of(&document);
        assert!(!contents.holds(0));
        assert!(contents.holds(1));
        assert!(contents.holds(2));
        assert!(contents.holds(3));
        assert!(!contents.holds(4));
    }

    #[test]
    fn a_hyperlink_inside_the_contents_does_not_close_it() {
        use wp_model::doc::Piece;
        // Every entry is a `HYPERLINK` field of its own, and a reader that
        // pops the stack on the first end it meets stops counting a paragraph
        // in.
        let document = body(vec![
            vec![
                Piece::FieldStart {
                    dirty: false,
                    lock: false,
                },
                Piece::Instruction(r" TOC \h ".into()),
                Piece::FieldSeparate,
                Piece::FieldStart {
                    dirty: false,
                    lock: false,
                },
                Piece::Instruction(r#"HYPERLINK \l "_Toc1""#.into()),
                Piece::FieldSeparate,
                Piece::Text("one".into()),
                Piece::FieldEnd,
            ],
            vec![Piece::Text("two".into())],
            vec![Piece::FieldEnd],
        ]);
        let contents = Contents::of(&document);
        assert!(contents.holds(1), "the second entry is still inside");
        assert!(contents.holds(2));
    }

    #[test]
    fn an_ordinary_field_is_not_a_table_of_contents() {
        use wp_model::doc::Piece;
        let document = body(vec![vec![
            Piece::FieldStart {
                dirty: false,
                lock: false,
            },
            Piece::Instruction(" PAGE ".into()),
            Piece::FieldSeparate,
            Piece::Text("1".into()),
            Piece::FieldEnd,
        ]]);
        assert!(Contents::of(&document).is_empty());
    }

    #[test]
    fn a_field_with_no_value_falls_back_to_what_the_file_cached() {
        let values = FieldValues::new();
        assert_eq!(values.get(mark(0, Kind::Page)), None);
    }

    #[test]
    fn the_second_pass_answers_the_fields_the_first_could_not() {
        let mut values = FieldValues::new();
        values.set(mark(0, Kind::Page), "7");
        assert_eq!(values.get(mark(0, Kind::Page)), Some("7"));
        assert_eq!(values.get(mark(1, Kind::Page)), None, "a different field");
    }

    #[test]
    fn a_date_is_supplied_rather_than_read_from_the_clock() {
        // A layout that read the clock could not be tested, and the same
        // document laid out twice would differ for no reason the user caused.
        let mut values = FieldValues::new();
        assert_eq!(values.get(mark(0, Kind::Date)), None);
        values.today = Some("14 August 2026".into());
        assert_eq!(values.get(mark(0, Kind::Date)), Some("14 August 2026"));
    }

    #[test]
    fn only_the_page_fields_need_the_document_to_have_been_paginated() {
        assert!(FieldValues::needs_pages(Kind::Page));
        assert!(FieldValues::needs_pages(Kind::NumPages));
        assert!(!FieldValues::needs_pages(Kind::Date));
        assert!(!FieldValues::needs_pages(Kind::Other));
    }
}
