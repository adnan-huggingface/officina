//! The reader against the corpus — fifteen documents produced by real Word.
//!
//! Unit tests check what we understood. These check what Word actually wrote,
//! which is the only thing that catches a misunderstanding. `LEARNINGS.md` §5.
//!
//! **The corpus is thinner than its file names suggest.** Most of these
//! documents are a single paragraph: `lists-numbering.docx` has one bullet and
//! `styles-headings-toc.docx` has a table-of-contents field and no headings at
//! all. The generator writes several lines into each and Word keeps only the
//! last, because assigning to a fresh paragraph's `Range.Text` writes away the
//! paragraph mark that range includes and merges it into the next one. The
//! assertions below say what the files actually contain rather than what they
//! were meant to; a test that asserts more than the corpus holds is a test that
//! fails for the wrong reason. `office_templates.rs` is where the depth comes
//! from.
//!
//! `rtl-and-cjk.docx` was the same story and is no longer: it held one Arabic
//! word and neither of the scripts it is named for until the layout comparison
//! went looking for CJK across the whole corpus and found not one glyph of it.
//! It now carries all six of its lines.

use std::path::{Path, PathBuf};

use wp_model::doc::{Block, Inline, Piece};
use wp_model::table::VMerge;

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/docx")
        .canonicalize()
        .expect("the corpus is in the repository")
}

fn open(name: &str) -> wp_model::Document {
    let path = corpus().join(name);
    let (document, _) = wp_docx::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    document
}

fn documents() -> Vec<(String, wp_model::Document)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(corpus()).expect("the corpus directory is readable") {
        let path = entry.expect("a readable entry").path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".docx") && !name.ends_with(".dotx") {
            continue;
        }
        let (document, _) = wp_docx::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        out.push((name, document));
    }
    assert!(out.len() >= 15, "the corpus should be fifteen documents");
    out
}

#[test]
fn every_document_in_the_corpus_opens() {
    for (name, document) in documents() {
        assert!(
            !document.body.is_empty(),
            "{name}: opened with an empty body"
        );
        // Word writes a `<w:sectPr>` at the end of every body without exception,
        // so a document whose page is the built-in default has not been read.
        assert!(
            document.section.page.width.0 > 0,
            "{name}: no page size was read"
        );
    }
}

#[test]
fn every_document_has_the_styles_word_writes() {
    for (name, document) in documents() {
        assert!(
            document.styles.len() > 10,
            "{name}: only {} styles",
            document.styles.len()
        );
        let normal = document
            .styles
            .default_style(wp_model::StyleKind::Paragraph)
            .and_then(|id| document.styles.get(id))
            .expect("every Word document has a default paragraph style");
        assert_eq!(normal.id.as_ref(), "Normal", "{name}");
        // Document defaults carry the body font. A document read without them
        // renders in whatever the renderer's fallback is.
        assert!(
            document.styles.doc_defaults().run.size.is_some(),
            "{name}: no default font size"
        );
    }
}

#[test]
fn the_minimal_document_reads_its_text() {
    let document = open("minimal.docx");
    assert_eq!(document.text(), "A single paragraph. The baseline case.");
    assert_eq!(document.body.len(), 1);
}

#[test]
fn a_tracked_deletion_is_kept_and_is_not_part_of_the_text() {
    let document = open("tracked-changes.docx");
    let text = document.text();
    let shown = match &document.body[0] {
        Block::Paragraph(p) => p.shown_text(),
        other => panic!("the first block is a paragraph: {other:?}"),
    };
    assert!(
        shown.len() > text.len(),
        "the deleted sentence is drawn but not counted: {text:?} vs {shown:?}"
    );
    assert!(
        text.contains("inserted"),
        "the insertion is ordinary text: {text:?}"
    );
    assert!(
        shown.contains("deleted"),
        "the deletion survives the read: {shown:?}"
    );
    let authors = document.authors();
    let authors: Vec<&str> = authors.iter().map(|a| a.as_ref()).collect();
    assert_eq!(authors, ["Adnan Khan"]);
}

#[test]
fn nested_tables_are_nested() {
    let document = open("nested-tables.docx");
    let outer = document
        .body
        .iter()
        .find_map(|block| match block {
            Block::Table(table) => Some(table),
            _ => None,
        })
        .expect("there is a table");
    assert_eq!(outer.columns(), 3);
    assert_eq!(outer.rows.len(), 3);

    let inner = outer
        .rows
        .iter()
        .flat_map(|row| &row.cells)
        .flat_map(|cell| &cell.content)
        .find_map(|block| match block {
            Block::Table(table) => Some(table),
            _ => None,
        })
        .expect("a cell holds a table of its own");
    assert_eq!(inner.columns(), 2);
    assert!(inner.text().contains("inner"));

    // Every cell must end with a paragraph, including the one holding the inner
    // table — the format requires it and Word writes it.
    for row in &outer.rows {
        for cell in &row.cells {
            assert!(
                matches!(cell.content.last(), Some(Block::Paragraph(_))),
                "a cell ends with a paragraph"
            );
        }
    }
}

