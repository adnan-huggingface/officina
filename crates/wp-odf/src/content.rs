//! `content.xml` and `styles.xml` — the two parts that between them are the
//! document.
//!
//! They have the same shape and are read by the same code: font declarations,
//! then automatic styles, then the part's own thing — a body in one, the named
//! styles and the master pages in the other. Reading them the same way is not
//! only tidiness; `styles.xml`'s master pages *contain a body*, because a
//! header is paragraphs and tables like any other, and a reader that had a
//! separate simpler path for them would be the reader that cannot show the
//! sample document's header, which is a table.
//!
//! **A list is a structure in ODF and a property in the model.** `<text:list>`
//! wraps `<text:list-item>` wraps paragraphs, and nesting is what says which
//! level a paragraph is at; the model hangs a `NumRef` on the paragraph and
//! keeps the level as a number. So the reader carries the stack of lists it is
//! inside, and the depth of that stack *is* the level.

use std::sync::Arc;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use wp_model::doc::{
    Block, Break, HeaderFooter, Hyperlink, Inline, Note, NoteKind, Paragraph, Piece, Run,
};
use wp_model::prop::{NumRef, ParaProps, RunProps};
use wp_model::revision::Anchor;
use wp_model::section::{HeaderId, HeaderKind, HeaderRef};
use wp_model::style::StyleKind;

use crate::xml::{attr_in, end_local_name, local_name, push_text, skip_element};
use crate::{Ctx, Error, Master, Result};

/// Which of the two parts is being read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Which {
    Content,
    Styles,
    /// The stylesheets alone: no body, and no master pages either. What the
    /// writer needs to put a context back in the state the reader was in at the
    /// moment it began reading a body.
    Stylesheet,
}

/// Reads one part, and returns the body it held — empty for `styles.xml`,
/// whose content is the master pages and reaches the caller through `ctx`.
pub fn part<'a>(bytes: &'a [u8], ctx: &mut Ctx<'a>, which: Which) -> Result<Vec<Block>> {
    let text = std::str::from_utf8(bytes).map_err(|e| Error::Xml(format!("not UTF-8: {e}")))?;
    // A byte-order mark is dropped before anything counts bytes, so that the
    // offsets the reader hands out index the same buffer the parser walks —
    // which is what lets a drawing be kept as the bytes it was read from.
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    ctx.source = text.as_bytes();
    let mut reader = Reader::from_str(text);
    let mut body = Vec::new();
    loop {
        let event = reader
            .read_event()
            .map_err(|e| Error::Xml(format!("{e}")))?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(&e).to_vec();
                match name.as_slice() {
                    b"font-face-decls" if !empty => {
                        crate::fonts::declarations(&mut reader, &mut ctx.styles.faces)
                    }
                    b"automatic-styles" if !empty => {
                        stylesheet(&mut reader, b"automatic-styles", ctx)
                    }
                    b"styles" if !empty => stylesheet(&mut reader, b"styles", ctx),
                    // The master pages hold bodies of their own, so a caller
                    // that wants the stylesheets and nothing that numbers a
                    // note or mints a picture name asks for `Stylesheet`. The
                    // writer does, because it is about to read those bodies
                    // itself and every id has to land where the first reading
                    // put it.
                    b"master-styles" if !empty && which != Which::Stylesheet => {
                        master_styles(&mut reader, ctx)
                    }
                    b"text" if !empty && which == Which::Content => {
                        body = blocks(&mut reader, b"text", ctx)
                    }
                    // Nothing else is skipped, and that is the point: the part's
                    // root element and `<office:body>` are wrappers, and a scan
                    // that stepped over an element it did not recognise would
                    // step over the whole document at its first event.
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(body)
}

/// `<office:styles>` or `<office:automatic-styles>`. The page layouts live in
/// the second of them, which is why this is here rather than in `styles.rs`.
fn stylesheet(reader: &mut Reader<&[u8]>, end: &[u8], ctx: &mut Ctx<'_>) {
    crate::styles::read(
        reader,
        end,
        &mut ctx.table,
        &mut ctx.styles,
        &mut ctx.numbering,
        &mut ctx.layouts,
    );
}

/// `<office:master-styles>` — the master pages, and the header and footer of
/// each.
fn master_styles(reader: &mut Reader<&[u8]>, ctx: &mut Ctx<'_>) {
    while let Ok(event) = reader.read_event() {
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(&e).to_vec();
                if name == b"master-page" {
                    let master = master_page(reader, &e, empty, ctx);
                    ctx.masters.push(master);
                } else if !empty {
                    skip_element(reader, &name);
                }
            }
            Event::End(e) if end_local_name(&e) == b"master-styles" => return,
            Event::Eof => return,
            _ => {}
        }
    }
}

