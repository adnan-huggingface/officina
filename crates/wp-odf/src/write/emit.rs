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
use wp_model::prop::ParaProps;
use wp_model::revision::Anchor;
use wp_model::table::{Table, VMerge};
use wp_model::units::Twips;

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

/// `<text:p>`, or `<text:h>` where the paragraph says how deep it is — and
/// more than one of them where a page break stands inside it.
///
/// **ODF has no page break inside a paragraph**, only one before a paragraph
/// (`fo:break-before`) or after it (`fo:break-after`), each a property of the
/// paragraph's style. This writer passed over a `Piece::Break(Page)` as such a
/// property and never set it, so every page break in a document saved as
/// `.odt` was gone and its pages ran together. A paragraph is now cut at each
/// break, every piece after the first starting a page. A break at the very
/// end — the shape Ctrl+Enter makes — is the paragraph's own `break-after`,
/// which the reader gives back as the break it was; one at the very start is
/// its `break-before`.
pub(crate) fn paragraph(out: &mut String, paragraph: &Paragraph, w: &mut Out<'_>) {
    let mut parts = split_at_pages(&paragraph.content);
    if parts.len() == 1 {
        one(out, &paragraph.props, &paragraph.content, false, w);
        return;
    }
    let trailing = parts.last().is_some_and(Vec::is_empty);
    if trailing {
        parts.pop();
    }
    let leading = parts.len() > 1 && parts.first().is_some_and(Vec::is_empty);
    if leading {
        parts.remove(0);
    }
    let last = parts.len() - 1;
    for (at, content) in parts.iter().enumerate() {
        let mut props = paragraph.props.clone();
        if at > 0 || leading {
            props.page_break_before = Some(true);
        }
        one(out, &props, content, trailing && at == last, w);
    }
}

/// A paragraph's inlines, cut at every page or column break among its runs.
/// The breaks themselves go; what is either side of each is a part.
fn split_at_pages(content: &[Inline]) -> Vec<Vec<Inline>> {
    let breaks = |piece: &Piece| matches!(piece, Piece::Break(Break::Page | Break::Column));
    let mut parts: Vec<Vec<Inline>> = vec![Vec::new()];
    for inline in content {
        let Inline::Run(run) = inline else {
            parts.last_mut().expect("never empty").push(inline.clone());
            continue;
        };
        if !run.content.iter().any(breaks) {
            parts.last_mut().expect("never empty").push(inline.clone());
            continue;
        }
        let mut piece_run = Run {
            content: Vec::new(),
            ..run.clone()
        };
        for piece in &run.content {
            if breaks(piece) {
                if !piece_run.content.is_empty() {
                    let done = std::mem::take(&mut piece_run.content);
                    parts
                        .last_mut()
                        .expect("never empty")
                        .push(Inline::Run(Run {
                            content: done,
                            ..run.clone()
                        }));
                }
                parts.push(Vec::new());
            } else {
                piece_run.content.push(piece.clone());
            }
        }
        if !piece_run.content.is_empty() {
            parts
                .last_mut()
                .expect("never empty")
                .push(Inline::Run(piece_run));
        }
    }
    parts
}

