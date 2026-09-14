//! `comments.xml` and `commentsExtended.xml`: rewritten when the comments
//! changed, authored when the package has none.
//!
//! A comment is three anchors in the document part and a body in a part of
//! its own, and Word wants all four: a `<w:commentReference>` naming a
//! comment the package does not hold is not a missing comment to Word, it is
//! a damaged file. The document part wrote the anchors from the day a comment
//! could be posted; nothing wrote the body, and a save with a new comment in
//! it was a save Word refused. Found when the page began washing a comment's
//! words and the pane's cards were read against what the file held.
//!
//! Changed means "would read back differently", the same bargain the header
//! parts strike: the existing part is re-read and compared with the model, so
//! a save that touched no comment keeps the part's exact bytes. The extended
//! part carries what Word keeps beside the bodies — resolved, and which
//! comment a reply answers — keyed by the `w14:paraId` of each comment's last
//! paragraph, which is why a paragraph of a comment born here is given an id
//! before either part is written.

use ooxml::{Package, PartName, Relationship, TargetMode};
use wp_model::doc::Block;
use wp_model::revision::Comment;
use wp_model::style::StyleTable;
use wp_model::Document;

use crate::ctx::{Ctx, HeaderIndex};
use crate::error::{Error, Result};
use crate::parts::DocumentParts;

use super::emit;

const COMMENTS_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml";
const EXTENDED_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml";
const REL_BASE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const EXTENDED_REL: &str =
    "http://schemas.microsoft.com/office/2011/relationships/commentsExtended";
const RELS_TYPE: &str = "application/vnd.openxmlformats-package.relationships+xml";
const WML: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const W15: &str = "http://schemas.microsoft.com/office/word/2012/wordml";
const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n";

pub(crate) fn flush(
    document: &mut Document,
    package: &mut Package,
    located: &DocumentParts,
) -> Result<()> {
    if document.comments.is_empty() && located.comments.is_none() {
        return Ok(());
    }
    let mode = document.settings.compatibility_mode;
    let changed = match &located.comments {
        Some(name) => match package.part(name) {
            Some(part) => {
                let mut existing = {
                    let mut index = HeaderIndex::default();
                    let scope = crate::parts::scope_of(name);
                    let mut ctx =
                        Ctx::of_named_part(&mut document.styles, &mut index, part.data(), &scope);
                    ctx.compat_mode = mode;
                    crate::notes::read_comments(part.data(), &mut ctx)
                };
                if let Some(extended) = located
                    .comments_extended
                    .as_ref()
                    .and_then(|name| package.part(name))
                {
                    crate::notes::apply_resolved(extended.data(), &mut existing);
                }
                existing != document.comments
            }
            None => true,
        },
        None => true,
    };
    if !changed {
        return Ok(());
    }

    mint_paragraph_ids(&mut document.comments);
    let body = comments_out(&document.comments, &document.styles, mode);
    let extended = extended_out(&document.comments);
    let mut rels = package.relationships(&located.document)?;
    let mut related = false;

    match &located.comments {
        Some(name) => {
            let content_type = package
                .part(name)
                .map(|part| part.content_type.clone())
                .unwrap_or_else(|| COMMENTS_TYPE.to_owned());
            package.put_part(name.clone(), &content_type, body);
        }
        None => {
            let name = beside(&located.document, "comments.xml")?;
            package.put_part(name, COMMENTS_TYPE, body);
            rels.insert(Relationship {
                id: rels.next_id(),
                rel_type: format!("{REL_BASE}/comments"),
                target: "comments.xml".to_owned(),
                mode: TargetMode::Internal,
            });
            related = true;
        }
    }
    match (&located.comments_extended, extended) {
        (Some(name), Some(data)) => {
            let content_type = package
                .part(name)
                .map(|part| part.content_type.clone())
                .unwrap_or_else(|| EXTENDED_TYPE.to_owned());
            package.put_part(name.clone(), &content_type, data);
        }
        (None, Some(data)) => {
            let name = beside(&located.document, "commentsExtended.xml")?;
            package.put_part(name, EXTENDED_TYPE, data);
            rels.insert(Relationship {
                id: rels.next_id(),
                rel_type: EXTENDED_REL.to_owned(),
                target: "commentsExtended.xml".to_owned(),
                mode: TargetMode::Internal,
            });
            related = true;
        }
        // A part that says nothing is resolved and nothing is a reply is a
        // part that says nothing; one that exists keeps saying so.
        (Some(name), None) => {
            let content_type = package
                .part(name)
                .map(|part| part.content_type.clone())
                .unwrap_or_else(|| EXTENDED_TYPE.to_owned());
            package.put_part(name.clone(), &content_type, extended_empty());
        }
        (None, None) => {}
    }
    if related {
        package.put_part(located.document.rels_part(), RELS_TYPE, rels.to_xml());
    }
    Ok(())
}