fn master_page(
    reader: &mut Reader<&[u8]>,
    e: &BytesStart<'_>,
    empty: bool,
    ctx: &mut Ctx<'_>,
) -> Master {
    let mut master = Master {
        layout: attr_in(e, b"style", b"page-layout-name").unwrap_or_default(),
        headers: Vec::new(),
        footers: Vec::new(),
    };
    if empty {
        return master;
    }
    loop {
        let event = match reader.read_event() {
            Ok(event) => event,
            Err(_) => return master,
        };
        let inner_empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(&e).to_vec();
                let band = match name.as_slice() {
                    b"header" => Some((false, HeaderKind::Default)),
                    b"header-left" => Some((false, HeaderKind::Even)),
                    b"header-first" => Some((false, HeaderKind::First)),
                    b"footer" => Some((true, HeaderKind::Default)),
                    b"footer-left" => Some((true, HeaderKind::Even)),
                    b"footer-first" => Some((true, HeaderKind::First)),
                    _ => None,
                };
                match band {
                    Some((footer, kind)) if !inner_empty => {
                        let content = blocks(reader, &name, ctx);
                        let id = HeaderId(ctx.headers.len() as u32);
                        ctx.headers.push(HeaderFooter {
                            id,
                            part: None,
                            rel: None,
                            footer,
                            content,
                        });
                        let reference = HeaderRef {
                            kind,
                            body: id,
                            rel: None,
                        };
                        match footer {
                            true => master.footers.push(reference),
                            false => master.headers.push(reference),
                        }
                    }
                    _ if !inner_empty => skip_element(reader, &name),
                    _ => {}
                }
            }
            Event::End(e) if end_local_name(&e) == b"master-page" => return master,
            Event::Eof => return master,
            _ => {}
        }
    }
}

/// Everything between here and `end` that is a block.
pub fn blocks(reader: &mut Reader<&[u8]>, end: &[u8], ctx: &mut Ctx<'_>) -> Vec<Block> {
    let mut out = Vec::new();
    let mut list = Vec::new();
    blocks_into(reader, end, ctx, &mut out, &mut list);
    out
}

/// The list stack: which list style each level is in, innermost last.
pub(crate) type Lists = Vec<Option<u32>>;

/// The list style a `<text:list>` puts its paragraphs in.
///
/// Stated here or inherited from the list this one is inside — a nested list
/// usually names nothing and continues its parent's definition. The writer
/// walks the same structure and asks the same question, which is why this is a
/// function rather than four lines in one place.
pub(crate) fn list_level(e: &BytesStart<'_>, ctx: &Ctx<'_>, lists: &Lists) -> Option<u32> {
    let named =
        attr_in(e, b"text", b"style-name").and_then(|name| ctx.styles.lists.get(&name).copied());
    named.or_else(|| lists.iter().rev().flatten().next().copied())
}

/// A frame standing where a block belongs, which the model has no room for
/// except as a paragraph holding it.
pub(crate) fn framed(drawing: wp_model::doc::Drawing) -> Block {
    Block::Paragraph(Paragraph {
        content: vec![Inline::Run(Run {
            content: vec![Piece::Drawing(Box::new(drawing))],
            ..Run::default()
        })],
        ..Paragraph::default()
    })
}

