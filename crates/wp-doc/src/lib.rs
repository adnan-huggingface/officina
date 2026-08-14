//! Reading Word 97–2003 `.doc` files.
//!
//! **Read-only, and deliberately so.** A `.doc` is a memory image with a fast
//! save log on the end of it: the piece table, the bin tables of property
//! exceptions and the plexes all point at byte offsets in a stream that a writer
//! would have to keep valid. Writing one back would mean rebuilding every one of
//! those indices, and getting one wrong makes a file that Word opens and shows
//! as something else — the failure mode this project exists to avoid.
//!
//! So a `.doc` opens, and saving offers `.docx` instead. That is the honest
//! trade, and it is stated in the application rather than discovered.
//!
//! **What is read.** The text, through the piece table, with the parts told
//! apart (body, footnotes, headers, annotations, endnotes, text boxes). The
//! paragraphs, from the paragraph marks. Direct character formatting — bold,
//! italic, underline, size, font, colour — and paragraph formatting —
//! alignment, indents, spacing, the style it is in — from the bin tables of
//! property exceptions. Tables, from the cell and row marks.
//!
//! **What is not.** Pictures, drawings, fields, footnote *references* (the notes
//! themselves are read, but not which character they hang off), revision marks,
//! and the stylesheet's own definitions. A document opened here shows its words
//! and its shape; it does not claim to be the same document.

use std::path::Path;

pub mod fib;
pub mod fkp;
pub mod piece;
pub mod sprm;
pub mod style;
pub mod text;

pub use fib::{Counts, Fib, Part};
pub use piece::{Piece, Pieces};

#[derive(Debug)]
pub enum Error {
    /// The file is not a compound file at all.
    Container(String),
    /// It is a compound file, but not a Word document.
    NotADocument,
    /// Word 6 or Word 95, whose piece table is a different structure.
    TooOld(u16),
    /// Password-protected: there is nothing to read without the password.
    Encrypted,
    Malformed(&'static str),
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Container(why) => write!(f, "not a Word document: {why}"),
            Error::NotADocument => write!(f, "this file is not a Word 97-2003 document"),
            Error::TooOld(version) => write!(
                f,
                "this document was written by Word 6 or Word 95 (version {version}), \
                 which stores its text differently"
            ),
            Error::Encrypted => write!(f, "this document is password-protected"),
            Error::Malformed(why) => write!(f, "this document is damaged: {why}"),
            Error::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Error {
        Error::Io(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Everything read out of one `.doc`.
pub struct Doc {
    pub fib: Fib,
    pub pieces: Pieces,
    /// The `WordDocument` stream, which the piece offsets are into.
    pub stream: Vec<u8>,
    /// The table stream the FIB chose.
    pub table: Vec<u8>,
}

impl Doc {
    /// The text of one part of the document.
    pub fn part(&self, part: Part) -> String {
        let Some((_, from, to)) = self
            .fib
            .counts
            .ranges()
            .into_iter()
            .find(|(found, _, _)| *found == part)
        else {
            return String::new();
        };
        self.pieces.text(&self.stream, from, to)
    }
}

/// Opens a `.doc` and reads what can be read out of it.
pub fn read(path: impl AsRef<Path>) -> Result<Doc> {
    let container =
        cfb_reader::Cfb::open(path).map_err(|error| Error::Container(error.to_string()))?;
    let stream = container
        .stream("WordDocument")
        .map_err(|error| Error::Container(error.to_string()))?
        .ok_or(Error::NotADocument)?;
    let fib = Fib::read(&stream)?;
    if fib.encrypted {
        return Err(Error::Encrypted);
    }
    let table = container
        .stream(fib.table_stream())
        .map_err(|error| Error::Container(error.to_string()))?
        .ok_or(Error::Malformed(
            "the table stream the FIB names is missing",
        ))?;
    let pieces = Pieces::read(&fib, &table)?;
    Ok(Doc {
        fib,
        pieces,
        stream,
        table,
    })
}

/// Opens a `.doc` as a document the rest of the application can lay out.
pub fn open(path: impl AsRef<Path>) -> Result<wp_model::Document> {
    let doc = read(path)?;
    Ok(text::document(&doc))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_file_that_is_not_a_compound_file_says_so_rather_than_panicking() {
        let path = std::env::temp_dir().join("wp-doc-not-a-doc.doc");
        std::fs::write(&path, b"this is a text file").expect("written");
        let outcome = super::read(&path);
        let _ = std::fs::remove_file(&path);
        assert!(matches!(outcome, Err(super::Error::Container(_))));
    }
}
