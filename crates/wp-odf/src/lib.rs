//! Reading OpenDocument text documents — `.odt`.
//!
//! **The format is a specification rather than a behaviour.** `.doc` was
//! archaeology and `.docx` is Word's own dialect of a standard it wrote; ODF is
//! a document anybody may read, and every decision in here can cite a clause
//! instead of a measurement. Where this crate departs from the letter of it,
//! the comment says which clause and why.
//!
//! **What is read.** The text, with its paragraphs, headings and spans. Both
//! stylesheets — the named styles of `styles.xml` and the automatic styles that
//! direct formatting is written as — kept as styles rather than flattened into
//! the paragraphs that point at them. The page: size, orientation, margins,
//! columns, and the header and footer of every master page, with the margin
//! conversion `page.rs` sets out. Lists, both the definitions and the nesting
//! that says which level a paragraph is at. Tables, with their columns, spanned
//! cells and borders. Frames and the pictures in them. Footnotes and endnotes.
//! Bookmarks. Tab stops. The faces the document names, and the ones it carries.
//!
//! **What is not.** Change tracking, forms, embedded objects and charts, and
//! the drawing layer beyond a frame holding a picture. None of it is dropped: a
//! part this crate does not model is held as it arrived and written back byte
//! for byte, and an element inside a part it does model is skipped for reading
//! and kept for writing. "Unsupported" means "survives".
//!
//! The shape of the API is `wp-docx`'s, because an `.odt` is a package the way
//! a `.docx` is: [`open`] hands back the document *and the container it came
//! from*, and the container is what a save writes through.

use std::path::Path;

pub mod container;
pub mod manifest;

mod content;
mod draw;
mod fonts;
mod list;
mod page;
mod props;
mod styles;
mod table;
mod xml;

pub use container::{Container, Part};

/// The version of the standard this crate writes, and the newest it knows.
///
/// OpenDocument v1.4 became an OASIS Standard on 6 October 2025. A document
/// that declares an older version is read as what it says it is — nothing here
/// upgrades one — and only a document this crate authors from nothing is
/// written as 1.4.
pub const ODF_VERSION: &str = "1.4";

#[derive(Debug)]
pub enum Error {
    /// Not a zip, or a zip with nothing recognisable in it.
    NotAPackage(String),
    /// No `mimetype` entry, which every ODF package has and which is the one
    /// thing that tells an `.odt` from any other zip without unpacking it.
    NoMimetype,
    /// A package this cannot read because its parts are ciphertext. Named as
    /// locked rather than reported as broken.
    Encrypted,
    /// A mimetype this crate has no reader for — a spreadsheet or a
    /// presentation, which are ODF too.
    NotATextDocument(String),
    /// No `content.xml`.
    MissingPart(&'static str),
    Xml(String),
    BadPartName(String),
    Io(std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotAPackage(why) => write!(f, "not an OpenDocument package: {why}"),
            Error::NoMimetype => write!(f, "not an OpenDocument package: it has no mimetype"),
            Error::Encrypted => write!(
                f,
                "this document is encrypted, and nothing here can unlock it"
            ),
            Error::NotATextDocument(kind) => {
                write!(f, "this is an OpenDocument {kind}, not a text document")
            }
            Error::MissingPart(part) => write!(f, "the package has no {part}"),
            Error::Xml(detail) => write!(f, "{detail}"),
            Error::BadPartName(raw) => write!(f, "{raw} is not a name a part can have"),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::Io(e)
    }
}

impl From<ooxml::Error> for Error {
    fn from(e: ooxml::Error) -> Error {
        Error::BadPartName(e.to_string())
    }
}

/// A picture read out of an `.odt`, and the name the document's drawings call
/// it by.
///
/// ODF has no relationships: a frame names its picture by the path it sits at
/// inside the package. The model names a picture by a relationship, because
/// that is how every other reader here hands one over, so the names are minted
/// while reading and the bytes come out alongside the document. Exactly what
/// `wp_doc::Media` does, and for the same reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Media {
    pub rel: String,
    pub data: Vec<u8>,
    pub content_type: &'static str,
}