fn blocks_into(
    reader: &mut Reader<&[u8]>,
    end: &[u8],
    ctx: &mut Ctx<'_>,
    out: &mut Vec<Block>,
    lists: &mut Lists,
) {
    loop {
        // Where the event about to be read starts, so that a frame can be kept
        // as the bytes it was written as.
        let at = reader.buffer_position() as usize;
        let Ok(event) = reader.read_event() else {
            return;
        };
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(&e).to_vec();
                match name.as_slice() {
                    b"p" | b"h" => {
                        let paragraph = paragraph(reader, &e, &name, empty, ctx, lists);
                        out.push(Block::Paragraph(paragraph));
                    }
                    b"list" if !empty => {
                        lists.push(list_level(&e, ctx, lists));
                        blocks_into(reader, b"list", ctx, out, lists);
                        lists.pop();
                    }
                    b"list-item" | b"list-header" if !empty => {
                        blocks_into(reader, &name, ctx, out, lists)
                    }
                    b"table" if !empty => {
                        if let Some(table) = crate::table::read(reader, &e, ctx) {
                            out.push(Block::Table(table));
                        }
                    }
                    // A section, an index and a table of contents are all
                    // wrappers around blocks. Their own structure is not
                    // modelled, and flattening them keeps the text in the
                    // document rather than losing it to an element nobody read.
                    b"section"
                    | b"index-body"
                    | b"table-of-content"
                    | b"illustration-index"
                    | b"table-index"
                    | b"object-index"
                    | b"user-index"
                    | b"alphabetical-index"
                    | b"bibliography"
                        if !empty =>
                    {
                        blocks_into(reader, &name, ctx, out, lists)
                    }
                    b"index-title" if !empty => blocks_into(reader, &name, ctx, out, lists),
                    b"frame" if !empty => {
                        // A frame anchored to the page or to a paragraph that
                        // is not there yet still has to be drawn, so it becomes
                        // a paragraph of its own.
                        if let Some(drawing) = crate::draw::frame(reader, &e, ctx, at) {
                            out.push(framed(drawing));
                        }
                    }
                    _ if !empty => skip_element(reader, &name),
                    _ => {}
                }
            }
            Event::End(e) if end_local_name(&e) == end => return,
            Event::Eof => return,
            _ => {}
        }
    }
}

fn paragraph(
    reader: &mut Reader<&[u8]>,
    e: &BytesStart<'_>,
    name: &[u8],
    empty: bool,
    ctx: &mut Ctx<'_>,
    lists: &Lists,
) -> Paragraph {
    let style_name = attr_in(e, b"text", b"style-name");
    let style = style_name
        .as_deref()
        .map(|name| ctx.styles.id(&mut ctx.table, name, StyleKind::Paragraph));
    let mut props = ParaProps {
        style,
        ..ParaProps::default()
    };

    // A heading is a paragraph that says how deep it is. The model keeps the
    // depth as an outline level counting from zero, as its own format does.
    if name == b"h" {
        let level = attr_in(e, b"text", b"outline-level")
            .and_then(|v| v.trim().parse::<u8>().ok())
            .unwrap_or(1)
            .clamp(1, 9);
        props.outline_level = Some(level - 1);
    }

    // **A paragraph is numbered by the list it is inside, and by nothing else.**
    // Two other things look as though they number one and do not. A paragraph
    // style may name a list style, and Word's ODF export names one on the
    // automatic style of every heading in a document whose headings carry no
    // numbers at all; a document may define an outline style, and the same
    // export writes one whose every level states a number format for the same
    // headings. Trusting either puts a counter and a 0.3in hanging indent in
    // front of every heading, which is neither what the file means nor what the
    // reference draws — it draws no number at all. So the enclosing
    // `<text:list>` is the whole of it, and its depth is the level.
    if let Some(num_id) = lists.iter().rev().flatten().next().copied() {
        props.numbering = Some(NumRef {
            num_id,
            level: (lists.len() as u8).saturating_sub(1).min(8),
        });
    }

    // ODF spells a section break as a property of the paragraph after it: the
    // paragraph's style names the master page it starts on. Only the break is
    // taken here — the page setup itself is the master page's, and a document
    // whose sections differ needs the paragraph to carry a whole `SectionProps`,
    // which is the next thing this reader will need to learn.
    if style_name
        .as_deref()
        .and_then(|name| ctx.styles.master_of_style.get(name))
        .is_some_and(|master| !master.is_empty())
    {
        props.page_break_before = Some(true);
    }

    let mut paragraph = Paragraph {
        props,
        ..Paragraph::default()
    };
    if !empty {
        paragraph.content = inlines(reader, name, ctx, &RunProps::default());
    }
    paragraph
}