/// One `<text:p>` or `<text:h>`.
fn one(
    out: &mut String,
    props: &ParaProps,
    content: &[Inline],
    break_after: bool,
    w: &mut Out<'_>,
) {
    let tag = match props.outline_level {
        Some(_) => "text:h",
        None => "text:p",
    };
    let _ = write!(out, "<{tag}");
    if let Some(style) = w.auto.paragraph_style(props, break_after) {
        let _ = write!(out, r#" text:style-name="{}""#, escape_attr(&style));
    }
    if let Some(level) = props.outline_level {
        // The model counts outline levels from zero as its usual format does;
        // ODF counts a heading's depth from one.
        let _ = write!(out, r#" text:outline-level="{}""#, level as u32 + 1);
    }
    if content.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    // A space here would be the first thing on the line, and ODF drops one that
    // is, so the paragraph starts out owing `<text:s/>` for it.
    let mut fresh = true;
    inlines(out, content, w, &mut fresh);
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
        // not a mark inside one: `paragraph` has already cut the paragraph at
        // every one and given the pieces the styles that say so.
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
    // Only bytes an ODF reader kept: a `<w:drawing>` put back here would be
    // markup no part of this package declares, and a file LibreOffice refuses.
    if let Some(source) = drawing.source_in(wp_model::SourceFormat::Odf) {
        if let Ok(source) = std::str::from_utf8(source) {
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
/// through instead — see `write::mod` — so that everything this crate does not
/// model about it survives. This is for a table authored here, or crossing in
/// from the other format, and for the rows a spliced table gained.
///
/// **An ODF table says what it looks like in automatic styles, or not at all.**
/// Written as bare elements, a table authored here lost its column widths and
/// every rule on the way into the file, and came back — in LibreOffice and
/// here — as a grid of unruled text. So the table, each width of column, each
/// row that states a height and each look of cell mints a style, the way a run
/// with direct formatting mints one; a fully ruled table is one cell style.
pub(crate) fn table(out: &mut String, table: &Table, w: &mut Out<'_>) {
    let columns = columns_of(table);
    out.push_str("<table:table");
    if let Some(name) = &table.props.caption {
        let _ = write!(out, r#" table:name="{}""#, escape_attr(name));
    }
    let style = w.auto.table_style("table", &table_body(table));
    let _ = write!(out, r#" table:style-name="{}">"#, escape_attr(&style));
    // One element per run of equal widths, which is how a producer writes it
    // and how the reader counts it back out.
    let mut at = 0;
    while at < columns {
        let width = table.grid.get(at).copied().unwrap_or(Twips(0));
        let mut run = 1;
        while at + run < columns && table.grid.get(at + run).copied().unwrap_or(Twips(0)) == width {
            run += 1;
        }
        out.push_str("<table:table-column");
        // A width of nothing is a width nobody stated, and stating none is
        // how the reader is told so.
        if width.0 > 0 {
            let body = format!(
                r#"<style:table-column-properties style:column-width="{}"/>"#,
                super::auto::twips(width)
            );
            let name = w.auto.table_style("table-column", &body);
            let _ = write!(out, r#" table:style-name="{}""#, escape_attr(&name));
        }
        if run > 1 {
            let _ = write!(out, r#" table:number-columns-repeated="{run}""#);
        }
        out.push_str("/>");
        at += run;
    }
    for index in 0..table.rows.len() {
        row(out, table, index, w);
    }
    out.push_str("</table:table>");
}

/// How many grid columns a table has: its grid, or its widest row.
fn columns_of(table: &Table) -> usize {
    table.grid.len().max(
        table
            .rows
            .iter()
            .map(|row| {
                row.props.grid_before as usize
                    + row
                        .cells
                        .iter()
                        .map(|c| c.props.span() as usize)
                        .sum::<usize>()
            })
            .max()
            .unwrap_or(1),
    )
}

/// `<style:table-properties>`: the table's width and where it stands.
fn table_body(table: &Table) -> String {
    use wp_model::table::Width;
    let mut attrs = String::new();
    let measured: i32 = table.grid.iter().map(|width| width.0).sum();
    match table.props.width {
        Width::Fixed(width) if width.0 > 0 => {
            let _ = write!(attrs, r#" style:width="{}""#, super::auto::twips(width));
        }
        Width::Percent(share) => {
            if measured > 0 {
                let _ = write!(
                    attrs,
                    r#" style:width="{}""#,
                    super::auto::twips(Twips(measured))
                );
            }
            let _ = write!(attrs, r#" style:rel-width="{}%""#, trim(share.percent()));
        }
        _ if measured > 0 => {
            let _ = write!(
                attrs,
                r#" style:width="{}""#,
                super::auto::twips(Twips(measured))
            );
        }
        _ => {}
    }
    // Stated whatever the model says, because LibreOffice reads a table with
    // no alignment as one stretched from margin to margin.
    let align = match table.props.justify {
        Some(wp_model::prop::Justify::Center) => "center",
        Some(wp_model::prop::Justify::End) => "right",
        _ => "left",
    };
    let _ = write!(attrs, r#" table:align="{align}""#);
    if let Some(Width::Fixed(indent)) = table.props.indent {
        let _ = write!(attrs, r#" fo:margin-left="{}""#, super::auto::twips(indent));
    }
    if let Some(fill) = table.props.shading.and_then(|shading| shading.fill) {
        let _ = write!(
            attrs,
            r#" fo:background-color="{}""#,
            super::auto::hex(fill)
        );
    }
    format!("<style:table-properties{attrs}/>")
}

/// One row of a table, as it goes into a `.odt` for the first time.
pub(crate) fn row(out: &mut String, table: &Table, index: usize, w: &mut Out<'_>) {
    use wp_model::table::RowHeight;
    let Some(this) = table.rows.get(index) else {
        return;
    };
    let columns = columns_of(table);
    let last_row = index + 1 == table.rows.len();
    out.push_str("<table:table-row");
    let mut attrs = String::new();
    match this.props.height {
        Some(RowHeight::AtLeast(height)) => {
            let _ = write!(
                attrs,
                r#" style:min-row-height="{}""#,
                super::auto::twips(height)
            );
        }
        Some(RowHeight::Exact(height)) => {
            let _ = write!(
                attrs,
                r#" style:row-height="{}""#,
                super::auto::twips(height)
            );
        }
        _ => {}
    }
    if this.props.cant_split {
        attrs.push_str(r#" fo:keep-together="always""#);
    }
    if !attrs.is_empty() {
        let body = format!("<style:table-row-properties{attrs}/>");
        let name = w.auto.table_style("table-row", &body);
        let _ = write!(out, r#" table:style-name="{}""#, escape_attr(&name));
    }
    out.push('>');
    let mut column = this.props.grid_before as usize;
    for cell in &this.cells {
        let span = cell.props.span() as usize;
        if cell.props.v_merge == Some(VMerge::Continue) {
            out.push_str("<table:covered-table-cell/>");
            column += span;
            continue;
        }
        let edges = Edges {
            first_row: index == 0,
            last_row,
            first_column: column == 0,
            last_column: column + span >= columns,
        };
        out.push_str("<table:table-cell");
        let body = cell_body(table, cell, edges);
        if !body.is_empty() {
            let name = w.auto.table_style("table-cell", &body);
            let _ = write!(out, r#" table:style-name="{}""#, escape_attr(&name));
        }
        if span > 1 {
            let _ = write!(out, r#" table:number-columns-spanned="{span}""#);
        }
        // A vertical merge is a count here, of the rows below that continue it.
        if cell.props.v_merge == Some(VMerge::Restart) {
            let below = table.rows[index + 1..]
                .iter()
                .take_while(|row| continues_at(row, column))
                .count();
            if below > 0 {
                let _ = write!(out, r#" table:number-rows-spanned="{}""#, below + 1);
            }
        }
        out.push('>');
        for block in &cell.content {
            self::block(out, block, w);
        }
        out.push_str("</table:table-cell>");
        // Every position a span covers is spelled out, or the row is short.
        for _ in 1..span {
            out.push_str("<table:covered-table-cell/>");
        }
        column += span;
    }
    out.push_str("</table:table-row>");
}

/// Whether the cell of `row` standing at grid column `column` continues a
/// vertical merge from above.
fn continues_at(row: &wp_model::table::Row, column: usize) -> bool {
    let mut at = row.props.grid_before as usize;
    for cell in &row.cells {
        if at == column {
            return cell.props.v_merge == Some(VMerge::Continue);
        }
        at += cell.props.span() as usize;
        if at > column {
            return false;
        }
    }
    false
}

/// Which of the table's outer edges a cell stands on.
#[derive(Clone, Copy)]
struct Edges {
    first_row: bool,
    last_row: bool,
    first_column: bool,
    last_column: bool,
}

/// `<style:table-cell-properties>`, or nothing where the cell has no look.
///
/// ODF has no table-wide rules, only a cell's own edges, so the table's are
/// handed out to its cells: the outer ones to the cells on the outside and the
/// inside ones to the rest — and a cell's own edge, where it states one, wins.
fn cell_body(table: &Table, cell: &wp_model::table::Cell, at: Edges) -> String {
    use wp_model::table::Width;
    let rules = &table.props.borders;
    let own = &cell.props.borders;
    let top = if at.first_row {
        own.top.or(rules.top)
    } else {
        own.top.or(rules.inside_h)
    };
    let bottom = if at.last_row {
        own.bottom.or(rules.bottom)
    } else {
        own.bottom.or(rules.inside_h)
    };
    let left = if at.first_column {
        own.start.or(rules.start)
    } else {
        own.start.or(rules.inside_v)
    };
    let right = if at.last_column {
        own.end.or(rules.end)
    } else {
        own.end.or(rules.inside_v)
    };
    let mut attrs = String::new();
    for (side, border) in [
        ("top", top),
        ("left", left),
        ("bottom", bottom),
        ("right", right),
    ] {
        if let Some(border) = border {
            let _ = write!(
                attrs,
                r#" fo:border-{side}="{}""#,
                super::auto::border_words(&border)
            );
        }
    }
    let fallback = &table.props.cell_margins;
    for (side, own, table) in [
        ("top", cell.props.margins.top, fallback.top),
        ("left", cell.props.margins.start, fallback.start),
        ("bottom", cell.props.margins.bottom, fallback.bottom),
        ("right", cell.props.margins.end, fallback.end),
    ] {
        if let Some(Width::Fixed(pad)) = own.or(table) {
            let _ = write!(attrs, r#" fo:padding-{side}="{}""#, super::auto::twips(pad));
        }
    }
    if let Some(fill) = cell.props.shading.and_then(|shading| shading.fill) {
        let _ = write!(
            attrs,
            r#" fo:background-color="{}""#,
            super::auto::hex(fill)
        );
    }
    match cell.props.v_align {
        wp_model::table::CellVAlign::Center => attrs.push_str(r#" style:vertical-align="middle""#),
        wp_model::table::CellVAlign::Bottom => attrs.push_str(r#" style:vertical-align="bottom""#),
        _ => {}
    }
    match attrs.is_empty() {
        true => String::new(),
        false => format!("<style:table-cell-properties{attrs}/>"),
    }
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
