//! The document's skeleton: headings, bookmarks, and the table of contents
//! built from them.
//!
//! **A heading is not a style name.** `<w:outlineLvl>` is what makes a paragraph
//! a heading, and it is usually inherited from a style called `Heading1` — but a
//! document may set it directly, and a document may have a style called
//! `ChapterTitle` that sets it. Matching on the name finds Word's own headings
//! and misses everybody else's; matching on the outline level finds both.
//!
//! The name is still worth reading, because a style called `Heading 2` that
//! forgot to set an outline level is a real document too. Both, in that order.

use crate::doc::{Block, Document, Paragraph};
use crate::style::{StyleKind, StyleTable};

/// One heading, as an outline or a table of contents sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// Index into [`Document::paragraphs`].
    pub paragraph: usize,
    /// One-based, as `TOC \o "1-3"` counts them. `<w:outlineLvl>` is zero-based.
    pub level: u8,
    pub text: String,
}

/// A bookmark, and where it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub name: std::sync::Arc<str>,
    pub paragraph: usize,
    /// Byte offset into that paragraph's text.
    pub offset: usize,
}

impl Bookmark {
    /// Whether this is one of Word's own, which the user never asked for and
    /// should not be offered.
    ///
    /// `_GoBack` is where the cursor was; `_Toc…` are the anchors a table of
    /// contents points at; `_Ref…` are what a cross-reference points at.
    pub fn is_internal(&self) -> bool {
        self.name.starts_with('_')
    }
}

/// The outline level of a paragraph, one-based, or `None` for body text.
pub fn heading_level(paragraph: &Paragraph, styles: &StyleTable) -> Option<u8> {
    let resolved = styles.resolve_paragraph(&paragraph.props, None);
    if let Some(level) = resolved.para.outline_level {
        // 9 is `<w:outlineLvl w:val="9">`, which is Word's spelling of "body
        // text" — a paragraph that names a level and means none.
        return (level < 9).then_some(level + 1);
    }
    // A style called "Heading 2" that never set an outline level is still a
    // heading, and documents from other producers are full of them.
    let style = paragraph
        .props
        .style
        .or_else(|| styles.default_style(StyleKind::Paragraph))?;
    for step in styles.chain(style).into_iter().rev() {
        let style = styles.get(step)?;
        let name = style.name.as_deref().unwrap_or(&style.id);
        if let Some(level) = heading_name_level(name) {
            return Some(level);
        }
    }
    None
}

/// `heading 2`, `Heading2`, `Heading 2` -> 2.
fn heading_name_level(name: &str) -> Option<u8> {
    let lower = name.to_ascii_lowercase();
    let rest = lower.strip_prefix("heading")?.trim_start();
    let level: u8 = rest.parse().ok()?;
    (1..=9).contains(&level).then_some(level)
}

/// Every heading in the document, in order.
pub fn headings(document: &Document) -> Vec<Heading> {
    document
        .paragraphs()
        .iter()
        .enumerate()
        .filter_map(|(index, paragraph)| {
            let level = heading_level(paragraph, &document.styles)?;
            let text = paragraph.text();
            // A heading with no text is a spacer, and a table of contents full
            // of blank rows is worse than one that is a line short.
            (!text.trim().is_empty()).then(|| Heading {
                paragraph: index,
                level,
                text: text.trim().to_owned(),
            })
        })
        .collect()
}

/// Every bookmark, in document order.
pub fn bookmarks(document: &Document) -> Vec<Bookmark> {
    let mut out = Vec::new();
    for (index, paragraph) in document.paragraphs().iter().enumerate() {
        // A bookmark's offset is where it sits in the text, which means counting
        // the text before it — the anchors are between the runs rather than in
        // them.
        let mut offset = 0usize;
        for inline in &paragraph.content {
            match inline {
                crate::doc::Inline::Anchor(crate::revision::Anchor::BookmarkStart {
                    name, ..
                }) => out.push(Bookmark {
                    name: name.clone(),
                    paragraph: index,
                    offset,
                }),
                other => {
                    let mut text = String::new();
                    write_inline_text(other, &mut text);
                    offset += text.len();
                }
            }
        }
    }
    out
}

