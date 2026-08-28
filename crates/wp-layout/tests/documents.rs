//! The layout engine against real documents.
//!
//! The unit tests measure the engine with arithmetic. These run it over the
//! corpus and over the templates Office ships, and ask the two questions a
//! layout engine can be wrong about without any test noticing: **is anything
//! lost**, and **does anything reach past the margin**.
//!
//! Both are asked of the *laid-out result* rather than of the model. The
//! upside-down bar chart in `LEARNINGS.md` §5 survived a whole chunk because
//! every test asked the model; this is the answer to that.
//!
//! Page *counts* are not compared with Word's. They cannot be: these run through
//! [`Fixed`], whose glyphs are half their point size, so every line holds a
//! different number of characters than a real face would. The comparison against
//! `<w:lastRenderedPageBreak>` is printed rather than asserted, and it becomes
//! meaningful in C20 where a real shaper exists.

use std::path::{Path, PathBuf};

use wp_layout::block::{self, Placed};
use wp_layout::inline::{Content, Context};
use wp_layout::shape::Fixed;
use wp_layout::Memo;
use wp_model::Document;

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/docx")
        .canonicalize()
        .expect("the corpus is in the repository")
}

fn documents() -> Vec<(String, Document)> {
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
    out
}

fn office_templates() -> Vec<(String, Document)> {
    let dirs = [
        r"C:\Program Files\Microsoft Office\root\Templates\1033",
        r"C:\Program Files\Microsoft Office\root\Office16\1033\QuickStyles",
    ];
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .and_then(|e| e.to_str())
                .is_none_or(|e| !e.eq_ignore_ascii_case("dotx"))
            {
                continue;
            }
            if let Ok((document, _)) = wp_docx::open(&path) {
                out.push((
                    path.file_name().unwrap().to_string_lossy().into_owned(),
                    document,
                ));
            }
        }
    }
    out
}

fn lay(document: &Document) -> Vec<block::Page> {
    lay_with(document, None)
}

fn lay_with(document: &Document, memo: Option<&Memo>) -> Vec<block::Page> {
    let theme = document.theme.clone();
    let marks = wp_layout::NoteMarks::of(document);
    let contents = wp_layout::field::Contents::of(document);
    let ctx = Context {
        theme: &theme,
        styles: &document.styles,
        notes: &marks,
        contents: &contents,
        note_mark: None,
        table_part: None,
        default_tab: document.settings.default_tab_stop,
        no_leading: document.settings.no_leading,
        no_tab_for_hanging_indent: document.settings.no_tab_for_hanging_indent,
        fallback_font: "test",
        has_face: |_| false,
        show_revisions: true,
        show_hidden: false,
        fields: &wp_layout::FieldValues::new(),
        band: None,
        memo,
        wraps: &wp_layout::block::Wraps::default(),
    };
    let mut shaper = Fixed;
    block::layout(document, &ctx, &mut shaper)
}

/// The first paragraph of the body with words in it, and where it stands.
fn first_words(document: &mut Document) -> Option<&mut wp_model::doc::Paragraph> {
    document.body.iter_mut().find_map(|block| match block {
        wp_model::doc::Block::Paragraph(paragraph) if !paragraph.text().is_empty() => {
            Some(paragraph)
        }
        _ => None,
    })
}

/// What the layout drew for each paragraph, keyed by the paragraph index the
/// line placements carry.
///
/// Per paragraph rather than per page: a table row is flowed in bands across
/// its cells, so the page's raw placement order interleaves the cells of a row
/// — the second line of one cell comes after the first line of its neighbour.
/// Within one paragraph the placements stay in reading order.
fn drawn_by_paragraph(pages: &[block::Page], count: usize) -> Vec<String> {
    let mut out = vec![String::new(); count];
    for page in pages {
        for placement in &page.content {
            if let Placed::Line { line, paragraph } = &placement.kind {
                let Some(into) = out.get_mut(*paragraph) else {
                    continue;
                };
                for fragment in &line.fragments {
                    match &fragment.content {
                        Content::Text { text, .. } => into.push_str(text),
                        Content::Tab { .. } => into.push(' '),
                        _ => {}
                    }
                }
            }
        }
    }
    out
}

