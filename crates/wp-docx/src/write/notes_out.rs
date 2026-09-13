//! Footnote and endnote parts, authored for a package that has none.
//!
//! A package Word wrote keeps its notes part, and it is retained as it came.
//! A document put into a package it did not come out of — read out of an
//! `.odt` or a `.doc`, and saved as a `.docx` — carries its notes in the model
//! and nowhere else, while its text names them: `<w:footnoteReference w:id="1"/>`.
//! **A reference to a note the package does not hold is not a missing footnote
//! to Word, it is a corrupted file**, and Word refuses the whole of it. So when
//! the model has notes and the package has no part to hold them, the part is
//! authored here, with the relationship that finds it and the content type
//! that says what it is.
//!
//! What goes in is what Word itself writes for a new document's first note:
//! the two separators first — the rule above the note area and its
//! continuation, ids -1 and 0 — unless the model already has its own, and each
//! note beginning with its own number. A note read out of ODF has no number of
//! its own, because ODF draws it from the citation; without one Word shows the
//! note's text with nothing to say which reference it belongs to.

use ooxml::{Package, PartName, Relationship, TargetMode};
use wp_model::doc::{Block, Inline, Note, NoteKind, Paragraph, Piece, Run};
use wp_model::prop::{RunProps, VertAlign};
use wp_model::style::StyleTable;
use wp_model::Document;

use crate::error::{Error, Result};
use crate::parts::DocumentParts;

use super::emit;

const REL_BASE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const RELS_TYPE: &str = "application/vnd.openxmlformats-package.relationships+xml";
const WML: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n";

/// The two kinds of note, and everything that differs between them.
#[derive(Clone, Copy)]
struct Kind {
    endnote: bool,
    /// `footnotes`, the root element and the relationship type's last word.
    plural: &'static str,
    /// `footnote`, each note's element.
    singular: &'static str,
    content_type: &'static str,
}

const FOOTNOTES: Kind = Kind {
    endnote: false,
    plural: "footnotes",
    singular: "footnote",
    content_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml",
};

const ENDNOTES: Kind = Kind {
    endnote: true,
    plural: "endnotes",
    singular: "endnote",
    content_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml",
};

pub(crate) fn flush(
    document: &Document,
    package: &mut Package,
    located: &DocumentParts,
) -> Result<()> {
    for (kind, notes, existing) in [
        (FOOTNOTES, &document.footnotes, &located.footnotes),
        (ENDNOTES, &document.endnotes, &located.endnotes),
    ] {
        let wanted = notes.iter().any(|note| note.kind == NoteKind::Normal);
        if !wanted || existing.is_some() {
            continue;
        }
        let name = free_name(&located.document, kind)?;
        package.put_part(
            name.clone(),
            kind.content_type,
            part_out(kind, notes, &document.styles),
        );

        let mut rels = package.relationships(&located.document)?;
        let id = rels.next_id();
        rels.insert(Relationship {
            id,
            rel_type: format!("{REL_BASE}/{}", kind.plural),
            target: format!("{}.xml", kind.plural),
            mode: TargetMode::Internal,
        });
        package.put_part(located.document.rels_part(), RELS_TYPE, rels.to_xml());
    }
    Ok(())
}

fn part_out(kind: Kind, notes: &[Note], styles: &StyleTable) -> Vec<u8> {
    let mut out = String::from(DECL);
    out.push_str(&format!(
        r#"<w:{} xmlns:w="{WML}" xmlns:r="{REL_BASE}">"#,
        kind.plural
    ));
    // A separator the model holds without a word in it is Word's own rule: a
    // `.doc` reader brings one across as an empty paragraph, because the
    // character its story held stood for the rule rather than being text.
    // Written as the element Word draws the rule from, and under the id Word
    // gives it, so that the rule is there when Word opens the file.
    let wordless = |note: &Note| {
        note.kind != NoteKind::Normal
            && note.content.iter().all(|block| match block {
                Block::Paragraph(paragraph) => paragraph.text().trim().is_empty(),
                _ => false,
            })
    };
    let has = |wanted: NoteKind| notes.iter().any(|note| note.kind == wanted);
    if !has(NoteKind::Separator)
        || notes
            .iter()
            .any(|n| n.kind == NoteKind::Separator && wordless(n))
    {
        separator(&mut out, kind, -1, "separator");
    }
    if !has(NoteKind::ContinuationSeparator)
        || notes
            .iter()
            .any(|n| n.kind == NoteKind::ContinuationSeparator && wordless(n))
    {
        separator(&mut out, kind, 0, "continuationSeparator");
    }
    for note in notes.iter().filter(|note| !wordless(note)) {
        let what = match note.kind {
            NoteKind::Normal => "",
            NoteKind::Separator => r#" w:type="separator""#,
            NoteKind::ContinuationSeparator => r#" w:type="continuationSeparator""#,
            NoteKind::ContinuationNotice => r#" w:type="continuationNotice""#,
        };
        out.push_str(&format!(
            r#"<w:{}{what} w:id="{}">"#,
            kind.singular, note.id
        ));
        let numbered = numbered(kind, note);
        for block in numbered.as_deref().unwrap_or(&note.content) {
            match block {
                Block::Paragraph(paragraph) => emit::paragraph(&mut out, paragraph, styles),
                Block::Table(table) => emit::table(&mut out, table, styles),
                _ => {}
            }
        }
        // A note with nothing in it is still a note, and the schema wants a
        // paragraph in every one.
        if note.content.is_empty() && numbered.is_none() {
            out.push_str("<w:p/>");
        }
        out.push_str(&format!("</w:{}>", kind.singular));
    }
    out.push_str(&format!("</w:{}>", kind.plural));
    out.into_bytes()
}