/// One face of one family, ready to hand to a text shaper.
///
/// The same shape `wp_docx::EmbeddedFace` has, so that a caller wanting the
/// faces a document carries asks the same question of either format. Unlike a
/// `.docx`, the bytes need no unlocking: ECMA-376 obfuscates an embedded face
/// and ODF simply stores it.
#[derive(Debug, Clone)]
pub struct EmbeddedFace {
    /// The family as the document's runs name it — `Ubuntu`, not `Ubuntu Bold`.
    pub family: String,
    pub bold: bool,
    pub italic: bool,
    pub bytes: Vec<u8>,
}

/// Every face the package carries.
///
/// A face whose part is missing is skipped rather than reported: the document
/// still opens, and the only cost is that this one family falls back to a
/// substitute, exactly as it would have done had it never been carried at all.
pub fn embedded(container: &Container) -> Vec<EmbeddedFace> {
    let mut faces = fonts::FontFaces::default();
    for part in ["styles.xml", "content.xml"] {
        let Some(bytes) = container.data(part) else {
            continue;
        };
        let Ok(text) = std::str::from_utf8(bytes) else {
            continue;
        };
        let mut reader = quick_xml::Reader::from_str(text);
        loop {
            match reader.read_event() {
                Ok(quick_xml::events::Event::Start(e))
                    if xml::local_name(&e) == b"font-face-decls" =>
                {
                    fonts::declarations(&mut reader, &mut faces)
                }
                Ok(quick_xml::events::Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
    }
    faces
        .embedded()
        .iter()
        .filter_map(|face| {
            Some(EmbeddedFace {
                family: face.family.to_string(),
                bold: face.bold,
                italic: face.italic,
                bytes: container.data(&face.href)?.to_vec(),
            })
        })
        .collect()
}

/// Opens an `.odt` as a document the rest of the application can lay out, with
/// the pictures it draws and the package it came out of.
pub fn open(path: impl AsRef<Path>) -> Result<(wp_model::Document, Vec<Media>, Container)> {
    let container = Container::open(path)?;
    let (document, media) = read(&container)?;
    Ok((document, media, container))
}

/// The document a package holds.
pub fn read(container: &Container) -> Result<(wp_model::Document, Vec<Media>)> {
    if container.mimetype() != container::TEXT_MIMETYPE {
        let kind = container
            .mimetype()
            .rsplit('.')
            .next()
            .unwrap_or("file")
            .to_string();
        return Err(Error::NotATextDocument(kind));
    }
    let mut ctx = Ctx::new(container);

    // `styles.xml` first, because it holds the named styles, the list and
    // outline definitions, and the master pages — everything `content.xml`
    // points at. Nothing depends on the order, since a name that has not been
    // read yet is interned, but reading it this way round means the interning
    // is a safety net rather than the usual case.
    if let Some(bytes) = container.data("styles.xml") {
        content::part(bytes, &mut ctx, content::Which::Styles)?;
    }
    let body = container
        .data("content.xml")
        .ok_or(Error::MissingPart("content.xml"))?;
    let blocks = content::part(body, &mut ctx, content::Which::Content)?;

    Ok(ctx.finish(blocks))
}

/// Everything one reading of a package accumulates.
///
/// One structure rather than a dozen threaded arguments, and mutable
/// throughout, because reading ODF is a single pass over two parts that refer
/// to each other in both directions.
pub(crate) struct Ctx<'a> {
    pub container: &'a Container,
    pub table: wp_model::StyleTable,
    pub numbering: wp_model::Numbering,
    pub styles: styles::Styles,
    pub media: Vec<Media>,
    /// The rel name a package path has already been given, so that a picture
    /// drawn twice is carried once.
    pub minted: std::collections::HashMap<String, std::sync::Arc<str>>,
    pub headers: Vec<wp_model::doc::HeaderFooter>,
    pub layouts: std::collections::HashMap<String, page::Layout>,
    /// Master pages, in the order the file declares them: the first is the one
    /// the document starts on.
    pub masters: Vec<Master>,
    pub footnotes: Vec<wp_model::doc::Note>,
    pub endnotes: Vec<wp_model::doc::Note>,
    pub bookmarks: std::collections::HashMap<String, u32>,
}

/// A master page: a layout, and the bands drawn on it.
pub(crate) struct Master {
    pub layout: String,
    pub headers: Vec<wp_model::section::HeaderRef>,
    pub footers: Vec<wp_model::section::HeaderRef>,
}

impl<'a> Ctx<'a> {
    fn new(container: &'a Container) -> Ctx<'a> {
        Ctx {
            container,
            table: wp_model::StyleTable::new(),
            numbering: wp_model::Numbering::new(),
            styles: styles::Styles::default(),
            media: Vec::new(),
            minted: std::collections::HashMap::new(),
            headers: Vec::new(),
            layouts: std::collections::HashMap::new(),
            masters: Vec::new(),
            footnotes: Vec::new(),
            endnotes: Vec::new(),
            bookmarks: std::collections::HashMap::new(),
        }
    }

    /// A context over a package with nothing in it, for the readers whose
    /// tests are about one element rather than about a document.
    #[cfg(test)]
    pub(crate) fn for_tests(container: &'a Container) -> Ctx<'a> {
        Ctx::new(container)
    }

    /// The section the document's own page setup comes to.
    ///
    /// The first master page declared, because that is the one a document with
    /// one section is on, and a document with several still starts on it.
    fn section(&self) -> wp_model::section::SectionProps {
        let Some(master) = self.masters.first() else {
            return wp_model::section::SectionProps::default();
        };
        let layout = self
            .layouts
            .get(&master.layout)
            .cloned()
            .unwrap_or_default();
        let mut section = page::section(
            &layout,
            !master.headers.is_empty(),
            !master.footers.is_empty(),
        );
        section.headers = master.headers.clone();
        section.footers = master.footers.clone();
        section
    }

    fn finish(self, body: Vec<wp_model::doc::Block>) -> (wp_model::Document, Vec<Media>) {
        let section = self.section();
        let mut document = wp_model::Document::new();
        document.body = body;
        document.section = section;
        document.styles = self.table;
        document.numbering = self.numbering;
        document.headers = self.headers;
        document.footnotes = self.footnotes;
        document.endnotes = self.endnotes;
        // ODF has no compatibility mode: it is one specification and there is
        // nothing to be compatible with a previous reading of. The flag the
        // layout engine reads for justified spacing is left where it is, which
        // means the oldest behaviour — the right answer here, because the rule
        // it gates was measured out of Word and no clause of ODF asks for it.
        //
        // A band's trailing space is the other way round: it is not in the file
        // either, and the answer is fixed for the format rather than chosen by
        // the document. Measured — see the setting's own note.
        document.settings.bands_keep_trailing_space = false;
        (document, self.media)
    }
}

impl std::fmt::Debug for Ctx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ctx").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A spreadsheet is ODF too, and opening one as a document would show its
    /// cells as an empty page rather than say what it is.
    #[test]
    fn a_package_that_is_not_a_text_document_says_what_it_is() {
        let container = Container::empty("application/vnd.oasis.opendocument.spreadsheet");
        match read(&container) {
            Err(Error::NotATextDocument(kind)) => assert_eq!(kind, "spreadsheet"),
            other => panic!("a spreadsheet must be refused by name: {other:?}"),
        }
    }

    #[test]
    fn a_text_document_with_no_content_says_which_part_is_missing() {
        let container = Container::empty(container::TEXT_MIMETYPE);
        assert!(matches!(
            read(&container),
            Err(Error::MissingPart("content.xml"))
        ));
    }
}