/// A part name next to the document part, as Word places it.
fn beside(document: &PartName, file: &str) -> Result<PartName> {
    let raw = document.as_str();
    let folder = &raw[..raw.rfind('/').map_or(0, |at| at + 1)];
    PartName::new(&format!("{folder}{file}")).map_err(Error::Package)
}

/// Gives every comment paragraph without a `w14:paraId` one, unique across
/// the comments. The extended part names a comment by its last paragraph's
/// id, so a comment born in the application has to have one before either
/// part is written. Below `0x80000000`, as Word's are.
fn mint_paragraph_ids(comments: &mut [Comment]) {
    let mut taken: Vec<u32> = comments
        .iter()
        .flat_map(|comment| comment.content.iter())
        .filter_map(|block| match block {
            Block::Paragraph(paragraph) => paragraph.id,
            _ => None,
        })
        .collect();
    // A stride with no factor in common with the range walks every id once
    // before repeating, and starts well away from the small numbers a hand-
    // written file uses.
    let mut next: u32 = 0x1A2B_3C4D;
    for comment in comments {
        for block in &mut comment.content {
            let Block::Paragraph(paragraph) = block else {
                continue;
            };
            if paragraph.id.is_some() {
                continue;
            }
            while taken.contains(&next) {
                next = next.wrapping_add(0x0F1E_2D3B) & 0x7FFF_FFFF;
            }
            paragraph.id = Some(next);
            taken.push(next);
            next = next.wrapping_add(0x0F1E_2D3B) & 0x7FFF_FFFF;
        }
    }
}