fn write_inline_text(inline: &crate::doc::Inline, out: &mut String) {
    use crate::doc::Inline;
    match inline {
        Inline::Run(run) => out.push_str(&run.text()),
        Inline::Hyperlink(link) => {
            for inner in &link.content {
                write_inline_text(inner, out);
            }
        }
        Inline::Revised { revision, content } => {
            if revision.is_present() {
                for inner in content {
                    write_inline_text(inner, out);
                }
            }
        }
        Inline::Structured(sdt) => {
            for inner in &sdt.content {
                write_inline_text(inner, out);
            }
        }
        Inline::Wrapper { content, .. } | Inline::SimpleField { content, .. } => {
            for inner in content {
                write_inline_text(inner, out);
            }
        }
        Inline::Math(math) => out.push_str(&math.text),
        Inline::Anchor(_) => {}
    }
}

/// One row of a table of contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocEntry {
    pub level: u8,
    pub text: String,
    /// Index into [`Document::paragraphs`], so the row can be made to lead
    /// somewhere.
    pub paragraph: usize,
    /// The bookmark Word anchors the entry to, where the document has one.
    pub anchor: Option<std::sync::Arc<str>>,
}

/// The rows a `TOC` field covering `levels` would have.
///
/// The page numbers are not here: they are a property of the *laid-out*
/// document, and a table of contents that guessed them would be wrong in a way
/// nobody could see. `wp-layout` fills them in.
pub fn table_of_contents(
    document: &Document,
    levels: std::ops::RangeInclusive<u8>,
) -> Vec<TocEntry> {
    let anchors = bookmarks(document);
    headings(document)
        .into_iter()
        .filter(|heading| levels.contains(&heading.level))
        .map(|heading| {
            let anchor = anchors
                .iter()
                .find(|bookmark| {
                    bookmark.paragraph == heading.paragraph && bookmark.name.starts_with("_Toc")
                })
                .map(|bookmark| bookmark.name.clone());
            TocEntry {
                level: heading.level,
                text: heading.text,
                paragraph: heading.paragraph,
                anchor,
            }
        })
        .collect()
}

/// Where a `TOC` field lives, as paragraph indices.
///
/// A table of contents is not one paragraph. Word writes the field's `begin` in
/// one, every entry as a paragraph of its own, and the `end` in the last — so
/// rebuilding it means replacing everything *between* the two, and leaving the
/// two alone, because they carry the field characters that make it a field at
/// all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocSpan {
    /// The paragraph holding the field's `begin`.
    pub first: usize,
    /// The paragraph holding its `end`. Equal to `first` for a `<w:fldSimple>`
    /// or a field that fits in one paragraph, and the entries then go after it.
    pub last: usize,
    pub levels: std::ops::RangeInclusive<u8>,
}

impl TocSpan {
    /// The paragraphs a rebuild replaces.
    pub fn entries(&self) -> std::ops::Range<usize> {
        if self.last > self.first {
            self.first + 1..self.last
        } else {
            self.first + 1..self.first + 1
        }
    }
}

