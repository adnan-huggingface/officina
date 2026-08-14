//! `.docx` reader and writer, over `ooxml` and `wp-model`.
//!
//! The reader's contract is the project's central one: **parse what we
//! understand, preserve verbatim what we do not.** Opening a document leaves
//! every part of the package retained, so a save with no edits reproduces the
//! original bytes; the model is a view over the package rather than a
//! replacement for it.
//!
//! Two things about the order parts are read in are worth knowing before
//! changing anything here.
//!
//! **`styles.xml` may be read after the document that refers to it.** The
//! relationship order decides, and nothing requires styles to come first. So a
//! `<w:pStyle w:val="Heading1"/>` interns a placeholder id, and the real
//! definition lands *on* that id rather than beside it. [`ctx::Ctx`] is what
//! carries the shared table between them.
//!
//! **A header body is a part, and a section refers to it by relationship id.**
//! The two are joined by an index that hands out a [`HeaderId`] on first sight,
//! which is also what tells the reader which header parts are actually
//! referenced — a package may contain headers no section uses, and Word writes
//! them.
//!
//! [`HeaderId`]: wp_model::section::HeaderId

#![forbid(unsafe_code)]

mod bits;
mod body;
mod ctx;
mod error;
mod notes;
mod numbering;
mod parts;
mod props;
mod styles;
mod xml;

use std::path::Path;

use ooxml::Package;

pub use error::{Error, Result};
pub use parts::DocumentParts;

use ctx::{Ctx, HeaderIndex};
use quick_xml::events::Event;
use quick_xml::Reader;
use wp_model::doc::{Block, Document, HeaderFooter};

/// Opens a `.docx` from disk.
pub fn open(path: impl AsRef<Path>) -> Result<(Document, Package)> {
    let package = Package::open(path)?;
    let document = read(&package)?;
    Ok((document, package))
}

/// Reads a document out of an already-open package.
pub fn read(package: &Package) -> Result<Document> {
    let parts = parts::locate(package)?;
    let mut document = Document::new();
    let mut headers = HeaderIndex::default();

    // Styles first where they exist, so that a `<w:basedOn>` chain is complete
    // before anything resolves against it. Reading them second still works —
    // ids are interned, not invented — but the resolution during a read would
    // see an empty table.
    if let Some(name) = &parts.styles {
        if let Some(part) = package.part(name) {
            let mut ctx = Ctx::new(&mut document.styles, &mut headers);
            styles::read(part.data(), &mut ctx);
        }
    }

    if let Some(name) = &parts.theme {
        if let Some(part) = package.part(name) {
            document.theme = bits::theme(part.data());
        }
    }

    if let Some(name) = &parts.settings {
        if let Some(part) = package.part(name) {
            document.settings = bits::settings(part.data());
        }
    }

    if let Some(name) = &parts.numbering {
        if let Some(part) = package.part(name) {
            let mut ctx = Ctx::new(&mut document.styles, &mut headers);
            document.numbering = numbering::read(part.data(), &mut ctx);
        }
    }

    let main = package
        .part(&parts.document)
        .ok_or_else(|| Error::MissingPart {
            referenced_by: "/_rels/.rels".to_owned(),
            rel_id: "officeDocument".to_owned(),
        })?;
    {
        let mut ctx = Ctx::new(&mut document.styles, &mut headers);
        let (blocks, section) = read_body(main.data(), &mut ctx);
        document.body = blocks;
        if let Some(section) = section {
            document.section = section;
        }
    }

    if let Some(name) = &parts.footnotes {
        if let Some(part) = package.part(name) {
            let mut ctx = Ctx::new(&mut document.styles, &mut headers);
            document.footnotes = notes::read_notes(part.data(), &mut ctx, b"footnote");
        }
    }
    if let Some(name) = &parts.endnotes {
        if let Some(part) = package.part(name) {
            let mut ctx = Ctx::new(&mut document.styles, &mut headers);
            document.endnotes = notes::read_notes(part.data(), &mut ctx, b"endnote");
        }
    }
    if let Some(name) = &parts.comments {
        if let Some(part) = package.part(name) {
            let mut ctx = Ctx::new(&mut document.styles, &mut headers);
            document.comments = notes::read_comments(part.data(), &mut ctx);
        }
    }
    if let Some(name) = &parts.comments_extended {
        if let Some(part) = package.part(name) {
            notes::apply_resolved(part.data(), &mut document.comments);
        }
    }
    if let Some(name) = &parts.people {
        if let Some(part) = package.part(name) {
            document.people = bits::people(part.data());
        }
    }

    // Headers last, because which ones exist is decided by what the sections
    // referred to while the body was read.
    let referenced: Vec<(wp_model::section::HeaderId, String, bool)> = headers
        .referenced()
        .map(|(id, rel, footer)| (id, rel.to_owned(), footer))
        .collect();
    for (id, rel, footer) in referenced {
        let Some(name) = parts.target(&rel) else {
            continue;
        };
        let Some(part) = package.part(name) else {
            continue;
        };
        let content = {
            let mut ctx = Ctx::new(&mut document.styles, &mut headers);
            read_header(part.data(), &mut ctx, footer)
        };
        document.headers.push(HeaderFooter {
            id,
            part: Some(name.as_str().into()),
            rel: Some(rel.into()),
            footer,
            content,
        });
    }

    Ok(document)
}

/// Finds `<w:body>` and reads it.
fn read_body(
    xml: &[u8],
    ctx: &mut Ctx<'_>,
) -> (Vec<Block>, Option<wp_model::section::SectionProps>) {
    let mut reader = Reader::from_reader(xml);
    // Whitespace between elements is not significant in WordprocessingML — the
    // text is inside `<w:t>`, which is read with its own reader state — but
    // trimming it would also trim the *contents* of `<w:t xml:space="preserve">`,
    // which is where the space between two differently formatted words lives.
    reader.config_mut().trim_text(false);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if xml::local_name(&e) == b"body" => {
                return body::read_blocks(&mut reader, ctx, b"body")
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    (Vec::new(), None)
}

fn read_header(xml: &[u8], ctx: &mut Ctx<'_>, footer: bool) -> Vec<Block> {
    let element: &[u8] = if footer { b"ftr" } else { b"hdr" };
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if xml::local_name(&e) == element => {
                return body::read_blocks(&mut reader, ctx, element).0
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    Vec::new()
}