/// Everything inside a paragraph, a span or a link.
fn inlines(
    reader: &mut Reader<&[u8]>,
    end: &[u8],
    ctx: &mut Ctx<'_>,
    inherited: &RunProps,
) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    let mut pieces: Vec<Piece> = Vec::new();
    let mut text = String::new();
    let mut at;

    macro_rules! flush_text {
        () => {
            if !text.is_empty() {
                pieces.push(Piece::Text(std::mem::take(&mut text).into()));
            }
        };
    }
    macro_rules! flush_run {
        () => {{
            flush_text!();
            if !pieces.is_empty() {
                out.push(Inline::Run(Run {
                    props: inherited.clone(),
                    content: std::mem::take(&mut pieces),
                    prop_change: None,
                }));
            }
        }};
    }

    loop {
        at = reader.buffer_position() as usize;
        let Ok(event) = reader.read_event() else {
            break;
        };
        if push_text(&mut text, &event) {
            continue;
        }
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(&e).to_vec();
                match name.as_slice() {
                    // `<text:s/>` is one space; `text:c` says how many. ODF
                    // collapses runs of whitespace in text, so every space
                    // after the first is written this way and a reader that
                    // skipped it would join the words either side of it.
                    b"s" => {
                        let count = attr_in(&e, b"text", b"c").and_then(|v| v.trim().parse().ok());
                        for _ in 0..count.unwrap_or(1u32) {
                            text.push(' ');
                        }
                        if !empty {
                            skip_element(reader, &name);
                        }
                    }
                    b"tab" => {
                        flush_text!();
                        pieces.push(Piece::Tab);
                        if !empty {
                            skip_element(reader, &name);
                        }
                    }
                    b"line-break" => {
                        flush_text!();
                        pieces.push(Piece::Break(Break::Line));
                        if !empty {
                            skip_element(reader, &name);
                        }
                    }
                    // LibreOffice's note to itself about where its own layout
                    // broke the page. It is not content, and reading it as a
                    // break would paginate the document twice over.
                    b"soft-page-break" => {
                        if !empty {
                            skip_element(reader, &name);
                        }
                    }
                    b"span" => {
                        flush_run!();
                        let props = span_props(&e, ctx, inherited);
                        if !empty {
                            out.extend(inlines(reader, &name, ctx, &props));
                        }
                    }
                    b"a" => {
                        flush_run!();
                        let href = attr_in(&e, b"xlink", b"href").unwrap_or_default();
                        let content = match empty {
                            true => Vec::new(),
                            false => inlines(reader, &name, ctx, inherited),
                        };
                        out.push(Inline::Hyperlink(Box::new(link(&href, content))));
                    }
                    b"bookmark" | b"bookmark-start" => {
                        flush_run!();
                        if let Some(name) = attr_in(&e, b"text", b"name") {
                            let id = ctx.bookmarks.len() as u32;
                            ctx.bookmarks.insert(name.clone(), id);
                            out.push(Inline::Anchor(Anchor::BookmarkStart {
                                id,
                                name: name.into(),
                            }));
                            if local_name(&e) == b"bookmark" {
                                out.push(Inline::Anchor(Anchor::BookmarkEnd { id }));
                            }
                        }
                        if !empty {
                            skip_element(reader, &name);
                        }
                    }
                    b"bookmark-end" => {
                        flush_run!();
                        if let Some(id) = attr_in(&e, b"text", b"name")
                            .and_then(|name| ctx.bookmarks.get(&name).copied())
                        {
                            out.push(Inline::Anchor(Anchor::BookmarkEnd { id }));
                        }
                        if !empty {
                            skip_element(reader, &name);
                        }
                    }
                    b"note" if !empty => {
                        flush_text!();
                        if let Some(piece) = note(reader, &e, ctx) {
                            pieces.push(piece);
                        }
                    }
                    b"frame" if !empty => {
                        flush_text!();
                        if let Some(drawing) = crate::draw::frame(reader, &e, ctx, at) {
                            pieces.push(Piece::Drawing(Box::new(drawing)));
                        }
                    }
                    // A field whose value the page decides, and the one place
                    // this reader builds something the file does not spell out.
                    //
                    // ODF states a page number as one element with the text it
                    // last showed inside it. WordprocessingML states the same
                    // thing as a *run* of marks — begin, instruction, separate,
                    // the cached result, end — and that is the shape the layout
                    // evaluates, because a page number cannot be known until the
                    // page exists and the cached result is what is drawn until
                    // it is. Written any other way, a header reads out the page
                    // number the file was last saved with: `6 do 0` where the
                    // page says `1 of 5`.
                    //
                    // Everything else that is a field arrives with the text it
                    // last showed, and that text is what a rendering has, so it
                    // is read as text rather than as a thing to recompute.
                    b"page-number" | b"page-count" if !empty => {
                        flush_run!();
                        let instruction = match name.as_slice() {
                            b"page-number" => " PAGE ",
                            _ => " NUMPAGES ",
                        };
                        out.push(Inline::Run(Run {
                            props: inherited.clone(),
                            content: vec![
                                Piece::FieldStart {
                                    dirty: false,
                                    lock: false,
                                },
                                Piece::Instruction(instruction.into()),
                                Piece::FieldSeparate,
                            ],
                            prop_change: None,
                        }));
                        out.extend(inlines(reader, &name, ctx, inherited));
                        out.push(Inline::Run(Run {
                            props: inherited.clone(),
                            content: vec![Piece::FieldEnd],
                            prop_change: None,
                        }));
                    }
                    // **A drawing's words are not the paragraph's words.** A
                    // shape carries its label as paragraphs of its own, and the
                    // rule below would harvest them into the line the shape is
                    // anchored in — which put a watermark's word at the left
                    // margin of a header, in the header's own face and size,
                    // where the reference draws it as outlines across the whole
                    // page and no reader would call it a word at all.
                    _ if crate::xml::prefix(e.name().into_inner()) == b"draw" && !empty => {
                        skip_element(reader, &name)
                    }
                    _ if !empty => {
                        // Everything else that holds text holds it as text:
                        // a date, a title, a cross-reference, a sequence
                        // number. Reading the characters keeps the page right
                        // and loses only the fact that they were computed.
                        let inner = inlines(reader, &name, ctx, inherited);
                        if !inner.is_empty() {
                            flush_run!();
                            out.extend(inner);
                        }
                    }
                    _ => {}
                }
            }
            Event::End(e) if end_local_name(&e) == end => break,
            Event::Eof => break,
            _ => {}
        }
    }
    flush_run!();
    out
}

