//! What stands in the text where a note is referenced, and what the note costs.
//!
//! A footnote is two things in two places: a small mark in the running text,
//! and the note itself in a band at the foot of the page the mark landed on.
//! Neither can be laid out without the other — the band's height is what the
//! page has less of, and which page the mark lands on is what pagination
//! decides — so the note is measured while the paragraph holding its mark is
//! flowed, and travels with the item that mark sits on.
//!
//! **The marks are not the ids.** `w:footnoteReference w:id="2"` is the second
//! entry in `footnotes.xml`, and the first two entries of that part are the
//! separator rules rather than notes. The mark a reader sees is the note's
//! position among the *real* notes, counted from one.
//!
//! Word states no numbering format in the demonstration document, and its
//! defaults are not the same for the two kinds: footnotes count in arabic
//! figures and endnotes in lower-case roman numerals. That is why the document
//! reads "Footnotes¹ and endnotesⁱ".

use std::collections::BTreeMap;

use wp_model::doc::{Document, NoteKind};

/// The mark that stands for each note where it is referenced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteMarks {
    /// Keyed by `(is endnote, id)`, because the two kinds number separately
    /// and their ids overlap.
    marks: BTreeMap<(bool, i32), String>,
}

impl NoteMarks {
    /// Numbers every real note in the document, in the order the part lists
    /// them — which is the order Word writes them and the order they are
    /// referenced.
    pub fn of(document: &Document) -> NoteMarks {
        let mut marks = BTreeMap::new();
        for (endnote, notes) in [(false, &document.footnotes), (true, &document.endnotes)] {
            let mut nth = 0usize;
            for note in notes.iter().filter(|n| n.kind == NoteKind::Normal) {
                nth += 1;
                let mark = if endnote { roman(nth) } else { nth.to_string() };
                marks.insert((endnote, note.id), mark);
            }
        }
        NoteMarks { marks }
    }

    pub fn mark(&self, endnote: bool, id: i32) -> Option<&str> {
        self.marks.get(&(endnote, id)).map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }
}

/// Lower-case roman numerals, which is what Word numbers endnotes in when the
/// document says nothing.
fn roman(mut n: usize) -> String {
    const TABLE: [(usize, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut out = String::new();
    for (value, numeral) in TABLE {
        while n >= value {
            out.push_str(numeral);
            n -= value;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::doc::Note;

    fn note(id: i32, kind: NoteKind) -> Note {
        Note {
            id,
            kind,
            content: Vec::new(),
        }
    }

    #[test]
    fn the_separators_are_not_counted_and_the_notes_start_at_one() {
        let mut document = Document::new();
        // What Word writes: the separator and its continuation first, under
        // ids that are not note numbers, then the notes themselves.
        document.footnotes = vec![
            note(-1, NoteKind::Separator),
            note(0, NoteKind::ContinuationSeparator),
            note(2, NoteKind::Normal),
            note(3, NoteKind::Normal),
        ];
        let marks = NoteMarks::of(&document);
        assert_eq!(marks.mark(false, 2), Some("1"), "the first real note is 1");
        assert_eq!(marks.mark(false, 3), Some("2"));
        assert_eq!(marks.mark(false, 0), None, "a separator has no mark");
    }

    #[test]
    fn endnotes_count_in_roman_and_apart_from_the_footnotes() {
        let mut document = Document::new();
        document.footnotes = vec![note(2, NoteKind::Normal)];
        document.endnotes = vec![note(2, NoteKind::Normal), note(3, NoteKind::Normal)];
        let marks = NoteMarks::of(&document);
        assert_eq!(marks.mark(false, 2), Some("1"));
        assert_eq!(
            marks.mark(true, 2),
            Some("i"),
            "the same id, the other kind"
        );
        assert_eq!(marks.mark(true, 3), Some("ii"));
    }

    #[test]
    fn roman_numerals_read_the_way_they_are_written() {
        let seen: Vec<String> = (1..=12).map(roman).collect();
        assert_eq!(
            seen,
            ["i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x", "xi", "xii"]
        );
        assert_eq!(roman(49), "xlix");
        assert_eq!(roman(1994), "mcmxciv");
    }
}
