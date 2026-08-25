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
//! property exceptions. The font table, without which no run in the file names
//! a face at all — a run says an index and nothing more. The stylesheet's own
//! definitions, paragraph and character both, and what each style is based on.
//! Tables, from the cell and row marks, with the grid, the borders and both
//! kinds of merged cell. The headers and footers. The lists, both the
//! definitions and the instances that stand between them and a paragraph, so
//! that a numbered heading is numbered and an outline reads as one. The drawing
//! layer: pictures, whether they sit in a line or float on the page, and the
//! shapes Word writes a watermark and a page frame with. And the page setup of
//! the first section, so that a document written on A4 does not open as US
//! Letter and reflow from its first line.
//!
//! **What is not.** Pictures stored as metafiles — a Word 97 document keeps a
//! pasted chart or diagram as a deflated EMF, and nothing here plays one, so
//! it takes its right place on the page and draws as a frame. Fields beyond
//! their marks, footnote *references* (the notes themselves are read, but not
//! which character they hang off), revision marks, cell shading, every shape
//! geometry but the rectangle, and the second and later sections' page setup.
//! A document opened here shows its words and its shape; it does not claim to
//! be the same document, and it is opened as a copy rather than as itself.

use std::path::Path;

pub mod art;
pub mod fib;
pub mod fkp;
pub mod font;
pub mod list;
pub mod picture;
pub mod piece;
pub mod section;
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
    /// The `Data` stream, where an inline picture's bytes are. Empty when the
    /// document has none, which is not an error.
    pub data: Vec<u8>,
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
    // A document with no pictures has no `Data` stream at all, and that is
    // ordinary rather than damage.
    let data = container.stream("Data").ok().flatten().unwrap_or_default();
    Ok(Doc {
        fib,
        pieces,
        stream,
        table,
        data,
    })
}

/// A picture read out of a `.doc`, and the name the document's drawings call
/// it by.
///
/// A `.doc` has no package, so there is nothing for a `<a:blip r:embed>` to
/// point into and no part to hold the bytes. The model still names its
/// pictures by relationship — that is how every other reader here hands one
/// over — so the names are minted while reading and the bytes come out
/// alongside the document for the caller to put somewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Media {
    pub rel: String,
    pub data: Vec<u8>,
    pub content_type: &'static str,
}

/// Opens a `.doc` as a document the rest of the application can lay out, with
/// the pictures it draws.
pub fn open(path: impl AsRef<Path>) -> Result<(wp_model::Document, Vec<Media>)> {
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