fn span_props(e: &BytesStart<'_>, ctx: &mut Ctx<'_>, inherited: &RunProps) -> RunProps {
    let mut props = inherited.clone();
    if let Some(name) = attr_in(e, b"text", b"style-name") {
        props.style = Some(ctx.styles.id(&mut ctx.table, &name, StyleKind::Character));
    }
    props
}

/// `<text:a>` — a link to a place, spelled as a URL either way.
///
/// A link inside the document is a `#`-prefixed fragment and becomes an anchor;
/// anything else is external and becomes a relationship the package does not
/// have, which is why the target is carried as the anchor of nothing. A `.docx`
/// keeps an external link in the relationship part, and this document has no
/// such part — so the name is minted and the writer will have to author one.
fn link(href: &str, content: Vec<Inline>) -> Hyperlink {
    match href.strip_prefix('#') {
        Some(anchor) => Hyperlink {
            rel: None,
            anchor: Some(anchor.into()),
            tooltip: None,
            history: true,
            content,
        },
        None => Hyperlink {
            rel: Some(href.into()),
            anchor: None,
            tooltip: None,
            history: true,
            content,
        },
    }
}

/// `<text:note>` — a footnote or an endnote, body and all.
fn note(reader: &mut Reader<&[u8]>, e: &BytesStart<'_>, ctx: &mut Ctx<'_>) -> Option<Piece> {
    let endnote = attr_in(e, b"text", b"note-class").as_deref() == Some("endnote");
    let mut content = Vec::new();
    while let Ok(event) = reader.read_event() {
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(&e).to_vec();
                match name.as_slice() {
                    b"note-body" if !empty => content = blocks(reader, &name, ctx),
                    _ if !empty => skip_element(reader, &name),
                    _ => {}
                }
            }
            Event::End(e) if end_local_name(&e) == b"note" => break,
            Event::Eof => break,
            _ => {}
        }
    }
    let notes = match endnote {
        true => &mut ctx.endnotes,
        false => &mut ctx.footnotes,
    };
    // The model numbers notes from one and keeps the separators below zero, as
    // the other format writes them; ODF has neither, so the ids are ours.
    let id = notes.len() as i32 + 1;
    notes.push(Note {
        id,
        kind: NoteKind::Normal,
        content,
    });
    Some(match endnote {
        true => Piece::EndnoteRef {
            id,
            custom_mark: false,
        },
        false => Piece::FootnoteRef {
            id,
            custom_mark: false,
        },
    })
}

