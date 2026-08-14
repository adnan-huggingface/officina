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
    pub kind: Kind,
}

impl FieldMark {
    fn key(&self) -> (usize, usize) {
        (self.paragraph, self.ordinal)
    }
}

/// What each field evaluates to.
#[derive(Debug, Clone, Default)]
pub struct FieldValues {
    values: HashMap<(usize, usize), Arc<str>>,
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
            kind,
        }
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