#[test]
fn a_table_that_spans_pages_marks_its_header_row() {
    let document = open("table-spanning-pages.docx");
    let table = document
        .body
        .iter()
        .find_map(|block| match block {
            Block::Table(table) => Some(table),
            _ => None,
        })
        .expect("there is a table");
    assert_eq!(table.rows.len(), 60, "long enough to span a page");
    assert_eq!(table.columns(), 4);
    assert!(
        table.rows[0].props.header,
        "the first row repeats on every page"
    );
    assert!(
        !table.rows[1].props.header,
        "and only the first — Word stops repeating at the first row that does not say so"
    );
    for (index, row) in table.rows.iter().enumerate() {
        for cell in &row.cells {
            if cell.props.v_merge == Some(VMerge::Continue) {
                assert!(index > 0, "a continuation cannot be the first row");
            }
        }
    }
    assert_eq!(table.text().lines().count(), 60);
}

#[test]
fn a_bulleted_paragraph_gets_a_bullet_rather_than_a_number() {
    let document = open("lists-numbering.docx");
    assert!(!document.numbering.is_empty(), "numbering.xml was read");

    let paragraph = document.paragraphs()[0];
    let reference = paragraph.props.numbering.expect("it is in a list");
    assert!(reference.is_numbered());
    let level = document
        .numbering
        .level(reference.num_id, reference.level)
        .expect("the instance resolves to a level");
    assert_eq!(level.format, wp_model::NumFormat::Bullet);

    let label = document.list_labels()[0].clone().expect("it has a label");
    // Word's bullet is a private-use character that means nothing outside the
    // Symbol font the level's own `<w:rPr>` names.
    assert_eq!(label, "\u{F0B7}");
    assert_eq!(
        level.run.fonts.ascii.as_deref(),
        Some("Symbol"),
        "the bullet's font is on the level, not on the paragraph"
    );
    // And the indent comes from the level rather than from ListParagraph.
    assert!(level.para.indent.start.is_some());
}

#[test]
fn sections_carry_their_own_orientation() {
    // The bug this test was written for: `<w:cols w:space="720"/>` is an empty
    // element, and reading children for it consumed `</w:sectPr>`. The section
    // reader then ran to the end of the document, swallowing two paragraphs and
    // a whole section — leaving a three-section document reading as one portrait
    // page with nothing on it.
    let document = open("sections-mixed-orientation.docx");
    let sections = document.sections();
    assert_eq!(sections.len(), 3);
    assert_eq!(document.body.len(), 3);

    let orientations: Vec<_> = sections
        .iter()
        .map(|(_, section)| section.page.orientation)
        .collect();
    assert_eq!(
        orientations,
        [
            wp_model::Orientation::Portrait,
            wp_model::Orientation::Landscape,
            wp_model::Orientation::Portrait
        ]
    );
    // Landscape means the width is already the larger measurement.
    for (_, section) in &sections {
        let landscape = section.page.orientation == wp_model::Orientation::Landscape;
        assert_eq!(
            landscape,
            section.page.width > section.page.height,
            "a landscape page is wider than it is tall"
        );
    }
    // The third section's wider left margin — the thing that makes it a third
    // section rather than a repeat of the first.
    assert_eq!(sections[2].1.margins.start, wp_model::Twips(2880));
    assert_eq!(sections[0].1.margins.start, wp_model::Twips(1440));
}

#[test]
fn headers_and_footers_are_read_and_belong_to_their_section() {
    let document = open("headers-footers.docx");
    assert!(
        !document.headers.is_empty(),
        "the header parts were followed"
    );
    let referenced: usize = document
        .sections()
        .iter()
        .map(|(_, section)| section.headers.len() + section.footers.len())
        .sum();
    assert!(referenced > 0, "a section refers to them");
    assert_eq!(
        document.headers.len(),
        referenced,
        "one body per reference, and no orphans"
    );
    let text: String = document
        .headers
        .iter()
        .map(|header| wp_model::doc::text_of(&header.content))
        .collect();
    assert!(!text.trim().is_empty(), "the headers have text in them");
    assert!(
        document.headers.iter().any(|header| header.footer),
        "and one of them is a footer"
    );
}

