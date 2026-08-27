//! Finding text in the document, and putting something else in its place.
//!
//! The search is case-insensitive and stays inside one paragraph, which is what
//! Word's plain Find does — matching across a paragraph mark needs the special
//! codes this does not offer. Matching is by characters rather than by lowering
//! the whole text, because lowercasing can change a string's byte length and an
//! offset into the lowered text names nothing in the document.

use wp_model::{Document, Scope};

use crate::edit::{Caret, Selection};

/// One match: where it is, and which of the document's flows it is in.
///
/// **A document has more than one story to search.** Word looks through the
/// text and then through the headers and footers, and a reader who has put a
/// spec number in a header expects Find to reach it. A `Selection` alone
/// cannot say where it was found — every flow counts its paragraphs from
/// zero — so the scope travels with it, all the way to the highlight the
/// painter draws and the band Find Next opens.
pub type Found = (Scope, Selection);

/// The state of the find bar: what is being looked for, and what would go in
/// its place.
#[derive(Debug, Clone, Default)]
pub struct Finder {
    pub query: String,
    pub replacement: String,
    /// Whether the bar shows the replace controls too.
    pub with_replace: bool,
    /// Ask the bar to focus the query field on the next frame.
    pub focus: bool,
    /// A short answer shown in the bar — "Replaced 12" — cleared by typing.
    pub note: Option<String>,
}

impl Finder {
    pub fn new(with_replace: bool) -> Finder {
        Finder {
            with_replace,
            focus: true,
            ..Finder::default()
        }
    }
}

/// Every match in the document — the text first, then each header and footer
/// — in the order Find Next walks them.
pub fn matches(document: &Document, query: &str) -> Vec<Found> {
    let mut out = Vec::new();
    if query.is_empty() {
        return out;
    }
    for scope in document.flows() {
        for (index, paragraph) in document.paragraphs_in(scope).iter().enumerate() {
            let text = paragraph.text();
            for range in find_in(&text, query) {
                out.push((
                    scope,
                    Selection {
                        anchor: Caret {
                            paragraph: index,
                            offset: range.start,
                        },
                        head: Caret {
                            paragraph: index,
                            offset: range.end,
                        },
                    },
                ));
            }
        }
    }
    // `Scope` orders the body before every band, so this is the order the
    // matches were gathered in unless the headers arrived out of order — and
    // Find Next must walk them the same way every time whatever the file did.
    out.sort_by_key(|(scope, at)| (*scope, at.ordered().0));
    out
}

/// The first match starting at or after `from`, wrapping to the start when
/// there is none. Callers pass the selection's *end*, so a match already
/// selected is stepped past rather than found again.
pub fn after(matches: &[Found], scope: Scope, from: Caret) -> Option<Found> {
    matches
        .iter()
        .copied()
        .find(|(at, found)| (*at, found.ordered().0) >= (scope, from))
        .or_else(|| matches.first().copied())
}

/// The last match before `from`, wrapping to the end when there is none.
pub fn before(matches: &[Found], scope: Scope, from: Caret) -> Option<Found> {
    matches
        .iter()
        .rev()
        .copied()
        .find(|(at, found)| (*at, found.ordered().0) < (scope, from))
        .or_else(|| matches.last().copied())
}

/// Whether `text` is exactly the query, ignoring case — what decides if the
/// selection is a match that Replace may replace.
pub fn equals(text: &str, query: &str) -> bool {
    match_len(text, query) == Some(text.len())
}

/// The byte ranges where `query` occurs in `text`, ignoring case.
/// Non-overlapping, like Word: "aa" occurs once in "aaa".
fn find_in(text: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < text.len() {
        match match_len(&text[at..], query) {
            Some(len) if len > 0 => {
                out.push(at..at + len);
                at += len;
            }
            _ => {
                at += text[at..].chars().next().map(char::len_utf8).unwrap_or(1);
            }
        }
    }
    out
}

/// How many bytes at the start of `text` the query matches, ignoring case.
fn match_len(text: &str, query: &str) -> Option<usize> {
    let mut hay = text.char_indices();
    let mut needle = query.chars();
    loop {
        let Some(want) = needle.next() else {
            return Some(hay.next().map(|(i, _)| i).unwrap_or(text.len()));
        };
        let (_, have) = hay.next()?;
        if !have.to_lowercase().eq(want.to_lowercase()) {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::doc::{Block, Paragraph};

    fn document(texts: &[&str]) -> Document {
        Document {
            body: texts
                .iter()
                .map(|text| Block::Paragraph(Paragraph::of(text)))
                .collect(),
            ..Document::new()
        }
    }

    #[test]
    fn a_search_ignores_case_and_finds_every_occurrence() {
        let document = document(&["The engineer engineered.", "No match here", "Engineer!"]);
        let found = matches(&document, "engineer");
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].1.ordered().0.paragraph, 0);
        assert_eq!(found[0].1.ordered().0.offset, 4);
        assert_eq!(found[1].1.ordered().0.offset, 13);
        assert_eq!(found[2].1.ordered().0.paragraph, 2);
    }

    #[test]
    fn matches_do_not_overlap() {
        assert_eq!(find_in("aaa", "aa"), vec![0..2]);
    }

    #[test]
    fn an_empty_query_matches_nothing() {
        let document = document(&["anything"]);
        assert!(matches(&document, "").is_empty());
        assert!(!equals("anything", ""));
    }

    #[test]
    fn next_and_previous_wrap_around() {
        let document = document(&["one two one"]);
        let found = matches(&document, "one");
        assert_eq!(found.len(), 2);
        let start = Caret {
            paragraph: 0,
            offset: 0,
        };
        let body = Scope::Body;
        let first = after(&found, body, start).expect("a match");
        assert_eq!(first.1.ordered().0.offset, 0, "a match at the caret counts");
        let second = after(&found, body, first.1.ordered().1).expect("a match");
        assert_eq!(
            second.1.ordered().0.offset,
            8,
            "stepped past the selected one"
        );
        let wrapped = after(&found, body, second.1.ordered().1).expect("a match");
        assert_eq!(wrapped.1.ordered().0.offset, 0, "wrapped to the start");
        let back = before(&found, body, start).expect("a match");
        assert_eq!(back.1.ordered().0.offset, 8, "wrapped to the end");
    }

    #[test]
    fn matching_survives_characters_whose_case_changes_their_length() {
        // 'İ' lowercases to two code points; byte offsets must still name the
        // original text.
        let document = document(&["İstanbul istanbul"]);
        let found = matches(&document, "İstanbul");
        assert_eq!(found.len(), 1, "the plain 'i' does not round-trip to İ");
        assert_eq!(found[0].1.ordered().0.offset, 0);
        assert!(equals("İstanbul", "İSTANBUL"));
    }
}