/// One block, read out of exactly the bytes it occupies in the part.
///
/// **This is the writer's definition of "changed".** A `<text:p>` is compared
/// against the model by reading it again and asking whether the result differs,
/// rather than by remembering what some earlier reading believed — which is the
/// only definition that cannot drift away from the reader. `lists` is the stack
/// of `<text:list>` elements the block stands inside, because a paragraph is
/// numbered by that and by nothing else and the bytes alone do not say.
pub(crate) fn block_of<'a>(bytes: &'a [u8], ctx: &mut Ctx<'a>, lists: &Lists) -> Option<Block> {
    let text = std::str::from_utf8(bytes).ok()?;
    ctx.source = bytes;
    let mut reader = Reader::from_str(text);
    loop {
        let at = reader.buffer_position() as usize;
        let event = reader.read_event().ok()?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = local_name(&e).to_vec();
                return match name.as_slice() {
                    b"p" | b"h" => Some(Block::Paragraph(paragraph(
                        &mut reader,
                        &e,
                        &name,
                        empty,
                        ctx,
                        lists,
                    ))),
                    b"table" if !empty => {
                        crate::table::read(&mut reader, &e, ctx).map(Block::Table)
                    }
                    b"frame" if !empty => crate::draw::frame(&mut reader, &e, ctx, at).map(framed),
                    _ => None,
                };
            }
            Event::Eof => return None,
            _ => {}
        }
    }
}

/// The text of an element and everything under it, for the places that want
/// characters rather than structure.
pub(crate) fn text_of(reader: &mut Reader<&[u8]>, end: &[u8]) -> String {
    let mut out = String::new();
    let mut depth = 1usize;
    while let Ok(event) = reader.read_event() {
        if push_text(&mut out, &event) {
            continue;
        }
        match event {
            Event::Start(e) if local_name(&e) == end => depth += 1,
            Event::End(e) if end_local_name(&e) == end => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

/// A rel name minted for something the package holds at `href`, and the bytes
/// carried out beside the document.
pub(crate) fn adopt(ctx: &mut Ctx<'_>, href: &str) -> Option<Arc<str>> {
    if let Some(rel) = ctx.minted.get(href) {
        return Some(rel.clone());
    }
    let data = ctx.container.data(href)?.to_vec();
    let rel: Arc<str> = format!("odf-picture-{}", ctx.media.len() + 1).into();
    ctx.media.push(crate::Media {
        rel: rel.to_string(),
        data,
        content_type: content_type(href),
    });
    ctx.minted.insert(href.to_string(), rel.clone());
    Some(rel)
}

fn content_type(href: &str) -> &'static str {
    match href
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        Some("tif") | Some("tiff") => "image/tiff",
        Some("emf") => "image/x-emf",
        Some("wmf") => "image/x-wmf",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}