#[test]
fn a_picture_watermark_is_a_picture_of_its_own_part_washed_out() {
    // Two things this is the only document in the corpus to say. A picture
    // inside a *header* names its relationship `rId1`, which is also the name
    // of the document's own first relationship, so the two can be told apart
    // only by the part they were written in. And the washout is what makes it
    // a watermark rather than a photograph over the text: Word states it as
    // `<a:lum bright="70000" contrast="-70000"/>`, which is three tenths of
    // the contrast and comes out as black at 205 — measured against Word's own
    // rendering of a ramp of every grey there is.
    let document = open("picture-watermark.docx");
    let bands: Vec<wp_model::Scope> = document
        .headers
        .iter()
        .map(|header| wp_model::Scope::Chrome(header.id))
        .collect();
    let drawing = bands
        .iter()
        .flat_map(|scope| document.paragraphs_in(*scope))
        .flat_map(|paragraph| paragraph.drawings())
        .find(|drawing| drawing.rel.is_some())
        .expect("the header holds a picture");
    let rel = drawing.rel.as_deref().expect("naming its part");
    assert!(
        rel.starts_with("header") && rel.contains(":rId"),
        "qualified by the part that named it, not left to collide with the document's own: {rel}"
    );
    let tone = drawing.tone.expect("and washed out");
    assert!((tone.gain - 0.3).abs() < 1e-3);
    assert_eq!(tone.apply(0), 205, "black comes out the grey Word draws");
    assert_eq!(tone.apply(255), 255);
}

#[test]
fn footnotes_are_read_and_the_separators_are_not_listed_as_notes() {
    let document = open("footnotes-endnotes.docx");
    let real: Vec<_> = document
        .footnotes
        .iter()
        .filter(|note| note.kind.is_real())
        .collect();
    assert!(!real.is_empty(), "there is at least one footnote");
    assert!(
        document.footnotes.len() > real.len(),
        "and the separators are there too, marked as what they are"
    );
    assert!(document.footnote(-1).is_none(), "id -1 is the separator");
    assert!(document.footnote(0).is_none(), "and id 0 is the other one");

    // Something in the body must point at the note, or the note is unreachable.
    let referenced = document.paragraphs().iter().any(|paragraph| {
        paragraph
            .runs()
            .iter()
            .flat_map(|run| &run.content)
            .any(|piece| matches!(piece, Piece::FootnoteRef { .. }))
    });
    assert!(referenced, "a run refers to a footnote");
    assert!(
        !document.endnotes.is_empty(),
        "the endnote part was followed too"
    );
}

#[test]
fn comments_are_read_with_their_anchors_in_the_text() {
    let document = open("comments.docx");
    assert!(!document.comments.is_empty(), "comments.xml was read");
    assert!(
        !document.comments[0].author.is_empty(),
        "a comment names its author"
    );
    assert!(
        !document.comments[0].text().trim().is_empty(),
        "and has something to say"
    );

    let paragraphs = document.paragraphs();
    let ranged = paragraphs.iter().any(|paragraph| {
        paragraph.content.iter().any(|inline| {
            matches!(
                inline,
                Inline::Anchor(wp_model::Anchor::CommentStart { .. })
            )
        })
    });
    let referenced = paragraphs.iter().any(|paragraph| {
        paragraph
            .runs()
            .iter()
            .flat_map(|run| &run.content)
            .any(|piece| matches!(piece, Piece::CommentRef(_)))
    });
    assert!(
        ranged && referenced,
        "the comment has both a range and the mark the balloon points at"
    );
}

#[test]
fn a_hyperlink_keeps_its_relationship_and_a_bookmark_keeps_its_name() {
    let document = open("hyperlinks-bookmarks.docx");
    let mut links = 0;
    let mut bookmarks = 0;
    for paragraph in document.paragraphs() {
        for inline in &paragraph.content {
            match inline {
                Inline::Hyperlink(link) => {
                    assert!(
                        link.rel.is_some() || link.anchor.is_some(),
                        "a hyperlink points somewhere"
                    );
                    links += 1;
                }
                Inline::Anchor(wp_model::Anchor::BookmarkStart { name, .. }) => {
                    assert!(!name.is_empty(), "a bookmark has a name");
                    bookmarks += 1;
                }
                _ => {}
            }
        }
    }
    assert!(links > 0, "there is a hyperlink");
    assert!(bookmarks > 0, "and a bookmark");
}

#[test]
fn a_floating_image_is_anchored_rather_than_inline() {
    let document = open("floating-image-wrap.docx");
    let drawings: Vec<_> = document
        .paragraphs()
        .iter()
        .flat_map(|paragraph| paragraph.runs())
        .flat_map(|run| &run.content)
        .filter_map(|piece| match piece {
            Piece::Drawing(drawing) => Some(drawing.as_ref().clone()),
            _ => None,
        })
        .collect();
    assert!(!drawings.is_empty(), "the drawing was read");
    let floating = drawings
        .iter()
        .find(|drawing| drawing.anchored)
        .expect("the image floats");
    assert!(floating.extent.0 .0 > 0, "it has a size");
    assert!(
        floating.rel.is_some(),
        "and names the part holding its bytes"
    );
    assert_ne!(floating.wrap, wp_model::Wrap::None, "text wraps around it");
    assert!(
        floating.position.is_some(),
        "an anchored drawing states where it sits"
    );
}

