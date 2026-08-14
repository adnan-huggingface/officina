//! `footnotes.xml`, `endnotes.xml`, `comments.xml` and `commentsExtended.xml`.

use std::collections::BTreeMap;

use quick_xml::events::Event;
use quick_xml::Reader;

use wp_model::doc::{Block, Note, NoteKind};
use wp_model::revision::Comment;

use crate::body::read_blocks;
use crate::ctx::Ctx;
use crate::xml::{attr, attr_i32, attr_u32, local_name};

/// Reads `footnotes.xml` or `endnotes.xml`.
///
/// The element name differs (`<w:footnote>` against `<w:endnote>`) and nothing
/// else does, so one function reads both.
pub(crate) fn read_notes(xml: &[u8], ctx: &mut Ctx<'_>, element: &[u8]) -> Vec<Note> {
    let mut notes = Vec::new();
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) if local_name(&e) == element => {
                let id = attr_i32(&e, b"id").unwrap_or(0);
                // The separator and the continuation separator are `<w:footnote>`
                // elements like any other and are not notes: they are the little
                // rules Word draws above the footnote area. Listing them shows
                // two empty footnotes at the top of every document, so the kind
                // is read rather than assumed.
                let kind = attr(&e, b"type")
                    .as_deref()
                    .and_then(NoteKind::from_val)
                    .unwrap_or(NoteKind::Normal);
                let (content, _) = read_blocks(&mut reader, ctx, element);
                notes.push(Note { id, kind, content });
            }
            Event::Eof => break,
            _ => {}
        }
    }
    notes
}

/// Reads `comments.xml`.
pub(crate) fn read_comments(xml: &[u8], ctx: &mut Ctx<'_>) -> Vec<Comment> {
    let mut comments = Vec::new();
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) if local_name(&e) == b"comment" => {
                let mut comment = Comment::new(
                    attr_u32(&e, b"id").unwrap_or(0),
                    attr(&e, b"author").unwrap_or_default(),
                );
                comment.initials = attr(&e, b"initials").map(Into::into);
                comment.date = attr(&e, b"date").map(Into::into);
                let (content, _) = read_blocks(&mut reader, ctx, b"comment");
                comment.content = content;
                comments.push(comment);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    comments
}

/// Applies `commentsExtended.xml`, which is where a comment's *resolved* flag
/// lives.
///
/// It is not keyed by comment id. It is keyed by the `w14:paraId` of the
/// comment's **last paragraph**, which means the two parts can only be joined by
/// walking into the comment bodies. Nothing in either file says so, and a reader
/// that matches on the id joins the wrong rows — usually off by one, so most
/// comments look right and one is wrongly shown as resolved.
pub(crate) fn apply_resolved(xml: &[u8], comments: &mut [Comment]) {
    let mut done: BTreeMap<u32, bool> = BTreeMap::new();
    let mut reader = Reader::from_reader(xml);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if local_name(&e) == b"commentEx" => {
                let Some(para_id) =
                    attr(&e, b"paraId").and_then(|hex| u32::from_str_radix(hex.trim(), 16).ok())
                else {
                    continue;
                };
                let resolved = attr(&e, b"done")
                    .as_deref()
                    .map(|v| wp_model::prop::on_off(Some(v)))
                    .unwrap_or(false);
                done.insert(para_id, resolved);
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    if done.is_empty() {
        return;
    }
    for comment in comments {
        if let Some(resolved) = last_para_id(&comment.content).and_then(|id| done.get(&id)) {
            comment.done = *resolved;
        }
    }
}

fn last_para_id(blocks: &[Block]) -> Option<u32> {
    blocks.iter().rev().find_map(|block| match block {
        Block::Paragraph(paragraph) => paragraph.id,
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::test_ctx;

    fn ctx_of<T>(f: impl FnOnce(&mut Ctx<'_>) -> T) -> T {
        let (mut styles, mut headers) = test_ctx();
        let mut ctx = Ctx::new(&mut styles, &mut headers);
        f(&mut ctx)
    }

    #[test]
    fn the_separators_are_read_as_separators_rather_than_as_notes() {
        let notes = ctx_of(|ctx| {
            read_notes(
                br#"<w:footnotes>
                  <w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote>
                  <w:footnote w:type="continuationSeparator" w:id="0"><w:p/></w:footnote>
                  <w:footnote w:id="1"><w:p><w:r><w:t>A real note.</w:t></w:r></w:p></w:footnote>
                </w:footnotes>"#,
                ctx,
                b"footnote",
            )
        });
        assert_eq!(notes.len(), 3);
        assert_eq!(notes[0].kind, NoteKind::Separator);
        assert!(!notes[0].kind.is_real());
        assert_eq!(notes[2].id, 1);
        assert!(notes[2].kind.is_real());
        assert_eq!(notes[2].text(), "A real note.");
    }

    #[test]
    fn a_comment_reads_its_author_and_its_body() {
        let comments = ctx_of(|ctx| {
            read_comments(
                br#"<w:comments><w:comment w:id="1" w:author="Adnan Khan" w:initials="AK" w:date="2026-08-14T00:00:00Z"><w:p w14:paraId="0B2A5F3C"><w:r><w:t>Needs a citation.</w:t></w:r></w:p></w:comment></w:comments>"#,
                ctx,
            )
        });
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].author.as_ref(), "Adnan Khan");
        assert_eq!(comments[0].initials.as_deref(), Some("AK"));
        assert_eq!(comments[0].text(), "Needs a citation.");
        assert!(!comments[0].done);
    }

    #[test]
    fn resolved_is_joined_by_the_last_paragraphs_id_and_not_by_the_comment_id() {
        let mut comments = ctx_of(|ctx| {
            read_comments(
                br#"<w:comments>
                  <w:comment w:id="1" w:author="A"><w:p w14:paraId="0B2A5F3C"><w:r><w:t>one</w:t></w:r></w:p></w:comment>
                  <w:comment w:id="2" w:author="B"><w:p w14:paraId="11111111"><w:r><w:t>two</w:t></w:r></w:p></w:comment>
                </w:comments>"#,
                ctx,
            )
        });
        apply_resolved(
            br#"<w15:commentsEx><w15:commentEx w15:paraId="0B2A5F3C" w15:done="1"/><w15:commentEx w15:paraId="11111111" w15:done="0"/></w15:commentsEx>"#,
            &mut comments,
        );
        assert!(comments[0].done, "the one whose paraId matched");
        assert!(!comments[1].done);
    }
}