/// Small capitals are drawn as capitals and a field's instruction is never
/// drawn, so text is compared on its letters rather than exactly.
fn letters(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

#[test]
fn every_document_in_the_corpus_lays_out() {
    for (name, document) in documents() {
        let pages = lay(&document);
        assert!(!pages.is_empty(), "{name}: laid out to nothing");
        for page in &pages {
            assert!(page.geometry.text_width() > 0.0, "{name}");
        }
    }
}

#[test]
fn nothing_is_lost_between_the_document_and_the_page() {
    // The check a layout engine most needs and least often has: the text that
    // reached the page is the text that was in the document. A run silently
    // skipped — a script the itemizer did not classify, a piece the walker did
    // not handle — is invisible from the model and obvious from here.
    for (name, document) in documents() {
        let pages = lay(&document);
        // The layout above shows revisions, so the text to compare against is
        // the one that is *drawn* — deletions included — rather than the one
        // that is in the document. `Document::text` is the other question.
        // Comparing paragraph by paragraph also proves the line placements
        // name the right paragraph, which is what a click stands on.
        let paragraphs = document.paragraphs();
        let drawn = drawn_by_paragraph(&pages, paragraphs.len());
        for (index, paragraph) in paragraphs.iter().enumerate() {
            assert_eq!(
                letters(&paragraph.shown_text()),
                letters(&drawn[index]),
                "{name}, paragraph {index}: the page does not say what the document says"
            );
        }
    }
}

#[test]
fn no_line_reaches_past_the_paper_it_is_printed_on() {
    // Past the *margin* is legal and common: a negative right indent and a table
    // wider than the text column both do it deliberately, and several of the
    // templates Office ships are built that way. Past the paper's edge is not —
    // it is text that no printer will put on the page.
    let mut past_the_margin = 0usize;
    for (name, document) in documents().into_iter().chain(office_templates()) {
        let pages = lay(&document);
        for page in &pages {
            let margin = page.geometry.width - page.geometry.end;
            for placement in &page.content {
                let Placed::Line { line, .. } = &placement.kind else {
                    continue;
                };
                let reach = placement.x + line.width;
                if reach > margin + 1.0 {
                    past_the_margin += 1;
                }
                assert!(
                    reach <= page.geometry.width + 1.0,
                    "{name}: a line reaches to {reach:.1} on paper {:.1} wide",
                    page.geometry.width
                );
                assert!(
                    placement.x >= -1.0,
                    "{name}: a line starts left of the paper"
                );
            }
        }
    }
    eprintln!("{past_the_margin} lines reach into a margin, which is legal");
}

#[test]
fn every_page_stays_inside_its_own_paper() {
    for (name, document) in documents().into_iter().chain(office_templates()) {
        for page in lay(&document) {
            let bottom = page.geometry.height;
            for placement in page.everything() {
                assert!(
                    placement.y >= -1.0 && placement.y <= bottom + 1.0,
                    "{name}: something was placed at y={:.1} on a page {bottom:.1} tall",
                    placement.y
                );
            }
        }
    }
}

#[test]
fn the_templates_office_ships_lay_out_too() {
    let templates = office_templates();
    if templates.is_empty() {
        eprintln!("skipped: Office is not installed");
        return;
    }
    let mut total_pages = 0;
    for (name, document) in &templates {
        let pages = lay(document);
        assert!(!pages.is_empty(), "{name}: laid out to nothing");
        total_pages += pages.len();
    }
    eprintln!(
        "{} templates laid out to {total_pages} pages",
        templates.len()
    );
    assert!(total_pages >= templates.len());
}

#[test]
fn words_own_page_breaks_are_where_ours_would_be_compared() {
    // `<w:lastRenderedPageBreak>` is the only pagination oracle a .docx holds.
    // It cannot be asserted against a fixed-width shaper — every line holds a
    // different number of characters than a real face would — so the comparison
    // is printed. It is here so that C20, which has real fonts, has somewhere to
    // put the assertion, and so the oracle's existence is not forgotten.
    let mut compared = 0;
    for (name, document) in office_templates() {
        let words: usize = document
            .paragraphs()
            .iter()
            .map(|paragraph| paragraph.rendered_page_breaks())
            .sum();
        if words == 0 {
            continue;
        }
        let ours = lay(&document).len();
        eprintln!("{name}: Word broke {} times, we made {ours} pages", words);
        compared += 1;
    }
    eprintln!("{compared} documents carry Word's own opinion of where the pages ended");
}

/// **A remembered layout has to be the layout that was remembered**, or the
/// cache is a silent corruption of the document rather than a saving. It is not
/// enough to compare page counts: the whole of every page is compared, every
/// placement and every fragment and every coordinate, over every document in
/// the corpus — which is where the headers, the footnotes, the tables, the
/// fields and the numbered lists are.
///
/// Three layouts, because the memo has three states and each can be wrong on
/// its own: none at all, an empty one it fills, and a full one it answers from.
#[test]
fn a_layout_answered_from_the_memo_is_the_layout_that_was_laid() {
    for (name, document) in documents() {
        let plain = lay(&document);
        let memo = Memo::new();
        let cold = lay_with(&document, Some(&memo));
        assert_eq!(cold, plain, "{name}: with an empty memo");
        let warm = lay_with(&document, Some(&memo));
        assert_eq!(warm, plain, "{name}: laid again from the memo");
        let (hits, _) = memo.tally();
        assert!(
            hits > 0 || document.paragraphs().is_empty(),
            "{name}: nothing was recalled, so the comparison proved nothing"
        );
    }
}

/// The keystroke: one paragraph changed and the rest of the document recalled.
///
/// The paragraph that was typed into must be laid again — a memo that answered
/// for it would show the letter as never typed — and the pages must come out
/// exactly as a layout that had never seen the document before.
#[test]
fn a_paragraph_typed_into_is_the_one_that_is_laid_again() {
    for (name, mut document) in documents() {
        let memo = Memo::new();
        lay_with(&document, Some(&memo));
        let Some(paragraph) = first_words(&mut document) else {
            continue;
        };
        paragraph.content = vec![wp_model::doc::Inline::Run(wp_model::doc::Run::of(
            "A word nobody in this document has written before.",
        ))];
        let recalled = lay_with(&document, Some(&memo));
        assert_eq!(
            recalled,
            lay(&document),
            "{name}: after one paragraph changed"
        );
    }
}

/// Return: a paragraph inserted, which moves every paragraph after it by one.
///
/// A memo that looked a paragraph up by its own index alone would miss all of
/// them, so this asserts both answers — that the pages are right, and that they
/// were mostly recalled rather than laid.
#[test]
fn splitting_a_paragraph_does_not_cost_the_whole_document() {
    for (name, mut document) in documents() {
        if document.paragraphs().len() < 8 {
            continue;
        }
        let memo = Memo::new();
        lay_with(&document, Some(&memo));
        document.body.insert(
            0,
            wp_model::doc::Block::Paragraph(wp_model::doc::Paragraph::new()),
        );
        let recalled = lay_with(&document, Some(&memo));
        assert_eq!(
            recalled,
            lay(&document),
            "{name}: after a paragraph was inserted"
        );
        let (hits, misses) = memo.tally();
        assert!(
            hits > misses,
            "{name}: {hits} recalled against {misses} laid — the shift was not followed"
        );
    }
}

/// **The memo is emptied by comparison, not by being told.** A style edit
/// changes every paragraph that wears it while changing none of them, so a
/// cache keyed on the paragraphs alone would answer with the old lines for ever
/// and no command would be at fault.
#[test]
fn editing_a_style_empties_the_memo_without_anybody_saying_so() {
    let (name, mut document) = documents()
        .into_iter()
        .find(|(_, document)| document.paragraphs().len() > 4)
        .expect("the corpus holds a document with some text in it");
    let memo = Memo::new();
    lay_with(&document, Some(&memo));
    let normal = document
        .styles
        .lookup("Normal")
        .or_else(|| document.styles.iter().next().map(|(id, _)| id))
        .expect("a document has styles");
    document
        .styles
        .get_mut(normal)
        .expect("the style just found")
        .para
        .spacing
        .after = Some(wp_model::units::Twips(1234));
    let recalled = lay_with(&document, Some(&memo));
    // The proof is the pages themselves. A memo that had kept its entries would
    // answer for every paragraph whose own text had not changed, which is all of
    // them, and the spacing the style now states would appear nowhere.
    assert_eq!(recalled, lay(&document), "{name}: after a style changed");
    let (hits, misses) = memo.tally();
    assert!(misses > hits, "{name}: {hits} recalled, {misses} laid");
}
