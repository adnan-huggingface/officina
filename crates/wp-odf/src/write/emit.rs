//! Writing a paragraph the way OpenDocument writes one.
//!
//! Only what changed comes through here — an untouched paragraph is copied byte
//! for byte and never reaches this module — so everything below is about saying
//! the model's own vocabulary in the format's, and about saying it in a way the
//! reader gives back unchanged.
//!
//! **Whitespace is the one place the format decides the spelling for you.** ODF
//! collapses a run of spaces in text down to one (ODF 1.4 part 3 §6.1.2), so the
//! second and every later space of a run is written as `<text:s/>`, and so is a
//! space that would otherwise begin a line. A writer that emitted the characters
//! it holds would close up every gap in a document that lines its columns up
//! with spaces, and the file would still be well-formed while reading back
//! wrong.
//!
//! **A drawing goes back as the bytes it came in as.** The reader keeps a
//! `<draw:frame>`'s source on the model — the field the other format's reader
//! keeps a `<w:drawing>` in, for the same reason — because a frame carries a
//! graphic style, a title, a description, an anchor and possibly an object this
//! crate cannot draw, and editing the paragraph a picture sits in is an ordinary
//! thing to do.

use std::collections::HashMap;
use std::fmt::Write as _;

use wp_model::doc::{Block, Break, Document, Drawing, Hyperlink, Inline, Paragraph, Piece, Run};
use wp_model::revision::Anchor;
use wp_model::table::{Table, VMerge};

use super::auto::Automatic;
use super::splice::{escape_attr, escape_text};

/// Everything writing a paragraph needs that is not in the paragraph.
pub(crate) struct Out<'a> {
    pub(crate) document: &'a Document,
    pub(crate) auto: Automatic,
    /// The path inside the package a minted picture name stands for. ODF names
    /// a picture by where it sits rather than by a relationship, so this is the
    /// reverse of what the reader minted.
    pub(crate) pictures: HashMap<String, String>,
    /// The name each bookmark id was read from. A `<text:bookmark-end>` states
    /// the name and the model keeps only the id that paired it with its start.
    pub(crate) bookmarks: HashMap<u32, String>,
}

/// One block of a body, a cell or a note.
pub(crate) fn block(out: &mut String, block: &Block, w: &mut Out<'_>) {
    match block {
        Block::Paragraph(paragraph) => self::paragraph(out, paragraph, w),
        Block::Table(table) => self::table(out, table, w),
        Block::Structured(sdt) => {
            for block in &sdt.content {
                self::block(out, block, w);
            }
        }
        Block::Anchor(_) | Block::AltChunk { .. } => {}
    }
}