fn comments_out(comments: &[Comment], styles: &StyleTable, mode: u32) -> Vec<u8> {
    let mut out = String::from(DECL);
    out.push_str(&format!(
        r#"<w:comments xmlns:w="{WML}" xmlns:w14="{W14}" xmlns:r="{REL_BASE}">"#
    ));
    for comment in comments {
        out.push_str(&format!(r#"<w:comment w:id="{}""#, comment.id));
        out.push_str(&format!(
            r#" w:author="{}""#,
            super::splice::escape_attr(&comment.author)
        ));
        if let Some(date) = &comment.date {
            out.push_str(&format!(
                r#" w:date="{}""#,
                super::splice::escape_attr(date)
            ));
        }
        if let Some(initials) = &comment.initials {
            out.push_str(&format!(
                r#" w:initials="{}""#,
                super::splice::escape_attr(initials)
            ));
        }
        out.push('>');
        for block in &comment.content {
            match block {
                Block::Paragraph(paragraph) => emit::paragraph(&mut out, paragraph, styles),
                Block::Table(table) => emit::table(&mut out, table, styles, mode),
                _ => {}
            }
        }
        if comment.content.is_empty() {
            out.push_str("<w:p/>");
        }
        out.push_str("</w:comment>");
    }
    out.push_str("</w:comments>");
    out.into_bytes()
}

/// The extended part, or nothing when no comment is resolved or a reply.
fn extended_out(comments: &[Comment]) -> Option<Vec<u8>> {
    if comments.iter().all(|c| !c.done && c.parent.is_none()) {
        return None;
    }
    let mut out = String::from(DECL);
    out.push_str(&format!(r#"<w15:commentsEx xmlns:w15="{W15}">"#));
    for comment in comments {
        let Some(para) = crate::notes::last_para_id(&comment.content) else {
            continue;
        };
        out.push_str(&format!(r#"<w15:commentEx w15:paraId="{para:08X}""#));
        if let Some(parent) = comment
            .parent
            .and_then(|id| comments.iter().find(|c| c.id == id))
            .and_then(|parent| crate::notes::last_para_id(&parent.content))
        {
            out.push_str(&format!(r#" w15:paraIdParent="{parent:08X}""#));
        }
        out.push_str(&format!(r#" w15:done="{}"/>"#, u8::from(comment.done)));
    }
    out.push_str("</w15:commentsEx>");
    Some(out.into_bytes())
}

fn extended_empty() -> Vec<u8> {
    format!(r#"{DECL}<w15:commentsEx xmlns:w15="{W15}"/>"#).into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::doc::{Inline, Paragraph, Piece, Run};

    fn commented() -> Document {
        let mut paragraph = Paragraph::of("A sentence.");
        paragraph
            .content
            .insert(0, Inline::Anchor(wp_model::Anchor::CommentStart { id: 1 }));
        paragraph
            .content
            .push(Inline::Anchor(wp_model::Anchor::CommentEnd { id: 1 }));
        paragraph.content.push(Inline::Run(Run {
            content: vec![Piece::CommentRef(1)],
            ..Run::new()
        }));
        let mut comment = Comment::new(1, "Reviewer");
        comment.content = vec![Block::Paragraph(Paragraph::of("Is it?"))];
        Document {
            body: vec![Block::Paragraph(paragraph)],
            comments: vec![comment],
            ..Document::default()
        }
    }

    /// The failure this exists for: a package authored for a document with
    /// a comment in it held the anchors and not the comment, and Word called
    /// the file damaged.
    #[test]
    fn a_comment_the_text_names_is_in_the_package_it_is_saved_into() {
        let mut document = commented();
        let mut package = crate::write::blank::package_for(&document).expect("authored");
        crate::write::flush(&mut document, &mut package).expect("written");

        let located = crate::parts::locate(&package).expect("a document part");
        let name = located
            .comments
            .clone()
            .expect("the comments part is related");
        let xml =
            String::from_utf8_lossy(package.part(&name).expect("and there").data()).into_owned();
        assert!(
            xml.contains(r#"<w:comment w:id="1" w:author="Reviewer">"#),
            "{xml}"
        );
        assert!(xml.contains("Is it?"), "{xml}");
        assert!(
            xml.contains("w14:paraId="),
            "the paragraph was given an id: {xml}"
        );
        assert_eq!(
            package.content_types().get(&name),
            Some(COMMENTS_TYPE),
            "and a content type that says what it is"
        );
        assert!(
            located.comments_extended.is_none(),
            "nothing resolved and no reply: no extended part"
        );

        // Read back, it is the same comment.
        let again = crate::read(&package).expect("the package reads");
        assert_eq!(again.comments.len(), 1);
        assert_eq!(again.comments[0].text(), "Is it?");
        assert_eq!(again.comments[0].author.as_ref(), "Reviewer");
    }

    #[test]
    fn a_resolved_comment_and_a_reply_are_in_the_extended_part() {
        let mut document = commented();
        document.comments[0].done = true;
        let mut reply = Comment::new(2, "Author");
        reply.parent = Some(1);
        reply.content = vec![Block::Paragraph(Paragraph::of("It is."))];
        document.comments.push(reply);
        let mut package = crate::write::blank::package_for(&document).expect("authored");
        crate::write::flush(&mut document, &mut package).expect("written");

        let located = crate::parts::locate(&package).expect("a document part");
        let name = located
            .comments_extended
            .clone()
            .expect("the extended part is related");
        let xml =
            String::from_utf8_lossy(package.part(&name).expect("and there").data()).into_owned();
        assert!(xml.contains(r#"w15:done="1""#), "{xml}");
        assert!(xml.contains("w15:paraIdParent="), "{xml}");
        assert_eq!(package.content_types().get(&name), Some(EXTENDED_TYPE));

        let again = crate::read(&package).expect("the package reads");
        assert!(again.comments[0].done, "resolved survives the round trip");
        assert!(!again.comments[1].done);
        assert_eq!(
            again.comments[1].parent,
            Some(1),
            "and so does the reply's parent"
        );
    }

    #[test]
    fn a_save_that_touched_no_comment_keeps_the_part_it_had() {
        let mut document = commented();
        let mut package = crate::write::blank::package_for(&document).expect("authored");
        crate::write::flush(&mut document, &mut package).expect("written");
        let located = crate::parts::locate(&package).expect("a document part");
        let name = located.comments.clone().expect("related");
        let before = package.part(&name).expect("there").data().to_vec();
        // Re-read, as a second session would, then saved untouched.
        let mut again = crate::read(&package).expect("reads");
        crate::write::flush(&mut again, &mut package).expect("written again");
        assert_eq!(
            package.part(&name).expect("still there").data(),
            &before[..]
        );
    }
}