#[test]
fn a_content_control_keeps_its_identity() {
    let document = open("content-controls.docx");
    let mut found = 0;
    fn walk(blocks: &[Block], found: &mut usize) {
        for block in blocks {
            match block {
                Block::Structured(sdt) => {
                    *found += 1;
                    walk(&sdt.content, found);
                }
                Block::Paragraph(paragraph) => {
                    for inline in &paragraph.content {
                        if let Inline::Structured(sdt) = inline {
                            *found += 1;
                            assert!(
                                sdt.alias.is_some() || sdt.tag.is_some() || sdt.id.is_some(),
                                "a control is identifiable"
                            );
                        }
                    }
                }
                Block::Table(table) => {
                    for row in &table.rows {
                        for cell in &row.cells {
                            walk(&cell.content, found);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    walk(&document.body, &mut found);
    assert!(found > 0, "the content controls survived the read");
}

#[test]
fn right_to_left_text_arrives_whole_and_names_its_own_direction() {
    let document = open("rtl-and-cjk.docx");
    let text = document.text();
    assert!(
        text.chars().any(|c| ('\u{0590}'..='\u{08FF}').contains(&c)),
        "the RTL text is there: {text:?}"
    );
    let complex_face = document
        .paragraphs()
        .iter()
        .flat_map(|paragraph| paragraph.runs())
        .any(|run| run.props.fonts.complex.is_some() || run.props.rtl == Some(true));
    assert!(
        complex_face,
        "an Arabic run names a complex-script face or marks itself right-to-left"
    );
}

/// The scripts this document is named for, which for a long time it did not
/// hold: the generator wrote six lines and Word kept the last. Asserted here so
/// that a corpus file cannot quietly stop containing the thing it exists for —
/// text with no spaces between its words is the one case where "what is a word"
/// has no easy answer, and nothing else in the corpus asks the question.
#[test]
fn the_document_named_for_cjk_contains_cjk() {
    let text = open("rtl-and-cjk.docx").text();
    let han = text
        .chars()
        .filter(|c| ('\u{4E00}'..='\u{9FFF}').contains(c));
    assert!(han.count() >= 4, "Chinese: {text:?}");
    let kana = text
        .chars()
        .filter(|c| ('\u{3040}'..='\u{30FF}').contains(c));
    assert!(kana.count() >= 5, "Japanese: {text:?}");
    assert!(
        text.chars().any(|c| ('\u{0590}'..='\u{05FF}').contains(&c)),
        "Hebrew: {text:?}"
    );
}

#[test]
fn a_table_of_contents_field_keeps_its_instruction() {
    // `<w:fldSimple>` is the compact spelling of a field: the instruction on the
    // element, the cached result inside. Rewriting it as the begin/separate/end
    // triple would change every byte of a paragraph nobody edited.
    let document = open("styles-headings-toc.docx");
    let field = document.paragraphs()[0]
        .content
        .iter()
        .find_map(|inline| match inline {
            Inline::SimpleField {
                instruction,
                content,
            } => Some((instruction.clone(), content.clone())),
            _ => None,
        })
        .expect("the TOC is a simple field");
    assert!(
        field.0.contains("TOC"),
        "the instruction survives: {:?}",
        field.0
    );
    assert!(!field.1.is_empty(), "and so does the cached result");
    assert!(document
        .text()
        .starts_with("No table of contents entries found."));
}

#[test]
fn a_template_opens_as_a_document() {
    let document = open("template.dotx");
    assert!(!document.body.is_empty());
    assert!(document.styles.len() > 10);
}

#[test]
fn the_corpus_carries_no_pagination_oracle() {
    // Worth an assertion rather than a note, because C19 will want one and this
    // is where it would look. Word writes `<w:lastRenderedPageBreak>` when it
    // saves a document it has laid out on screen; these were produced through
    // COM without ever being displayed, so not one of them has any. The oracle
    // has to come from a document Word actually opened — see
    // `office_templates.rs`, and `PROGRESS.md` under C19.
    let total: usize = documents()
        .iter()
        .flat_map(|(_, document)| document.paragraphs())
        .map(|paragraph| paragraph.rendered_page_breaks())
        .sum();
    assert_eq!(
        total, 0,
        "if this ever fails, the corpus gained an oracle and C19 should use it"
    );
}
