//! Notes attached to cells.
//!
//! Excel has had two of these. A **note** is the yellow box that has been there
//! since 1997: one author, one piece of text, drawn by a shape in a VML part
//! beside the sheet. A **threaded comment** is the newer conversation, stored
//! in its own part — and stored *again* as a note, because a file with threaded
//! comments still has to open in Excel 2010. That duplication is what lets one
//! model cover both: the note is the text every version of Excel can see.

use crate::CellRef;

/// One note on one cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub at: CellRef,
    /// Who wrote it. Excel puts the author's name in bold at the top of the
    /// box and stores it separately, in a list the note refers to by index.
    pub author: String,
    /// The text, with the runs flattened.
    ///
    /// A note's text is rich — Excel bolds the author's name — and the
    /// formatting of it is not modeled: what a reader needs is what it says,
    /// and a note rewritten here is rewritten as plain text on purpose rather
    /// than as an approximation of somebody's italics.
    pub text: String,
}

impl Comment {
    pub fn new(at: CellRef, author: impl Into<String>, text: impl Into<String>) -> Self {
        Comment {
            at,
            author: author.into(),
            text: text.into(),
        }
    }

    /// The text without the author's name repeated at the front.
    ///
    /// Excel writes the author into the body as well as into the author list —
    /// `Adnan:\nlooks high` — and showing that twice in a tooltip that already
    /// names the author reads as a stutter.
    pub fn body(&self) -> &str {
        let Some(rest) = self.text.strip_prefix(&self.author) else {
            return self.text.trim();
        };
        rest.trim_start().strip_prefix(':').unwrap_or(rest).trim()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_authors_own_name_is_not_part_of_what_they_wrote() {
        let note = Comment::new(CellRef::new(0, 0), "Adnan", "Adnan:\nchase this");
        assert_eq!(note.body(), "chase this");

        // A note whose text does not start with the author is left alone.
        let note = Comment::new(CellRef::new(0, 0), "Adnan", "chase this");
        assert_eq!(note.body(), "chase this");

        // And one written by somebody whose name is in the sentence.
        let note = Comment::new(CellRef::new(0, 0), "Sam", "Ask Sam about this");
        assert_eq!(note.body(), "Ask Sam about this");
    }
}