/// Finds the document's table of contents, if it has one.
pub fn toc_span(document: &Document) -> Option<TocSpan> {
    use crate::doc::{Inline, Piece};

    let paragraphs = document.paragraphs();
    let mut depth = 0usize;
    let mut opened_at: Option<usize> = None;
    let mut levels: Option<std::ops::RangeInclusive<u8>> = None;
    let mut instruction = String::new();

    for (index, paragraph) in paragraphs.iter().enumerate() {
        // A `<w:fldSimple>` carries the whole field in one element.
        for inline in &paragraph.content {
            if let Inline::SimpleField { instruction, .. } = inline {
                if let Some(field) = crate::Field::parse(instruction) {
                    if field.kind() == crate::field::Kind::Toc {
                        return Some(TocSpan {
                            first: index,
                            last: index,
                            levels: crate::field::toc_levels(&field).unwrap_or(1..=3),
                        });
                    }
                }
            }
        }
        for piece in paragraph.runs().iter().flat_map(|run| &run.content) {
            match piece {
                Piece::FieldStart { .. } => {
                    depth += 1;
                    if depth == 1 {
                        opened_at = Some(index);
                        instruction.clear();
                    }
                }
                Piece::Instruction(text) if depth == 1 => instruction.push_str(text),
                Piece::FieldSeparate if depth == 1 => {
                    levels = crate::Field::parse(&instruction)
                        .filter(|field| field.kind() == crate::field::Kind::Toc)
                        .map(|field| crate::field::toc_levels(&field).unwrap_or(1..=3));
                }
                Piece::FieldEnd => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        if let (Some(first), Some(levels)) = (opened_at, levels.clone()) {
                            return Some(TocSpan {
                                first,
                                last: index,
                                levels,
                            });
                        }
                        opened_at = None;
                        levels = None;
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// Where a paragraph index sits among the body's blocks, for a command that has
/// to replace it.
pub fn body_index_of(document: &Document, paragraph: usize) -> Option<usize> {
    let mut flat = 0usize;
    for (index, block) in document.body.iter().enumerate() {
        let count = count(block);
        if paragraph < flat + count {
            return matches!(block, Block::Paragraph(_)).then_some(index);
        }
        flat += count;
    }
    None
}

fn count(block: &Block) -> usize {
    match block {
        Block::Paragraph(_) => 1,
        Block::Table(table) => table
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .flat_map(|cell| &cell.content)
            .map(count)
            .sum(),
        Block::Structured(sdt) => sdt.content.iter().map(count).sum(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{Inline, Run};
    use crate::prop::ParaProps;
    use crate::revision::Anchor;
    use crate::style::Style;

    fn document() -> Document {
        let mut document = Document::new();
        let mut normal = Style::new("Normal", StyleKind::Paragraph);
        normal.default = true;
        document.styles.insert(normal);

        let mut h1 = Style::new("Heading1", StyleKind::Paragraph);
        h1.name = Some("heading 1".into());
        h1.para.outline_level = Some(0);
        let h1 = document.styles.insert(h1);

        // A style that is a heading by *name* and forgot to set a level, which
        // is what documents from other producers look like.
        let mut h2 = Style::new("Heading2", StyleKind::Paragraph);
        h2.name = Some("heading 2".into());
        let h2 = document.styles.insert(h2);

        let styled = |text: &str, style| {
            Block::Paragraph(Paragraph {
                props: ParaProps {
                    style: Some(style),
                    ..ParaProps::default()
                },
                ..Paragraph::of(text)
            })
        };
        document.body = vec![
            styled("Chapter One", h1),
            Block::Paragraph(Paragraph::of("Body text.")),
            styled("A Section", h2),
            Block::Paragraph(Paragraph::of("More body.")),
        ];
        document
    }

    #[test]
    fn a_heading_is_found_by_its_outline_level() {
        let document = document();
        let headings = headings(&document);
        assert_eq!(headings.len(), 2);
        assert_eq!(headings[0].level, 1);
        assert_eq!(headings[0].text, "Chapter One");
        assert_eq!(headings[0].paragraph, 0);
    }

    #[test]
    fn a_style_called_heading_two_is_a_heading_even_without_a_level() {
        // Matching on the outline level alone finds Word's own headings and
        // misses everybody else's.
        let document = document();
        let headings = headings(&document);
        assert_eq!(headings[1].level, 2);
        assert_eq!(headings[1].text, "A Section");
    }

    #[test]
    fn outline_level_nine_means_body_text_rather_than_a_ninth_level_heading() {
        let mut document = document();
        let mut paragraph = Paragraph::of("Not a heading");
        paragraph.props.outline_level = Some(9);
        document.body.push(Block::Paragraph(paragraph));
        assert_eq!(headings(&document).len(), 2, "still two");
    }

    #[test]
    fn a_heading_with_no_text_is_not_a_row_of_the_contents() {
        let mut document = document();
        let heading = document.styles.lookup("Heading1").unwrap();
        document.body.push(Block::Paragraph(Paragraph {
            props: ParaProps {
                style: Some(heading),
                ..ParaProps::default()
            },
            ..Paragraph::of("   ")
        }));
        assert_eq!(headings(&document).len(), 2);
    }

    #[test]
    fn a_table_of_contents_takes_the_levels_it_was_asked_for() {
        let document = document();
        let all = table_of_contents(&document, 1..=3);
        assert_eq!(all.len(), 2);
        let top = table_of_contents(&document, 1..=1);
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].text, "Chapter One");
    }

    #[test]
    fn a_bookmark_knows_how_far_into_the_paragraph_it_is() {
        let mut document = Document::new();
        document.body = vec![Block::Paragraph(Paragraph {
            content: vec![
                Inline::Run(Run::of("before ")),
                Inline::Anchor(Anchor::BookmarkStart {
                    id: 1,
                    name: "target".into(),
                }),
                Inline::Run(Run::of("after")),
            ],
            ..Paragraph::new()
        })];
        let found = bookmarks(&document);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name.as_ref(), "target");
        assert_eq!(found[0].offset, 7);
        assert!(!found[0].is_internal());
    }

    #[test]
    fn words_own_bookmarks_are_marked_as_its_own() {
        let bookmark = |name: &str| Bookmark {
            name: name.into(),
            paragraph: 0,
            offset: 0,
        };
        assert!(bookmark("_GoBack").is_internal());
        assert!(bookmark("_Toc123").is_internal());
        assert!(!bookmark("Introduction").is_internal());
    }

    #[test]
    fn a_table_of_contents_field_is_found_and_its_entries_named() {
        use crate::doc::{Inline, Piece, Run};
        let mut document = document();
        let field_start = Block::Paragraph(Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![
                    Piece::FieldStart {
                        dirty: false,
                        lock: false,
                    },
                    Piece::Instruction(" TOC \\o \"1-2\" ".into()),
                    Piece::FieldSeparate,
                    Piece::Text("Chapter One\t1".into()),
                ],
                ..Run::new()
            })],
            ..Paragraph::new()
        });
        let field_end = Block::Paragraph(Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![Piece::Text("A Section\t2".into()), Piece::FieldEnd],
                ..Run::new()
            })],
            ..Paragraph::new()
        });
        document.body.insert(0, field_start);
        document
            .body
            .insert(1, Block::Paragraph(Paragraph::of("middle row")));
        document.body.insert(2, field_end);

        let span = toc_span(&document).expect("a table of contents");
        assert_eq!(span.first, 0);
        assert_eq!(span.last, 2);
        assert_eq!(span.levels, 1..=2);
        // The two paragraphs carrying the field characters are left alone; only
        // what is between them is replaced.
        assert_eq!(span.entries(), 1..2);
    }

    #[test]
    fn a_document_with_no_table_of_contents_says_so() {
        assert_eq!(toc_span(&document()), None);
    }

    #[test]
    fn a_compact_field_puts_its_entries_after_itself() {
        let span = TocSpan {
            first: 4,
            last: 4,
            levels: 1..=3,
        };
        assert_eq!(span.entries(), 5..5);
    }

    #[test]
    fn a_paragraph_in_the_body_can_be_found_among_the_blocks() {
        let document = document();
        assert_eq!(body_index_of(&document, 0), Some(0));
        assert_eq!(body_index_of(&document, 3), Some(3));
        assert_eq!(body_index_of(&document, 9), None);
    }
}