/// One of the two little rules Word draws above a note area.
fn separator(out: &mut String, kind: Kind, id: i32, what: &str) {
    out.push_str(&format!(
        r#"<w:{} w:type="{what}" w:id="{id}"><w:p><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr><w:r><w:{what}/></w:r></w:p></w:{}>"#,
        kind.singular, kind.singular
    ));
}

/// The note's content with its own number put in front, when it has none.
///
/// `None` when nothing needs adding: a separator, or a note that already
/// carries its mark — a note Word wrote does, and a `.doc` reader mints one.
fn numbered(kind: Kind, note: &Note) -> Option<Vec<Block>> {
    if note.kind != NoteKind::Normal || carries_mark(&note.content) {
        return None;
    }
    let mut content = note.content.clone();
    let mark = Inline::Run(Run {
        props: RunProps {
            vert_align: Some(VertAlign::Superscript),
            ..RunProps::default()
        },
        content: vec![Piece::NoteMark {
            endnote: kind.endnote,
        }],
        prop_change: None,
    });
    match content.first_mut() {
        Some(Block::Paragraph(first)) => first.content.insert(0, mark),
        _ => {
            let mut paragraph = Paragraph::new();
            paragraph.content.push(mark);
            content.insert(0, Block::Paragraph(paragraph));
        }
    }
    Some(content)
}

fn carries_mark(content: &[Block]) -> bool {
    content.iter().any(|block| match block {
        Block::Paragraph(paragraph) => paragraph.content.iter().any(|inline| match inline {
            Inline::Run(run) => run
                .content
                .iter()
                .any(|piece| matches!(piece, Piece::NoteMark { .. })),
            _ => false,
        }),
        _ => false,
    })
}

/// `footnotes.xml` beside the document part.
///
/// Only ever asked for when the document relates no notes part, so a part of
/// that name, if there is one, is one nothing reads; it is replaced rather than
/// kept beside a second that no reader would find either.
fn free_name(beside: &PartName, kind: Kind) -> Result<PartName> {
    let raw = beside.as_str();
    let dir = &raw[..raw.rfind('/').map_or(0, |at| at + 1)];
    PartName::new(&format!("{dir}{}.xml", kind.plural)).map_err(Error::Package)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::doc::Paragraph;

    fn with_a_footnote() -> Document {
        let mut paragraph = Paragraph::of("Text with a note.");
        paragraph.content.push(Inline::Run(Run {
            props: RunProps::default(),
            content: vec![Piece::FootnoteRef {
                id: 1,
                custom_mark: false,
            }],
            prop_change: None,
        }));
        Document {
            body: vec![Block::Paragraph(paragraph)],
            footnotes: vec![Note {
                id: 1,
                kind: NoteKind::Normal,
                content: vec![Block::Paragraph(Paragraph::of("The note."))],
            }],
            ..Document::default()
        }
    }

    /// The failure this exists for: a package authored for a document that
    /// came with notes held the references and not the notes, and Word
    /// called the file corrupted.
    #[test]
    fn a_note_the_text_names_is_in_the_package_it_is_saved_into() {
        let mut document = with_a_footnote();
        let mut package = crate::write::blank::package_for(&document).expect("authored");
        crate::write::flush(&mut document, &mut package).expect("written");

        let located = crate::parts::locate(&package).expect("a document part");
        let name = located
            .footnotes
            .clone()
            .expect("the notes part is related");
        let xml =
            String::from_utf8_lossy(package.part(&name).expect("and there").data()).into_owned();
        assert!(xml.contains(r#"<w:footnote w:id="1">"#), "{xml}");
        assert!(
            xml.contains(r#"w:type="separator" w:id="-1""#)
                && xml.contains(r#"w:type="continuationSeparator" w:id="0""#),
            "the two separators Word expects: {xml}"
        );
        assert!(
            xml.contains("<w:footnoteRef/>"),
            "the note carries its own number: {xml}"
        );
        assert_eq!(
            package.content_types().get(&name),
            Some(FOOTNOTES.content_type),
            "and a content type that says what it is"
        );
    }

    #[test]
    fn a_document_without_notes_gets_no_notes_part() {
        let mut document = Document {
            body: vec![Block::Paragraph(Paragraph::of("No notes."))],
            ..Document::default()
        };
        let mut package = crate::write::blank::package_for(&document).expect("authored");
        crate::write::flush(&mut document, &mut package).expect("written");
        let located = crate::parts::locate(&package).expect("a document part");
        assert!(located.footnotes.is_none() && located.endnotes.is_none());
    }
}