/// `<text:p>`, or `<text:h>` where the paragraph says how deep it is.
pub(crate) fn paragraph(out: &mut String, paragraph: &Paragraph, w: &mut Out<'_>) {
    let tag = match paragraph.props.outline_level {
        Some(_) => "text:h",
        None => "text:p",
    };
    let _ = write!(out, "<{tag}");
    if let Some(style) = w.auto.paragraph_style(&paragraph.props) {
        let _ = write!(out, r#" text:style-name="{}""#, escape_attr(&style));
    }
    if let Some(level) = paragraph.props.outline_level {
        // The model counts outline levels from zero as its usual format does;
        // ODF counts a heading's depth from one.
        let _ = write!(out, r#" text:outline-level="{}""#, level as u32 + 1);
    }
    if paragraph.content.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    // A space here would be the first thing on the line, and ODF drops one that
    // is, so the paragraph starts out owing `<text:s/>` for it.
    let mut fresh = true;
    inlines(out, &paragraph.content, w, &mut fresh);
    let _ = write!(out, "</{tag}>");
}

fn inlines(out: &mut String, items: &[Inline], w: &mut Out<'_>, fresh: &mut bool) {
    let mut at = 0;
    while at < items.len() {
        if let Some(next) = field(out, items, at, w, fresh) {
            at = next;
            continue;
        }
        inline(out, &items[at], w, fresh);
        at += 1;
    }
}

fn inline(out: &mut String, item: &Inline, w: &mut Out<'_>, fresh: &mut bool) {
    match item {
        Inline::Run(run) => self::run(out, run, w, fresh),
        Inline::Hyperlink(link) => hyperlink(out, link, w, fresh),
        Inline::Anchor(anchor) => self::anchor(out, anchor, w),
        // Wrappers the other format has and this one does not. Their content is
        // the document's; dropping the wrapper loses a fact about a producer's
        // intentions and dropping the content would lose the words.
        Inline::Revised { content, .. } | Inline::Wrapper { content, .. } => {
            inlines(out, content, w, fresh)
        }
        Inline::SimpleField { content, .. } => inlines(out, content, w, fresh),
        Inline::Structured(sdt) => inlines(out, &sdt.content, w, fresh),
        Inline::Math(_) => {}
    }
}

/// The page number, which is the one field this writer builds rather than
/// copies.
///
/// The reader turns `<text:page-number>` into the run of marks the layout
/// evaluates — begin, instruction, separate, the cached result, end — because a
/// page number cannot be known until the page exists. Writing it back means
/// recognising that run and folding it into one element again. Returns where to
/// carry on from, or nothing if this is not a field.
fn field(
    out: &mut String,
    items: &[Inline],
    at: usize,
    w: &mut Out<'_>,
    fresh: &mut bool,
) -> Option<usize> {
    let Inline::Run(run) = &items[at] else {
        return None;
    };
    let [Piece::FieldStart { .. }, Piece::Instruction(instruction), Piece::FieldSeparate] =
        run.content.as_slice()
    else {
        return None;
    };
    let end = items[at + 1..].iter().position(
        |item| matches!(item, Inline::Run(run) if run.content.as_slice() == [Piece::FieldEnd]),
    )?;
    let end = at + 1 + end;
    let shown = &items[at + 1..end];
    let element = match instruction.trim().to_ascii_uppercase().as_str() {
        "PAGE" => Some(("text:page-number", r#" text:select-page="current""#)),
        "NUMPAGES" => Some(("text:page-count", "")),
        // Any other field arrives with the text it last showed, and that text is
        // what a rendering has. Writing the result and not the instruction is
        // the same trade the reader made in the other direction.
        _ => None,
    };
    match element {
        Some((name, attrs)) => {
            let _ = write!(out, "<{name}{attrs}>");
            inlines(out, shown, w, fresh);
            let _ = write!(out, "</{name}>");
        }
        None => inlines(out, shown, w, fresh),
    }
    Some(end + 1)
}

/// A run, inside a `<text:span>` where it has formatting of its own.
fn run(out: &mut String, run: &Run, w: &mut Out<'_>, fresh: &mut bool) {
    let style = w.auto.run_style(&run.props);
    if let Some(style) = &style {
        let _ = write!(
            out,
            r#"<text:span text:style-name="{}">"#,
            escape_attr(style)
        );
    }
    for piece in &run.content {
        self::piece(out, piece, w, fresh);
    }
    if style.is_some() {
        out.push_str("</text:span>");
    }
}

fn piece(out: &mut String, piece: &Piece, w: &mut Out<'_>, fresh: &mut bool) {
    match piece {
        Piece::Text(text) => self::text(out, text, fresh),
        Piece::Tab => {
            out.push_str("<text:tab/>");
            *fresh = false;
        }
        Piece::Break(Break::Line) => {
            out.push_str("<text:line-break/>");
            // A line break starts a line, and a space at the start of one goes
            // the same way as a space at the start of a paragraph.
            *fresh = true;
        }
        // A page or column break is a property of a paragraph in this format,
        // not a mark inside one, and the paragraph's style carries it.
        Piece::Break(_) => {}
        Piece::Hyphen { breaking } => match breaking {
            true => self::text(out, "\u{00AD}", fresh),
            false => self::text(out, "\u{2011}", fresh),
        },
        Piece::Symbol { ch, .. } => {
            let mut buffer = [0u8; 4];
            self::text(out, ch.encode_utf8(&mut buffer), fresh);
        }
        Piece::Drawing(drawing) => {
            self::drawing(out, drawing, w);
            *fresh = false;
        }
        Piece::FootnoteRef { id, .. } => note(out, *id, false, w),
        Piece::EndnoteRef { id, .. } => note(out, *id, true, w),
        // Text inside a tracked deletion is drawn and is not in the document;
        // the marks around a field are folded away by `field` above; the rest
        // are the other format's furniture and have no spelling here.
        _ => {}
    }
}

/// Text, with the spaces this format cannot state as characters written as the
/// element that stands for them.
fn text(out: &mut String, text: &str, fresh: &mut bool) {
    let mut plain = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != ' ' {
            plain.push(c);
            *fresh = false;
            continue;
        }
        if !*fresh {
            // The first space of a run is a space, and survives being read back.
            plain.push(' ');
            *fresh = true;
            continue;
        }
        let mut count = 1u32;
        while chars.peek() == Some(&' ') {
            chars.next();
            count += 1;
        }
        out.push_str(&escape_text(&plain));
        plain.clear();
        match count {
            1 => out.push_str("<text:s/>"),
            n => {
                let _ = write!(out, r#"<text:s text:c="{n}"/>"#);
            }
        }
    }
    out.push_str(&escape_text(&plain));
}

fn hyperlink(out: &mut String, link: &Hyperlink, w: &mut Out<'_>, fresh: &mut bool) {
    // ODF has no relationship part: a link states its target where it is, and a
    // link into the document is that target's name behind a `#`.
    let href = match (&link.anchor, &link.rel) {
        (Some(anchor), _) => format!("#{anchor}"),
        (None, Some(rel)) => rel.to_string(),
        (None, None) => String::new(),
    };
    let _ = write!(
        out,
        r#"<text:a xlink:type="simple" xlink:href="{}">"#,
        escape_attr(&href)
    );
    inlines(out, &link.content, w, fresh);
    out.push_str("</text:a>");
}

fn anchor(out: &mut String, anchor: &Anchor, w: &mut Out<'_>) {
    match anchor {
        Anchor::BookmarkStart { name, .. } => {
            let _ = write!(
                out,
                r#"<text:bookmark-start text:name="{}"/>"#,
                escape_attr(name)
            );
        }
        Anchor::BookmarkEnd { id } => {
            if let Some(name) = w.bookmarks.get(id) {
                let _ = write!(
                    out,
                    r#"<text:bookmark-end text:name="{}"/>"#,
                    escape_attr(name)
                );
            }
        }
        // A comment, a permission range: neither is modelled by this reader, so
        // neither can be in a document it read.
        _ => {}
    }
}

/// A footnote or an endnote, body and all.
///
/// ODF keeps a note where it is referenced rather than in a part of its own, so
/// writing one back means writing the whole note at the mark.
fn note(out: &mut String, id: i32, endnote: bool, w: &mut Out<'_>) {
    let document = w.document;
    let notes = match endnote {
        true => &document.endnotes,
        false => &document.footnotes,
    };
    let Some(note) = notes.iter().find(|note| note.id == id) else {
        return;
    };
    let (class, prefix) = match endnote {
        true => ("endnote", "edn"),
        false => ("footnote", "ftn"),
    };
    let _ = write!(
        out,
        r#"<text:note text:id="{prefix}{id}" text:note-class="{class}"><text:note-citation>{id}</text:note-citation><text:note-body>"#
    );
    for block in &note.content {
        self::block(out, block, w);
    }
    out.push_str("</text:note-body></text:note>");
}

/// A `<draw:frame>`, as the bytes it arrived as where there are any.
///
/// A drawing this application authored has no source, and what it can be
/// written as is what the model holds: a box, and the path in the package the
/// picture sits at.
fn drawing(out: &mut String, drawing: &Drawing, w: &mut Out<'_>) {
    if !drawing.source.is_empty() {
        if let Ok(source) = std::str::from_utf8(&drawing.source) {
            out.push_str(source);
            return;
        }
    }
    let Some(href) = drawing
        .rel
        .as_deref()
        .and_then(|rel| w.pictures.get(rel))
        .cloned()
    else {
        // A frame with nothing to point at is a blank rectangle on the page.
        return;
    };
    let anchor = match drawing.anchored {
        true => "paragraph",
        false => "as-char",
    };
    let _ = write!(
        out,
        r#"<draw:frame text:anchor-type="{anchor}" svg:width="{}pt" svg:height="{}pt""#,
        trim(drawing.extent.0.points()),
        trim(drawing.extent.1.points())
    );
    if let Some(name) = &drawing.name {
        let _ = write!(out, r#" draw:name="{}""#, escape_attr(name));
    }
    let _ = write!(
        out,
        r#"><draw:image xlink:href="{}" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/>"#,
        escape_attr(&href)
    );
    if let Some(description) = &drawing.description {
        let _ = write!(out, "<svg:desc>{}</svg:desc>", escape_text(description));
    }
    out.push_str("</draw:frame>");
}

/// A table the model holds and the file does not.
///
/// **A table already in the file is never written by this.** It is spliced
/// through instead — see `write::mod` — because an ODF table states its widths,
/// its shading and its borders in automatic styles named from every column, row
/// and cell, and rewriting one would mean minting the lot. This is for a table
/// authored here, which by construction has nothing in it that is not modelled.
pub(crate) fn table(out: &mut String, table: &Table, w: &mut Out<'_>) {
    let columns = table.grid.len().max(
        table
            .rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|c| c.props.grid_span.max(1) as usize)
                    .sum()
            })
            .max()
            .unwrap_or(1),
    );
    out.push_str("<table:table");
    if let Some(name) = &table.props.caption {
        let _ = write!(out, r#" table:name="{}""#, escape_attr(name));
    }
    out.push('>');
    let _ = write!(
        out,
        r#"<table:table-column table:number-columns-repeated="{columns}"/>"#
    );
    for row in &table.rows {
        out.push_str("<table:table-row>");
        for cell in &row.cells {
            if cell.props.v_merge == Some(VMerge::Continue) {
                out.push_str("<table:covered-table-cell/>");
                continue;
            }
            out.push_str("<table:table-cell");
            if cell.props.grid_span > 1 {
                let _ = write!(
                    out,
                    r#" table:number-columns-spanned="{}""#,
                    cell.props.grid_span
                );
            }
            out.push('>');
            for block in &cell.content {
                self::block(out, block, w);
            }
            out.push_str("</table:table-cell>");
            // Every position a span covers is spelled out, or the row is short.
            for _ in 1..cell.props.grid_span {
                out.push_str("<table:covered-table-cell/>");
            }
        }
        out.push_str("</table:table-row>");
    }
    out.push_str("</table:table>");
}

fn trim(value: f64) -> String {
    let text = format!("{value:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    match text.is_empty() || text == "-" {
        true => "0".to_owned(),
        false => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn writer<'a>(document: &'a Document, read: &'a crate::styles::Styles) -> Out<'a> {
        Out {
            document,
            auto: Automatic::new(document.styles.clone(), read),
            pictures: HashMap::new(),
            bookmarks: HashMap::new(),
        }
    }

    fn emitted(paragraph: &Paragraph) -> String {
        let document = Document::new();
        let read = crate::styles::Styles::default();
        let mut w = writer(&document, &read);
        let mut out = String::new();
        self::paragraph(&mut out, paragraph, &mut w);
        out
    }

    #[test]
    fn a_paragraph_of_plain_text_is_a_text_p() {
        assert_eq!(
            emitted(&Paragraph::of("hello")),
            "<text:p>hello</text:p>".to_owned()
        );
    }

    /// The rule the format states and the model does not: a second space is not
    /// a character here.
    #[test]
    fn every_space_after_the_first_is_an_element() {
        assert_eq!(
            emitted(&Paragraph::of("a  b")),
            r#"<text:p>a <text:s/>b</text:p>"#
        );
        assert_eq!(
            emitted(&Paragraph::of("a    b")),
            r#"<text:p>a <text:s text:c="3"/>b</text:p>"#
        );
        // And a space that would begin the line, which is dropped outright.
        assert_eq!(
            emitted(&Paragraph::of("  indented")),
            r#"<text:p><text:s text:c="2"/>indented</text:p>"#
        );
    }

    #[test]
    fn a_tab_and_a_line_break_are_elements_of_their_own() {
        let paragraph = Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![
                    Piece::Text("a".into()),
                    Piece::Tab,
                    Piece::Text("b".into()),
                    Piece::Break(Break::Line),
                    Piece::Text("c".into()),
                ],
                ..Run::default()
            })],
            ..Paragraph::default()
        };
        assert_eq!(
            emitted(&paragraph),
            "<text:p>a<text:tab/>b<text:line-break/>c</text:p>"
        );
    }

    #[test]
    fn text_is_escaped_the_way_the_format_escapes_it() {
        assert_eq!(
            emitted(&Paragraph::of("R&D <tag>")),
            "<text:p>R&amp;D &lt;tag&gt;</text:p>"
        );
    }

    #[test]
    fn a_heading_is_a_text_h_that_states_its_depth() {
        let paragraph = Paragraph {
            props: wp_model::prop::ParaProps {
                outline_level: Some(1),
                ..Default::default()
            },
            content: vec![Inline::Run(Run::of("Chapter"))],
            ..Paragraph::default()
        };
        assert_eq!(
            emitted(&paragraph),
            r#"<text:h text:outline-level="2">Chapter</text:h>"#
        );
    }

    #[test]
    fn a_run_that_carries_a_style_is_wrapped_in_a_span() {
        let mut document = Document::new();
        let emphasis = document
            .styles
            .intern("Emphasis", wp_model::StyleKind::Character);
        let read = crate::styles::Styles::default();
        let paragraph = Paragraph {
            content: vec![
                Inline::Run(Run::of("plain ")),
                Inline::Run(Run {
                    props: wp_model::prop::RunProps {
                        style: Some(emphasis),
                        ..Default::default()
                    },
                    content: vec![Piece::Text("loud".into())],
                    ..Run::default()
                }),
            ],
            ..Paragraph::default()
        };
        let mut w = writer(&document, &read);
        let mut out = String::new();
        self::paragraph(&mut out, &paragraph, &mut w);
        assert_eq!(
            out,
            r#"<text:p>plain <text:span text:style-name="Emphasis">loud</text:span></text:p>"#
        );
    }

    /// The claim the whole writer rests on: what it emits, the reader gives
    /// back unchanged.
    #[test]
    fn an_emitted_paragraph_reads_back_as_itself() {
        let document = Document::new();
        let read = crate::styles::Styles::default();
        let paragraph = Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![
                    Piece::Text("two  spaces and a".into()),
                    Piece::Tab,
                    Piece::Text("tab & an ampersand".into()),
                ],
                ..Run::default()
            })],
            ..Paragraph::default()
        };
        let mut w = writer(&document, &read);
        let mut out = String::new();
        self::paragraph(&mut out, &paragraph, &mut w);

        let container = crate::Container::empty(crate::container::TEXT_MIMETYPE);
        let mut ctx = crate::Ctx::for_tests(&container);
        let read_back = crate::content::block_of(out.as_bytes(), &mut ctx, &Vec::new())
            .expect("it reads back as a block");
        assert_eq!(read_back, Block::Paragraph(paragraph), "{out}");
    }

    #[test]
    fn a_table_authored_here_reads_back_with_its_cells() {
        use wp_model::table::{Cell, Row, RowProps};
        let document = Document::new();
        let read = crate::styles::Styles::default();
        let cell = |text: &str| Cell {
            content: vec![Block::Paragraph(Paragraph::of(text))],
            ..Cell::new()
        };
        let mut table = Table::new();
        table.grid = vec![wp_model::Twips(0), wp_model::Twips(0)];
        table.rows.push(Row {
            props: RowProps::default(),
            cells: vec![cell("left"), cell("right")],
        });
        let mut w = writer(&document, &read);
        let mut out = String::new();
        self::table(&mut out, &table, &mut w);

        let container = crate::Container::empty(crate::container::TEXT_MIMETYPE);
        let mut ctx = crate::Ctx::for_tests(&container);
        let Some(Block::Table(read_back)) =
            crate::content::block_of(out.as_bytes(), &mut ctx, &Vec::new())
        else {
            panic!("a table reads back from {out}");
        };
        assert_eq!(read_back.text(), "left\tright\n", "{out}");
    }
}
