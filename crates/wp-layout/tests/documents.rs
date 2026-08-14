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
    let theme = document.theme.clone();
    let ctx = Context {
        theme: &theme,
        default_tab: document.settings.default_tab_stop,
        fallback_font: "test",
        show_revisions: true,
        show_hidden: false,
    };
    let mut shaper = Fixed;
    block::layout(document, &ctx, &mut shaper)
}

/// Everything the layout drew, with whitespace collapsed.
fn drawn_text(pages: &[block::Page]) -> String {
    let mut out = String::new();
    for page in pages {
        for placement in &page.content {
            if let Placed::Line { line, .. } = &placement.kind {
                for fragment in &line.fragments {
                    match &fragment.content {
                        Content::Text { text, .. } => out.push_str(text),
                        Content::Tab { .. } => out.push(' '),
                        _ => {}
                    }
                }
            }
        }
    }
    out
}

/// The document's text as a revision-showing view draws it.
fn shown_text(document: &Document) -> String {
    document
        .paragraphs()
        .iter()
        .map(|paragraph| paragraph.shown_text())
        .collect::<Vec<_>>()
        .join("\n")
}

fn squashed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
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
        let expected = squashed(&shown_text(&document));
        let drawn = squashed(&drawn_text(&pages));
        // Small capitals are drawn as capitals and a field's instruction is
        // never drawn, so the comparison is on the letters rather than on the
        // exact string.
        let expected_letters: String = expected
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(|c| c.to_lowercase())
            .collect();
        let drawn_letters: String = drawn
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(|c| c.to_lowercase())
            .collect();
        assert_eq!(
            expected_letters, drawn_letters,
            "{name}: the page does not say what the document says"
        );
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
