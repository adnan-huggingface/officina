//! Blocks, tables and pages: turning a document into a stack of paper.
//!
//! The pipeline is two passes, and keeping them apart is what makes the keep
//! rules possible at all:
//!
//! 1. **Flow.** Every block becomes a list of [`Item`]s — one per line of a
//!    paragraph, one per row of a table — each with its height and with what it
//!    must not be separated from.
//! 2. **Paginate.** Items are placed onto pages until the column is full, and
//!    the break is then *pulled back* to honour keep-with-next, keep-lines and
//!    widow control.
//!
//! Doing it in one pass is possible right up to the moment a paragraph says
//! "keep with next": by then the decision has already been made and the only way
//! back is to unplace what was placed.
//!
//! **Stated limits**, rather than hidden ones. Text does not wrap *around* an
//! anchored drawing — the drawing is placed where the document says and the text
//! flows past it, which is right for `wrapNone` and `wrapTopAndBottom` and wrong
//! for `wrapSquare`. Multi-column sections lay out column by column rather than
//! balancing the last page. A table row splits across pages between the lines
//! of its cells, but only at a height that is a line boundary in *every* cell
//! at once — where two columns of text line up on nothing, the row moves whole,
//! because a row drawn in two pieces that disagree about where they were cut is
//! worse than a row that moved. Each is a body of work of its own and each is
//! visible rather than silent.

use std::sync::Arc;

use wp_model::doc::{Block, Break, Document, Paragraph, Piece};
use wp_model::numbering::Counters;
use wp_model::prop::Border;
use wp_model::section::{HeaderKind, PageBox, SectionProps};
use wp_model::style::Layers;
use wp_model::table::{CellVAlign, Table, VMerge, Width};
use wp_model::units::Twips;

use crate::inline::{self, Context, LaidParagraph, Line, ListLabel};
use crate::shape::Shaper;

/// Something drawn at a place on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub kind: Placed,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Placed {
    /// One laid-out line, and which paragraph it belongs to.
    ///
    /// The paragraph is its index in [`wp_model::Document::paragraphs`] — the
    /// same document-order walk an editor names positions by. Without it a click
    /// on a line resolves to an offset in *some* paragraph and no way to say
    /// which, which is the whole of placing a caret.
    Line { line: Box<Line>, paragraph: usize },
    /// A filled rectangle: cell or paragraph shading.
    Fill([u8; 3]),
    /// One edge of a border.
    Edge { border: Border, side: Side },
    /// One edge of a border that is only real if a page break cuts the row
    /// at this end of the band.
    ///
    /// A row is flowed in bands before anybody knows where the pages fall.
    /// Word closes a row cut by a page with the cell's own border — a bottom
    /// rule on the fragment above the cut and a top rule on the one below —
    /// but drawing those on every band would rule lines across the middle of
    /// whole cells. So the maybe-edges travel with the band, and pagination,
    /// which is what knows where the cut landed, turns the ones at a real cut
    /// into [`Placed::Edge`] and drops the rest.
    BreakEdge { border: Border, side: Side },
    /// A drawing, by the relationship naming the part that holds its bytes.
    ///
    /// `anchor` is the drawing itself, for a renderer that has to work out where
    /// an anchored one sits — see [`anchor_position`].
    Drawing {
        rel: Option<Arc<str>>,
        anchor: Option<Box<wp_model::Drawing>>,
        /// Which paragraph holds it, and which of that paragraph's drawings it
        /// is — the pair that turns a click on a picture into an edit.
        paragraph: usize,
        nth: usize,
    },
    /// The rule Word draws above the footnote area.
    FootnoteSeparator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Top,
    Start,
    Bottom,
    End,
}

/// One page of the document.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    /// One-based, and restarted by `<w:pgNumType w:start>` — so it is what a
    /// PAGE field shows rather than the index in the vector.
    pub number: u32,
    pub section: usize,
    pub geometry: PageBox,
    pub content: Vec<Placement>,
    pub header: Vec<Placement>,
    pub footer: Vec<Placement>,
    pub footnotes: Vec<Placement>,
}

impl Page {
    /// Everything on the page, for a renderer that does not care which band a
    /// placement came from.
    pub fn everything(&self) -> impl Iterator<Item = &Placement> {
        self.content
            .iter()
            .chain(&self.header)
            .chain(&self.footer)
            .chain(&self.footnotes)
    }
}

/// One breakable unit in the flow.
#[derive(Debug, Clone)]
pub struct Item {
    pub height: f64,
    /// Placements, relative to the top-left of the item.
    pub parts: Vec<Placement>,
    /// The block this came from. Widow control and keep-lines are about the
    /// items of one group.
    pub group: usize,
    pub index_in_group: usize,
    pub items_in_group: usize,
    /// This group must not be separated from the next.
    pub keep_with_next: bool,
    /// This group's items must not be split across pages.
    pub keep_lines: bool,
    /// At least two lines of this group must sit on each side of a break.
    pub widow_control: bool,
    /// Start a new page before this item.
    pub break_before: bool,
    /// A table header row: repeated at the top of every page the table covers.
    pub repeat: bool,
    /// Which table this came from, if it came from one.
    ///
    /// Separate from `group` because the two answer different questions. A
    /// group is what a keep rule holds together — one row, since Word splits a
    /// table between its rows — while a repeated header row has to be found
    /// again from a *later* row of the same table, which is a different group.
    pub table: Option<usize>,
    /// Footnotes referenced by this item, and how tall they are.
    pub footnotes: Vec<(i32, f64)>,
}

/// A document, flowed into items and not yet paginated.
#[derive(Debug, Clone, Default)]
pub struct Flow {
    pub items: Vec<Item>,
    /// How many paragraphs have been flowed. Counts in the same order as
    /// [`wp_model::Document::paragraphs`], so it *is* the next paragraph's index.
    pub paragraphs: usize,
}

/// Everything the block layout needs beyond the document.
pub struct Frame<'a, 'b> {
    pub document: &'a Document,
    pub inline: &'a Context<'b>,
}

/// Lays a whole document out into pages, twice.
///
/// **A page number cannot be known before the page exists**, so the first pass
/// draws whatever the file cached, the page each field landed on is read off the
/// result, and the second pass draws the real numbers. Word does the same thing;
/// it stops after a bounded number of passes too, because a document where the
/// page number changes the pagination that changes the page number has no fixed
/// point to reach.
///
/// A document with no page fields in it pays for one pass, not two.
pub fn layout(document: &Document, ctx: &Context<'_>, shaper: &mut dyn Shaper) -> Vec<Page> {
    let first = layout_once(document, ctx, shaper);
    let values = evaluate(&first, ctx.fields);
    if values.is_empty() {
        return first;
    }
    // Nothing a field says has changed, so the second pass would produce the
    // same pages as the first. This is the ordinary case once a document has
    // settled: a `{ PAGE }` in a footer would otherwise double the cost of
    // laying out on *every* keystroke, for a number that is already right.
    if values.same_as(ctx.fields) {
        return first;
    }
    let second = Context {
        fields: &values,
        ..*ctx
    };
    layout_once(document, &second, shaper)
}

/// Reads the page each field landed on off a laid-out document.
///
/// Public so that a caller holding pages from a previous layout can start the
/// next one from the values it already arrived at, rather than from nothing.
pub fn evaluate(pages: &[Page], known: &crate::field::FieldValues) -> crate::field::FieldValues {
    use wp_model::field::Kind;
    let mut values = crate::field::FieldValues::carrying(known);
    let total = pages.len();
    for page in pages {
        for placement in page.everything() {
            let Placed::Line { line, .. } = &placement.kind else {
                continue;
            };
            for fragment in &line.fragments {
                let Some(mark) = fragment.field else {
                    continue;
                };
                match mark.kind {
                    Kind::Page => values.set(mark, page.number.to_string()),
                    Kind::NumPages => values.set(mark, total.to_string()),
                    // A section's own page count: how many pages carry its
                    // number, which is what `{ SECTIONPAGES }` means and is not
                    // the same as the document's total.
                    Kind::SectionPages => {
                        let in_section = pages
                            .iter()
                            .filter(|other| other.section == page.section)
                            .count();
                        values.set(mark, in_section.to_string());
                    }
                    _ => {}
                }
            }
        }
    }
    values
}

fn layout_once(document: &Document, ctx: &Context<'_>, shaper: &mut dyn Shaper) -> Vec<Page> {
    let mut pages = Vec::new();
    let mut counters = Counters::new();
    let mut number = 1u32;

    for (section_index, (range, section)) in document.sections().into_iter().enumerate() {
        if let Some(start) = section.page_numbering.start {
            number = start;
        }
        let width = section.text_width().points();
        let (top, bottom) = band_margins(document, section, ctx, shaper);
        let height = section.page.height.points() - top - bottom;
        let columns = section.columns.resolve(section.text_width());

        let mut flow = Flow::default();
        for block in &document.body[range] {
            flow_block(
                block,
                document,
                ctx,
                shaper,
                &mut counters,
                width,
                &mut flow,
            );
        }

        let breaks = paginate(&flow.items, height);
        let mut placed = 0usize;
        for (page_index, end) in breaks.iter().enumerate() {
            let mut page = Page {
                number,
                section: section_index,
                geometry: PageBox {
                    top,
                    bottom,
                    ..PageBox::of(section)
                },
                content: Vec::new(),
                header: Vec::new(),
                footer: Vec::new(),
                footnotes: Vec::new(),
            };
            // A multi-column section runs its items down the first column and
            // then the next. Balancing the last page is a separate problem and
            // is not attempted.
            let column = columns.first().map(|c| c.width.points()).unwrap_or(width);
            let _ = column;
            let mut y = page.geometry.top;
            let slice = &flow.items[placed..*end];
            for (offset, item) in slice.iter().enumerate() {
                // A maybe-edge is real only where the page actually cut its
                // row: above the first item when the same row continues from
                // the previous page, below the last when it runs on to the
                // next. Everywhere else the row is whole and the edge is not.
                let cut_above = offset == 0
                    && placed > 0
                    && item.table.is_some()
                    && flow.items[placed - 1].table == item.table
                    && flow.items[placed - 1].group == item.group;
                let cut_below = offset + 1 == slice.len()
                    && *end < flow.items.len()
                    && item.table.is_some()
                    && flow.items[*end].table == item.table
                    && flow.items[*end].group == item.group;
                for part in &item.parts {
                    let kind = match &part.kind {
                        Placed::BreakEdge { border, side } => {
                            let cut = match side {
                                Side::Top => cut_above,
                                Side::Bottom => cut_below,
                                _ => false,
                            };
                            if !cut {
                                continue;
                            }
                            Placed::Edge {
                                border: *border,
                                side: *side,
                            }
                        }
                        other => other.clone(),
                    };
                    page.content.push(Placement {
                        x: page.geometry.start + part.x,
                        y: y + part.y,
                        kind,
                        ..part.clone()
                    });
                }
                y += item.height;
            }
            placed = *end;

            let is_first_page_of_section = page_index == 0;
            place_bands(
                &mut page,
                document,
                section,
                ctx,
                shaper,
                number,
                is_first_page_of_section,
            );
            pages.push(page);
            number += 1;
        }
    }
    pages
}

/// How far the body must actually stay from the page's edges.
///
/// The margins say where the body normally starts, but a header taller than
/// the gap between the header distance and the top margin pushes the text
/// *down* rather than being drawn over, and a tall footer pushes it up — Word
/// grows the effective margin to the band's distance plus its height. Without
/// this the body keeps the nominal margin and fits a line more per page than
/// Word does, and every page break after the first drifts.
///
/// Measured with the default bands: a section whose first or even pages carry
/// a different-sized band is approximated by its ordinary one, because the
/// body is paginated with one height per section.
fn band_margins(
    document: &Document,
    section: &SectionProps,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
) -> (f64, f64) {
    let width = section.text_width().points();
    let mut top = section.margins.top.points();
    let mut bottom = section.margins.bottom.points();
    if let Some(body) = section.header(HeaderKind::Default) {
        if let Some(header) = document.header(body) {
            let (_, height) = band(&header.content, document, ctx, shaper, width);
            top = top.max(section.margins.header.points() + height);
        }
    }
    if let Some(body) = section.footer(HeaderKind::Default) {
        if let Some(footer) = document.header(body) {
            let (_, height) = band(&footer.content, document, ctx, shaper, width);
            bottom = bottom.max(section.margins.footer.points() + height);
        }
    }
    (top, bottom)
}

/// Puts the header and the footer in the margins.
#[allow(clippy::too_many_arguments)]
fn place_bands(
    page: &mut Page,
    document: &Document,
    section: &SectionProps,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
    number: u32,
    _first_of_section: bool,
) {
    let even_and_odd = document.settings.even_and_odd_headers;
    let Some(kind) = section.header_for_page(number, even_and_odd) else {
        return;
    };
    let width = section.text_width().points();
    // The same header is laid out again for every page it appears on, so its
    // fields belong to this page rather than to the header. Without that, one
    // `{ PAGE }` in a footer would be a single question with a single answer,
    // and every page would show the number of the last one.
    let ctx = &Context {
        band: Some(number),
        ..*ctx
    };

    if let Some(body) = section.header(kind).or_else(|| {
        // A section with no header of the kind the page wants has *no* header —
        // the reference being absent is the instruction, and falling back to the
        // default one would put a header on a title page that asked for none.
        (kind == HeaderKind::Default).then_some(section.header(HeaderKind::Default)?)
    }) {
        if let Some(header) = document.header(body) {
            let y = section.margins.header.points();
            for placement in band(&header.content, document, ctx, shaper, width).0 {
                page.header.push(Placement {
                    x: section.margins.start.points() + placement.x,
                    y: y + placement.y,
                    ..placement
                });
            }
        }
    }

    if let Some(body) = section.footer(kind) {
        if let Some(footer) = document.header(body) {
            let (placements, height) = band(&footer.content, document, ctx, shaper, width);
            let top = section.page.height.points() - section.margins.footer.points() - height;
            for placement in placements {
                page.footer.push(Placement {
                    x: section.margins.start.points() + placement.x,
                    y: top + placement.y,
                    ..placement
                });
            }
        }
    }
}

/// Lays a header or footer body out as a simple stack, and says how tall it is.
///
/// The height is the stack's, not the sum of the placements': a table puts one
/// placement per cell, and adding those up makes a one-line footer measure
/// several inches tall — which is how a footer ends up floating in the middle of
/// the page instead of sitting above the bottom edge.
fn band(
    blocks: &[Block],
    document: &Document,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
    width: f64,
) -> (Vec<Placement>, f64) {
    let mut out = Vec::new();
    let mut counters = Counters::new();
    let mut flow = Flow::default();
    for block in blocks {
        flow_block(
            block,
            document,
            ctx,
            shaper,
            &mut counters,
            width,
            &mut flow,
        );
    }
    let mut y = 0.0;
    for item in flow.items {
        for part in item.parts {
            // A band is never cut by a page, so a maybe-edge never fires.
            if matches!(part.kind, Placed::BreakEdge { .. }) {
                continue;
            }
            out.push(Placement {
                y: y + part.y,
                ..part
            });
        }
        y += item.height;
    }
    (out, y)
}

/// Turns one block into items.
#[allow(clippy::too_many_arguments)]
fn flow_block(
    block: &Block,
    document: &Document,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
    counters: &mut Counters,
    width: f64,
    into: &mut Flow,
) {
    match block {
        Block::Paragraph(paragraph) => {
            flow_paragraph(paragraph, document, ctx, shaper, counters, width, 0.0, into)
        }
        Block::Table(table) => flow_table(table, document, ctx, shaper, counters, width, into),
        Block::Structured(sdt) => {
            for inner in &sdt.content {
                flow_block(inner, document, ctx, shaper, counters, width, into);
            }
        }
        Block::Anchor(_) | Block::AltChunk { .. } => {}
    }
}

/// Resolves and lays out a paragraph, then turns its lines into items.
#[allow(clippy::too_many_arguments)]
pub fn flow_paragraph(
    paragraph: &Paragraph,
    document: &Document,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
    counters: &mut Counters,
    width: f64,
    left: f64,
    into: &mut Flow,
) {
    let reference = resolved_numbering(paragraph, document);
    let numbering = reference.and_then(|r| document.numbering.layers(r));
    let layers = document
        .styles
        .resolve_paragraph(&paragraph.props, numbering.as_ref());
    let label = reference.and_then(|r| {
        let text = counters.advance(&document.numbering, r)?;
        let level = document.numbering.level(r.num_id, r.level)?;
        Some(ListLabel {
            text,
            props: level.run.clone(),
            suffix: level.suffix,
        })
    });

    let index = into.paragraphs;
    let laid = inline::layout(
        paragraph,
        index,
        &layers,
        label.as_ref(),
        ctx,
        width,
        shaper,
    );
    push_paragraph(paragraph, &layers, laid, left, into);
}

/// Which list a paragraph is in, its style's numbering included.
fn resolved_numbering(
    paragraph: &Paragraph,
    document: &Document,
) -> Option<wp_model::prop::NumRef> {
    if let Some(reference) = paragraph.props.numbering {
        return reference.is_numbered().then_some(reference);
    }
    let style = paragraph.props.style.or_else(|| {
        document
            .styles
            .default_style(wp_model::StyleKind::Paragraph)
    })?;
    document
        .styles
        .chain(style)
        .into_iter()
        .rev()
        .find_map(|step| document.styles.get(step)?.para.numbering)
        .filter(|reference| reference.is_numbered())
}

fn push_paragraph(
    paragraph: &Paragraph,
    layers: &Layers,
    laid: LaidParagraph,
    left: f64,
    into: &mut Flow,
) {
    let group = into.items.len();
    // Named apart from the line loop's own `index` below, which shadowed this
    // one and gave every line the paragraph number *zero* — so every click
    // landed in the first paragraph of the document.
    let paragraph_index = into.paragraphs;
    into.paragraphs += 1;
    let count = laid.lines.len().max(1);
    let explicit_break = paragraph
        .runs()
        .iter()
        .flat_map(|run| &run.content)
        .any(|piece| matches!(piece, Piece::Break(Break::Page)));
    let before = laid.space_before;
    let after = laid.space_after;

    // An anchored drawing is placed on the page rather than in the line, so it
    // rides with the paragraph's first item and is positioned from there.
    let floats: Vec<Placement> = anchored(paragraph)
        .into_iter()
        .map(|(nth, drawing)| Placement {
            x: 0.0,
            y: 0.0,
            width: drawing.extent.0.points(),
            height: drawing.extent.1.points(),
            kind: Placed::Drawing {
                rel: drawing.rel.clone(),
                anchor: Some(Box::new(drawing.clone())),
                paragraph: paragraph_index,
                nth,
            },
        })
        .collect();
    let mut floats = Some(floats);

    for (index, line) in laid.lines.into_iter().enumerate() {
        let is_first = index == 0;
        let is_last = index + 1 == count;
        let mut height = line.height;
        let mut top = 0.0;
        if is_first {
            height += before;
            top = before;
        }
        if is_last {
            height += after;
        }
        let ends_page = line.ended_by == Some(Break::Page);
        let x = left + line.x;
        let mut parts = floats.take().unwrap_or_default();
        parts.push(Placement {
            x,
            y: top,
            width: line.width,
            height: line.height,
            kind: Placed::Line {
                line: Box::new(line),
                paragraph: paragraph_index,
            },
        });
        into.items.push(Item {
            height,
            parts,
            group,
            index_in_group: index,
            items_in_group: count,
            keep_with_next: is_last && layers.para.keep_next.unwrap_or(false),
            keep_lines: layers.para.keep_lines.unwrap_or(false),
            // Word's default is on. A document that says nothing gets widow and
            // orphan control, which is why a single line of a paragraph almost
            // never sits alone at the top of a page in a Word document.
            widow_control: layers.para.widow_control.unwrap_or(true),
            break_before: (is_first && layers.para.page_break_before.unwrap_or(false))
                || (is_first && explicit_break && false),
            repeat: false,
            table: None,
            footnotes: Vec::new(),
        });
        if ends_page {
            if let Some(last) = into.items.last_mut() {
                last.keep_with_next = false;
            }
            // The *next* item starts a page. Marked on a sentinel so the break
            // survives being pulled back by the keep rules.
            into.items.push(Item {
                height: 0.0,
                parts: Vec::new(),
                group,
                index_in_group: index,
                items_in_group: count,
                keep_with_next: false,
                keep_lines: false,
                widow_control: false,
                break_before: true,
                repeat: false,
                table: None,
                footnotes: Vec::new(),
            });
        }
    }
    if into.items.len() == group {
        // A paragraph with no lines at all cannot happen — `inline::layout`
        // always produces one — but an empty group would make the keep rules
        // index out of range, so it is not left possible.
        into.items.push(Item {
            height: 0.0,
            parts: Vec::new(),
            group,
            index_in_group: 0,
            items_in_group: 1,
            keep_with_next: false,
            keep_lines: false,
            widow_control: false,
            break_before: false,
            repeat: false,
            table: None,
            footnotes: Vec::new(),
        });
    }
}

// ------------------------------------------------------------------ tables

/// Column widths for a table, from its grid and the space available.
///
/// The grid is a starting point rather than the answer: a table whose columns do
/// not add up to its width is normal — Word writes the widths it last measured —
/// so they are scaled to fit. A `w:tblLayout w:type="fixed"` table is not
/// scaled, because there the grid *is* the answer.
pub fn column_widths(table: &Table, available: f64) -> Vec<f64> {
    let count = table.columns().max(1) as usize;
    let mut widths: Vec<f64> = (0..count)
        .map(|index| table.grid.get(index).map(|w| w.points()).unwrap_or(0.0))
        .collect();
    let total: f64 = widths.iter().sum();

    // The grid decides, unless it does not fit. This is what `auto` and `nil`
    // mean, and it is also the only sane reading of a declared width that comes
    // out non-positive — `w:tblW w:w="0" w:type="dxa"` is written by real
    // producers for a table that is anything but zero wide, and scaling the
    // columns by zero would collapse them to nothing.
    let from_grid = if total > 0.0 {
        total.min(available)
    } else {
        available
    };
    let target = match table.props.width {
        Width::Fixed(twips) if twips.points() > 0.0 => twips.points(),
        Width::Percent(pct) if pct.0 > 0 => pct.of(Twips::from_points(available)).points(),
        _ => from_grid,
    };

    if total <= 0.0 {
        let each = target / count as f64;
        return vec![each; count];
    }
    if table.props.layout == wp_model::table::TableLayout::Fixed && total <= available + 0.01 {
        return widths;
    }
    let scale = target / total;
    for width in &mut widths {
        *width *= scale;
    }
    widths
}

#[allow(clippy::too_many_arguments)]
fn flow_table(
    table: &Table,
    document: &Document,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
    counters: &mut Counters,
    available: f64,
    into: &mut Flow,
) {
    let widths = column_widths(table, available);
    let indent = table
        .props
        .indent
        .and_then(|w| w.resolve(Twips::from_points(available)))
        .map(|t| t.points())
        .unwrap_or(0.0);
    // The style chain is heard from: a table whose margins live in its style —
    // where Google Docs puts them — pads its cells all the same.
    let margins = document.styles.resolve_cell_margins(&table.props);
    let pad_start = margins
        .start
        .and_then(|w| w.resolve(Twips::from_points(available)))
        .map(|t| t.points())
        .unwrap_or(5.4);
    let pad_end = margins
        .end
        .and_then(|w| w.resolve(Twips::from_points(available)))
        .map(|t| t.points())
        .unwrap_or(5.4);
    let pad_top = margins
        .top
        .and_then(|w| w.resolve(Twips::from_points(available)))
        .map(|t| t.points())
        .unwrap_or(0.0);
    let pad_bottom = margins
        .bottom
        .and_then(|w| w.resolve(Twips::from_points(available)))
        .map(|t| t.points())
        .unwrap_or(0.0);

    let table_id = into.items.len();
    // A header row repeats only while every row before it also says so: Word
    // stops at the first row that does not.
    let mut still_header = true;

    for row in table.rows.iter() {
        let mut cells: Vec<CellPlan> = Vec::new();
        let mut column = row.props.grid_before;
        let mut x = indent
            + widths
                .iter()
                .take(row.props.grid_before as usize)
                .sum::<f64>();

        for cell in &row.cells {
            let span = cell.props.span();
            let cell_width: f64 = widths
                .iter()
                .skip(column as usize)
                .take(span as usize)
                .sum();
            let inner = (cell_width - pad_start - pad_end).max(1.0);

            // A continuation cell draws its background and its borders and holds
            // no content of its own — the content is in the cell that started
            // the merge.
            let is_continuation = cell.props.is_merged_up();
            // The cell flows into its own Flow so the row can be banded, but
            // the paragraph numbering is the document's: a fresh counter here
            // once numbered every cell from zero, and a caret in any table then
            // named a paragraph near the top of the document — clicking in a
            // cell edited text the user was not looking at.
            let mut cell_flow = Flow {
                items: Vec::new(),
                paragraphs: into.paragraphs,
            };
            if !is_continuation {
                for block in &cell.content {
                    flow_block(
                        block,
                        document,
                        ctx,
                        shaper,
                        counters,
                        inner,
                        &mut cell_flow,
                    );
                }
                into.paragraphs = cell_flow.paragraphs;
            } else {
                // Not laid out, but still in the document's flattened order:
                // the numbering has to step over it all the same.
                into.paragraphs += count_paragraphs_in(&cell.content);
            }

            // Each of the cell's lines, with where it starts. Kept apart rather
            // than flattened into one block so the row can be split between two
            // of them.
            let mut lines: Vec<CellLine> = Vec::new();
            let mut y = 0.0;
            for item in cell_flow.items {
                let parts = item
                    .parts
                    .into_iter()
                    .map(|part| Placement {
                        x: x + pad_start + part.x,
                        ..part
                    })
                    .collect();
                lines.push(CellLine {
                    top: y,
                    height: item.height,
                    parts,
                });
                y += item.height;
            }

            cells.push(CellPlan {
                x,
                width: cell_width,
                align: cell.props.v_align,
                fill: cell
                    .props
                    .shading
                    .and_then(|s| s.background())
                    .and_then(|c| c.resolve(&document.theme)),
                borders: [
                    (
                        Side::Top,
                        cell.props.borders.top.or(table.props.borders.top),
                    ),
                    (
                        Side::Start,
                        cell.props.borders.start.or(table.props.borders.start),
                    ),
                    (
                        Side::Bottom,
                        cell.props.borders.bottom.or(table.props.borders.bottom),
                    ),
                    (
                        Side::End,
                        cell.props.borders.end.or(table.props.borders.end),
                    ),
                ],
                content: y,
                lines,
            });

            x += cell_width;
            column += span;
        }

        let tallest = cells.iter().map(|c| c.content).fold(0.0f64, f64::max);
        let mut height = tallest + pad_top + pad_bottom;
        // A stated row height is a floor or a ceiling depending on its rule.
        if let Some(rule) = row.props.height {
            height = match rule {
                wp_model::table::RowHeight::Auto => height,
                wp_model::table::RowHeight::AtLeast(t) => height.max(t.points()),
                wp_model::table::RowHeight::Exact(t) => t.points(),
            };
        }
        let inner_height = (height - pad_top - pad_bottom).max(0.0);
        // Vertical alignment is a shift of the cell's lines within the row's
        // final height, which is why it can only be applied once that is known.
        for cell in &mut cells {
            let offset = cell_offset(cell.align, cell.content, inner_height);
            if offset > 0.0 {
                for line in &mut cell.lines {
                    line.top += offset;
                }
            }
        }

        let bands = split_points(&cells, inner_height);
        let last_band = bands.len() - 2;
        still_header = still_header && row.props.header;
        let group = into.items.len();
        for (band, pair) in bands.windows(2).enumerate() {
            let (top, bottom) = (pair[0], pair[1]);
            let is_first = band == 0;
            let is_last = band == last_band;
            let band_height = (bottom - top)
                + if is_first { pad_top } else { 0.0 }
                + if is_last { pad_bottom } else { 0.0 };
            let above = if is_first { pad_top } else { 0.0 };
            let mut parts: Vec<Placement> = Vec::new();
            for cell in &cells {
                if let Some(fill) = cell.fill {
                    parts.push(Placement {
                        x: cell.x,
                        y: 0.0,
                        width: cell.width,
                        height: band_height,
                        kind: Placed::Fill(fill),
                    });
                }
                for (side, border) in cell.borders {
                    // The row's top edge is drawn once, above the first band,
                    // and its bottom edge once, below the last. Drawing either
                    // on every band would rule a line across the middle of a
                    // cell wherever a page break happened to fall.
                    if (side == Side::Top && !is_first) || (side == Side::Bottom && !is_last) {
                        // Unless a page break cuts the row right here — then
                        // Word closes the fragment with the cell's border.
                        // Pagination knows where the cuts land; the edge goes
                        // along as a maybe.
                        if let Some(border) = border.filter(|b| b.style.draws()) {
                            parts.push(Placement {
                                x: cell.x,
                                y: 0.0,
                                width: cell.width,
                                height: band_height,
                                kind: Placed::BreakEdge { border, side },
                            });
                        }
                        continue;
                    }
                    if let Some(border) = border.filter(|b| b.style.draws()) {
                        parts.push(Placement {
                            x: cell.x,
                            y: 0.0,
                            width: cell.width,
                            height: band_height,
                            kind: Placed::Edge { border, side },
                        });
                    }
                }
                for line in &cell.lines {
                    if line.top < top - EPSILON || line.top >= bottom - EPSILON {
                        continue;
                    }
                    let dy = above + (line.top - top);
                    for part in &line.parts {
                        parts.push(Placement {
                            y: dy + part.y,
                            ..part.clone()
                        });
                    }
                }
            }
            into.items.push(Item {
                height: band_height,
                parts,
                group,
                index_in_group: band,
                items_in_group: last_band + 1,
                keep_with_next: false,
                keep_lines: row.props.cant_split,
                widow_control: false,
                break_before: false,
                repeat: still_header,
                table: Some(table_id),
                footnotes: Vec::new(),
            });
        }
    }
}

/// How many paragraphs a run of blocks holds, counted exactly the way
/// [`wp_model::Document::paragraphs`] flattens them.
fn count_paragraphs_in(blocks: &[Block]) -> usize {
    blocks
        .iter()
        .map(|block| match block {
            Block::Paragraph(_) => 1,
            Block::Table(table) => table
                .rows
                .iter()
                .flat_map(|row| &row.cells)
                .map(|cell| count_paragraphs_in(&cell.content))
                .sum(),
            Block::Structured(sdt) => count_paragraphs_in(&sdt.content),
            _ => 0,
        })
        .sum()
}

/// Half a thousandth of a point: what split points are compared at.
const EPSILON: f64 = 0.0005;

/// One cell of a row, flowed but not yet placed.
struct CellPlan {
    x: f64,
    width: f64,
    align: CellVAlign,
    fill: Option<[u8; 3]>,
    borders: [(Side, Option<Border>); 4],
    /// How tall the cell's own content came out.
    content: f64,
    lines: Vec<CellLine>,
}

/// One line of a cell, and where it sits in the row.
struct CellLine {
    top: f64,
    height: f64,
    parts: Vec<Placement>,
}

/// Where a row may be broken, as offsets from the top of its content box.
///
/// **A row is split between lines, never through one.** Every line boundary of
/// every cell is a candidate; a candidate that falls *inside* some other cell's
/// line is not a height a page can end at, so it is dropped. What is left is
/// where the row can be cut, the row's own two edges included.
///
/// A row with nothing to split on comes back as a single band and travels
/// whole. That is the honest answer when two columns of text line up on
/// nothing: Word would break each cell on its own line boundaries and leave the
/// two columns of one row at different heights, and a row drawn in two pieces
/// that do not agree where they were cut is worse than a row that moved.
fn split_points(cells: &[CellPlan], inner_height: f64) -> Vec<f64> {
    let key = |v: f64| (v * 1000.0).round() as i64;
    let total = key(inner_height);
    let mut offsets: std::collections::BTreeSet<i64> = std::collections::BTreeSet::new();
    offsets.insert(0);
    offsets.insert(total);
    for cell in cells {
        for line in &cell.lines {
            offsets.insert(key(line.top));
            offsets.insert(key(line.top + line.height));
        }
    }
    let inside = |at: i64| {
        cells.iter().any(|cell| {
            cell.lines
                .iter()
                .any(|line| key(line.top) < at && at < key(line.top + line.height))
        })
    };
    let points: Vec<f64> = offsets
        .into_iter()
        .filter(|at| *at <= total && (*at == 0 || *at == total || !inside(*at)))
        .map(|at| at as f64 / 1000.0)
        .collect();
    if points.len() < 2 {
        return vec![0.0, inner_height];
    }
    points
}

// -------------------------------------------------------------- pagination

/// Splits the flow into pages, returning the index one past the last item on
/// each page.
///
/// The break is chosen by filling and then **pulling back**: a keep rule can
/// only be honoured once it is known that the thing it keeps something with does
/// not fit, and by then the decision has already been made.
pub fn paginate(items: &[Item], height: f64) -> Vec<usize> {
    let mut breaks = Vec::new();
    let mut start = 0usize;
    while start < items.len() {
        let mut y = 0.0;
        let mut end = start;
        // Items repeated at the top of a continuation page — a table's header
        // rows — cost their height on every page after the first.
        if !breaks.is_empty() {
            y += repeated_height(items, start);
        }
        while end < items.len() {
            let item = &items[end];
            if item.break_before && end > start {
                break;
            }
            if y + item.height > height + 0.01 && end > start {
                break;
            }
            y += item.height;
            end += 1;
        }
        if end == start {
            // One item taller than the page. Placing it whole and overflowing is
            // better than an empty page followed by the same problem.
            end += 1;
        }
        if end < items.len() {
            end = pull_back(items, start, end);
        }
        breaks.push(end);
        start = end;
    }
    if breaks.is_empty() {
        breaks.push(0);
    }
    breaks
}

/// The height of the header rows that repeat above `start`.
fn repeated_height(items: &[Item], start: usize) -> f64 {
    let Some(item) = items.get(start) else {
        return 0.0;
    };
    let Some(table) = item.table else {
        return 0.0;
    };
    items
        .iter()
        .take(start)
        .filter(|earlier| earlier.table == Some(table) && earlier.repeat)
        .map(|earlier| earlier.height)
        .sum()
}

/// Moves a page break earlier until it satisfies the keep rules.
fn pull_back(items: &[Item], start: usize, mut end: usize) -> usize {
    let floor = start + 1;
    // Keep-with-next: a paragraph that must stay with the next one travels with
    // it. Walk back over every linked item.
    while end > floor && items[end - 1].keep_with_next {
        end -= 1;
    }
    // Keep-lines: a group whose lines may not be split moves whole.
    if end > floor {
        let item = &items[end - 1];
        if item.keep_lines && item.index_in_group + 1 < item.items_in_group {
            let group = item.group;
            while end > floor && items[end - 1].group == group {
                end -= 1;
            }
        }
    }
    // Widow and orphan control: never one line of a paragraph alone at the
    // bottom of a page, and never one alone at the top of the next.
    if end > floor {
        let last = &items[end - 1];
        if last.widow_control && last.items_in_group >= 3 {
            let left_below = last.items_in_group - (last.index_in_group + 1);
            if left_below == 1 {
                // One line would be orphaned onto the next page: take another.
                end -= 1;
            } else if last.index_in_group == 0 && end > floor {
                // One line alone at the bottom: push it over.
                end -= 1;
            }
        }
    }
    end.max(floor)
}

/// Where an anchored drawing sits on the page, in points.
///
/// An inline drawing is placed by the line it is in and never reaches here. An
/// anchored one states its own position, relative to one of eleven things, and
/// this resolves the ones a page can answer without knowing which side of a
/// spread it is on.
///
/// **Stated limit.** `inside` and `outside` are resolved as left and right: they
/// mean "toward the binding", which is only decided once mirrored margins are
/// implemented.
pub fn anchor_position(drawing: &wp_model::Drawing, page: &PageBox, line_top: f64) -> (f64, f64) {
    use wp_model::doc::{Alignment, RelativeTo};

    let Some(position) = &drawing.position else {
        return (page.start, line_top);
    };
    let width = drawing.extent.0.points();
    let height = drawing.extent.1.points();

    let x = match (position.horizontal.align, position.horizontal.offset) {
        (Some(align), _) => {
            let (left, right) = match position.horizontal.relative_to {
                RelativeTo::Page => (0.0, page.width),
                _ => (page.start, page.width - page.end),
            };
            match align {
                Alignment::Center => (left + right) / 2.0 - width / 2.0,
                Alignment::Right | Alignment::Outside => right - width,
                _ => left,
            }
        }
        (None, Some(offset)) => {
            let base = match position.horizontal.relative_to {
                RelativeTo::Page => 0.0,
                RelativeTo::RightMargin => page.width - page.end,
                _ => page.start,
            };
            base + offset.points()
        }
        (None, None) => page.start,
    };

    let y = match (position.vertical.align, position.vertical.offset) {
        (Some(align), _) => {
            let (top, bottom) = match position.vertical.relative_to {
                RelativeTo::Page => (0.0, page.height),
                _ => (page.top, page.height - page.bottom),
            };
            match align {
                Alignment::Center => (top + bottom) / 2.0 - height / 2.0,
                Alignment::Bottom => bottom - height,
                _ => top,
            }
        }
        (None, Some(offset)) => {
            let base = match position.vertical.relative_to {
                RelativeTo::Page => 0.0,
                RelativeTo::Margin | RelativeTo::TopMargin => page.top,
                RelativeTo::BottomMargin => page.height - page.bottom,
                // Relative to the paragraph or the line: from where the text is.
                // This is what makes a picture travel with the paragraph it
                // belongs to rather than staying where it was written.
                _ => line_top,
            };
            base + offset.points()
        }
        (None, None) => line_top,
    };

    (x, y)
}

/// The origin each axis of an anchored drawing measures from.
///
/// Subtracting this from a position gives the offset that would put a drawing
/// there, which is what dragging one needs: the user moves it on the page, and
/// the file has to say the same thing in the drawing's own frame of reference.
pub fn anchor_base(drawing: &wp_model::Drawing, page: &PageBox, line_top: f64) -> (f64, f64) {
    use wp_model::doc::RelativeTo;

    let Some(position) = &drawing.position else {
        return (page.start, line_top);
    };
    let x = match position.horizontal.relative_to {
        RelativeTo::Page => 0.0,
        RelativeTo::RightMargin => page.width - page.end,
        _ => page.start,
    };
    let y = match position.vertical.relative_to {
        RelativeTo::Page => 0.0,
        RelativeTo::Margin | RelativeTo::TopMargin => page.top,
        RelativeTo::BottomMargin => page.height - page.bottom,
        _ => line_top,
    };
    (x, y)
}

/// Every anchored drawing of a paragraph.
pub fn anchored(paragraph: &Paragraph) -> Vec<(usize, &wp_model::Drawing)> {
    paragraph
        .drawings()
        .into_iter()
        .enumerate()
        .filter(|(_, drawing)| drawing.anchored)
        .collect()
}

/// The vertical alignment of a cell's content, for a renderer that places it.
pub fn cell_offset(align: CellVAlign, content: f64, available: f64) -> f64 {
    match align {
        CellVAlign::Top => 0.0,
        CellVAlign::Center => ((available - content) / 2.0).max(0.0),
        CellVAlign::Bottom => (available - content).max(0.0),
    }
}

/// Whether a cell holds the content of a vertical merge or continues one.
pub fn is_merge_origin(cell: &wp_model::table::Cell) -> bool {
    !matches!(cell.props.v_merge, Some(VMerge::Continue))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::doc::{Inline, Run};
    use wp_model::prop::ParaProps;
    use wp_model::style::{Style, StyleKind};
    use wp_model::table::{Cell, CellProps, Row, RowProps};
    use wp_model::units::HalfPoint;

    /// A document whose Normal style is 10pt, so the fixed shaper makes every
    /// line exactly ten points tall and every character five wide.
    fn document(blocks: Vec<Block>) -> Document {
        let mut document = Document {
            body: blocks,
            ..Document::new()
        };
        let mut normal = Style::new("Normal", StyleKind::Paragraph);
        normal.default = true;
        normal.run.size = Some(HalfPoint(20));
        document.styles.insert(normal);
        document
    }

    fn ctx<'a>(theme: &'a wp_model::color::Theme) -> Context<'a> {
        Context {
            theme,
            default_tab: Twips(720),
            fallback_font: "test",
            show_revisions: true,
            show_hidden: false,
            fields: Box::leak(Box::new(crate::field::FieldValues::default())),
            band: None,
        }
    }

    fn pages(document: &Document) -> Vec<Page> {
        let theme = document.theme.clone();
        let mut shaper = crate::shape::Fixed;
        layout(document, &ctx(&theme), &mut shaper)
    }

    /// A page holding exactly `lines` lines of ten-point text.
    fn page_of(lines: usize) -> SectionProps {
        let mut section = SectionProps::new();
        // Text height = page height - top - bottom.
        let height = Twips::from_points(lines as f64 * 10.0);
        section.page.height = Twips(height.0 + section.margins.top.0 + section.margins.bottom.0);
        section
    }

    fn paragraphs(count: usize) -> Vec<Block> {
        (0..count)
            .map(|index| Block::Paragraph(Paragraph::of(&format!("p{index}"))))
            .collect()
    }

    #[test]
    fn a_document_that_fits_is_one_page() {
        let mut document = document(paragraphs(3));
        document.section = page_of(10);
        let pages = pages(&document);
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].number, 1);
        assert_eq!(pages[0].content.len(), 3);
    }

    #[test]
    fn a_document_that_does_not_fit_breaks_onto_the_next_page() {
        let mut document = document(paragraphs(12));
        document.section = page_of(5);
        let pages = pages(&document);
        assert_eq!(pages.len(), 3);
        assert_eq!(pages[0].content.len(), 5);
        assert_eq!(pages[1].content.len(), 5);
        assert_eq!(pages[2].content.len(), 2);
        assert_eq!(pages[2].number, 3);
    }

    #[test]
    fn lines_are_placed_down_the_page_from_the_top_margin() {
        let mut document = document(paragraphs(3));
        document.section = page_of(10);
        let page = &pages(&document)[0];
        let top = page.geometry.top;
        let ys: Vec<f64> = page.content.iter().map(|p| p.y).collect();
        assert_eq!(ys, [top, top + 10.0, top + 20.0]);
        assert!(page.content.iter().all(|p| p.x == page.geometry.start));
    }

    #[test]
    fn page_break_before_starts_a_page_even_with_room_to_spare() {
        let mut blocks = paragraphs(2);
        let mut breaking = Paragraph::of("new page");
        breaking.props.page_break_before = Some(true);
        blocks.push(Block::Paragraph(breaking));
        let mut document = document(blocks);
        document.section = page_of(20);
        let pages = pages(&document);
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[1].content.len(), 1);
    }

    #[test]
    fn an_explicit_page_break_inside_a_run_ends_the_page() {
        let paragraph = Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![
                    Piece::Text("before".into()),
                    Piece::Break(Break::Page),
                    Piece::Text("after".into()),
                ],
                ..Run::new()
            })],
            ..Paragraph::new()
        };
        let mut document = document(vec![Block::Paragraph(paragraph)]);
        document.section = page_of(20);
        let pages = pages(&document);
        assert_eq!(pages.len(), 2);
    }

    #[test]
    fn keep_with_next_moves_a_heading_to_join_its_paragraph() {
        // The rule that stops a heading sitting alone at the foot of a page.
        let mut heading = Paragraph::of("heading");
        heading.props.keep_next = Some(true);
        let mut blocks = paragraphs(4);
        blocks.push(Block::Paragraph(heading));
        blocks.push(Block::Paragraph(Paragraph::of("body")));

        let mut document = document(blocks);
        document.section = page_of(5);
        let pages = pages(&document);
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].content.len(), 4, "the heading did not stay behind");
        assert_eq!(pages[1].content.len(), 2, "it travelled with its body");
    }

    #[test]
    fn keep_lines_moves_a_whole_paragraph_rather_than_splitting_it() {
        let mut kept = Paragraph::of("aa bb cc dd ee ff gg hh");
        kept.props.keep_lines = Some(true);
        let mut blocks = paragraphs(3);
        blocks.push(Block::Paragraph(kept));

        let mut document = document(blocks);
        // 25 points of text width: two words a line, so the kept paragraph is
        // four lines and cannot fit in the two remaining.
        document.section = page_of(5);
        document.section.page.width =
            Twips::from_points(25.0 + document.section.margins.start.points() * 2.0);
        let pages = pages(&document);
        assert!(pages.len() >= 2);
        assert_eq!(pages[0].content.len(), 3, "the kept paragraph moved whole");
    }

    #[test]
    fn widow_control_does_not_leave_one_line_of_a_paragraph_behind() {
        // Four lines, three of which fit. Without the rule the fourth would sit
        // alone at the top of the next page.
        let long = Paragraph::of("aa bb cc dd ee ff gg hh");
        let mut document = document(vec![Block::Paragraph(long)]);
        document.section = page_of(3);
        document.section.page.width =
            Twips::from_points(25.0 + document.section.margins.start.points() * 2.0);
        let pages = pages(&document);
        assert_eq!(pages.len(), 2);
        assert_eq!(
            pages[0].content.len(),
            2,
            "a line was pushed over so two travel together"
        );
        assert_eq!(pages[1].content.len(), 2);
    }

    #[test]
    fn a_table_becomes_one_item_per_row() {
        let table = Table {
            grid: vec![Twips(1440), Twips(1440)],
            rows: vec![
                Row {
                    cells: vec![cell("a"), cell("b")],
                    ..Row::new()
                },
                Row {
                    cells: vec![cell("c"), cell("d")],
                    ..Row::new()
                },
            ],
            ..Table::new()
        };
        let mut document = document(vec![Block::Table(table)]);
        document.section = page_of(20);
        let page = &pages(&document)[0];
        // Two rows of two cells: four lines, and the second row's pair sits a
        // line below the first's.
        assert_eq!(page.content.len(), 4);
        assert_eq!(page.content[0].y, page.geometry.top);
        assert_eq!(page.content[1].y, page.geometry.top);
        assert_eq!(page.content[2].y, page.geometry.top + 10.0);
    }

    #[test]
    fn a_table_styles_cell_margins_pad_every_row() {
        // The margins live in the table's *style* — where Google Docs puts
        // them — and a layout that read only the table's own tblCellMar drew
        // every row 5.5pt short and every text column 5.3pt narrow. The rows
        // of the two tables here differ only in the style's say-so.
        use wp_model::table::Width;
        let two_rows = || Table {
            grid: vec![Twips(1440)],
            rows: vec![
                Row {
                    cells: vec![cell("a")],
                    ..Row::new()
                },
                Row {
                    cells: vec![cell("b")],
                    ..Row::new()
                },
            ],
            ..Table::new()
        };
        let line_ys = |document: &Document| -> Vec<f64> {
            pages(document)[0]
                .content
                .iter()
                .filter(|p| matches!(p.kind, Placed::Line { .. }))
                .map(|p| p.y)
                .collect()
        };

        let mut plain = document(vec![Block::Table(two_rows())]);
        plain.section = page_of(20);
        let plain_ys = line_ys(&plain);

        let mut style = wp_model::Style::new("Boxed", wp_model::StyleKind::Table);
        style.cell_margins = wp_model::table::CellMargins {
            top: Some(Width::Fixed(Twips(55))),
            start: Some(Width::Fixed(Twips(55))),
            bottom: Some(Width::Fixed(Twips(55))),
            end: Some(Width::Fixed(Twips(55))),
        };
        let mut padded = document(vec![Block::Table(two_rows())]);
        let id = padded.styles.insert(style);
        if let Block::Table(table) = &mut padded.body[0] {
            table.props.style = Some(id);
        }
        padded.section = page_of(20);
        let padded_ys = line_ys(&padded);

        // 55 twips is 2.75pt: the first line sits that much below the row's
        // top, and the second row starts 5.5pt later than it otherwise would.
        assert_eq!(padded_ys[0], plain_ys[0] + 2.75);
        assert_eq!(padded_ys[1], plain_ys[1] + 2.75 + 5.5);
    }

    #[test]
    fn a_row_taller_than_the_page_is_split_between_the_lines_of_its_cells() {
        // The shape every table-heavy document has: one short cell beside one
        // long one. Word breaks the row and carries the rest over, and a reader
        // that moves the whole row instead leaves most of a page blank.
        let table = Table {
            grid: vec![Twips(1440), Twips(1440)],
            rows: vec![Row {
                cells: vec![
                    cell("date"),
                    Cell {
                        props: CellProps::new(),
                        content: paragraphs(10),
                    },
                ],
                ..Row::new()
            }],
            ..Table::new()
        };
        let mut document = document(vec![Block::Table(table)]);
        document.section = page_of(6);
        let pages = pages(&document);
        assert!(pages.len() >= 2, "the row did not split");
        assert!(
            pages[0]
                .content
                .iter()
                .any(|p| matches!(&p.kind, Placed::Line { .. })),
            "the first page holds part of the row"
        );
        // Nothing is lost or drawn twice: ten lines in the tall cell and one in
        // the short one, across every page.
        let lines: usize = pages
            .iter()
            .map(|page| {
                page.content
                    .iter()
                    .filter(|p| matches!(p.kind, Placed::Line { .. }))
                    .count()
            })
            .sum();
        assert_eq!(lines, 11);
    }

    #[test]
    fn a_row_that_says_it_cannot_be_split_moves_whole() {
        let table = Table {
            grid: vec![Twips(1440)],
            rows: vec![Row {
                props: RowProps {
                    cant_split: true,
                    ..RowProps::default()
                },
                cells: vec![Cell {
                    props: CellProps::new(),
                    content: paragraphs(6),
                }],
            }],
            ..Table::new()
        };
        let mut blocks = paragraphs(3);
        blocks.push(Block::Table(table));
        let mut document = document(blocks);
        document.section = page_of(8);
        let pages = pages(&document);
        assert_eq!(pages.len(), 2);
        assert_eq!(
            pages[0].content.len(),
            3,
            "the row moved rather than being cut"
        );
    }

    #[test]
    fn a_row_cut_by_a_page_is_closed_on_both_sides_of_the_cut() {
        // The page break must not rule a line across the middle of a cell —
        // but where it genuinely cuts the row, Word closes both fragments
        // with the cell's own border: a bottom rule above the cut and a top
        // rule below it, so each page shows a whole box rather than three
        // sides and a hole.
        let border = Border {
            style: wp_model::prop::BorderStyle::Single,
            size: Some(wp_model::units::Eighth(4)),
            color: None,
            space: None,
            shadow: false,
        };
        let mut props = CellProps::new();
        props.borders.top = Some(border);
        props.borders.bottom = Some(border);
        props.borders.start = Some(border);
        let table = Table {
            grid: vec![Twips(1440)],
            rows: vec![Row {
                cells: vec![Cell {
                    props,
                    content: paragraphs(8),
                }],
                ..Row::new()
            }],
            ..Table::new()
        };
        let mut document = document(vec![Block::Table(table)]);
        document.section = page_of(5);
        let pages = pages(&document);
        assert_eq!(pages.len(), 2, "the row splits across two pages");
        let count = |page: &Page, side: Side| -> usize {
            page.content
                .iter()
                .filter(|p| matches!(&p.kind, Placed::Edge { side: s, .. } if *s == side))
                .count()
        };
        for page in &pages {
            assert_eq!(count(page, Side::Top), 1, "each fragment closed above");
            assert_eq!(count(page, Side::Bottom), 1, "each fragment closed below");
            assert!(
                count(page, Side::Start) >= 1,
                "the side edge is drawn on every band it passes through"
            );
            assert!(
                !page
                    .content
                    .iter()
                    .any(|p| matches!(p.kind, Placed::BreakEdge { .. })),
                "a maybe-edge never reaches a page unresolved"
            );
        }
    }

    fn cell(text: &str) -> Cell {
        Cell {
            props: CellProps::new(),
            content: vec![Block::Paragraph(Paragraph::of(text))],
        }
    }

    #[test]
    fn every_line_names_the_paragraph_the_document_flattening_names() {
        // A fresh counter per cell once numbered every cell's paragraphs from
        // zero — and a caret in a table then edited text near the top of the
        // document instead of the text under it.
        let table = Table {
            grid: vec![Twips(1440), Twips(1440)],
            rows: vec![
                Row {
                    cells: vec![cell("cell-a"), cell("cell-b")],
                    ..Row::new()
                },
                Row {
                    cells: vec![cell("cell-c"), cell("cell-d")],
                    ..Row::new()
                },
            ],
            ..Table::new()
        };
        let mut blocks = vec![Block::Paragraph(Paragraph::of("before"))];
        blocks.push(Block::Table(table));
        blocks.push(Block::Paragraph(Paragraph::of("after")));
        let mut document = document(blocks);
        document.section = page_of(30);
        let flattened = document.paragraphs();
        let mut seen = 0;
        for page in pages(&document) {
            for placement in &page.content {
                let Placed::Line { line, paragraph } = &placement.kind else {
                    continue;
                };
                let text: String = line
                    .fragments
                    .iter()
                    .filter_map(|fragment| match &fragment.content {
                        crate::inline::Content::Text { text, .. } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    flattened[*paragraph].text(),
                    text,
                    "line claims paragraph {paragraph}"
                );
                seen += 1;
            }
        }
        assert_eq!(seen, 6, "one line per paragraph, tables included");
    }

    #[test]
    fn a_cells_content_is_placed_inside_its_column_and_its_padding() {
        let table = Table {
            grid: vec![Twips(1440), Twips(1440)],
            rows: vec![Row {
                cells: vec![cell("a"), cell("b")],
                ..Row::new()
            }],
            ..Table::new()
        };
        let mut document = document(vec![Block::Table(table)]);
        document.section = page_of(20);
        let page = &pages(&document)[0];
        let xs: Vec<f64> = page.content.iter().map(|p| p.x).collect();
        // The first cell starts at the margin plus Word's own 0.08in padding;
        // the second an inch further across.
        assert_eq!(xs[0], page.geometry.start + 5.4);
        assert_eq!(xs[1], page.geometry.start + 72.0 + 5.4);
    }

    #[test]
    fn a_column_grid_that_does_not_fit_is_scaled_rather_than_clipped() {
        let table = Table {
            grid: vec![Twips(7200), Twips(7200)],
            rows: vec![Row {
                cells: vec![cell("a"), cell("b")],
                ..Row::new()
            }],
            ..Table::new()
        };
        // Ten inches of grid into a six-inch column.
        let widths = column_widths(&table, 432.0);
        assert_eq!(widths.len(), 2);
        assert!((widths.iter().sum::<f64>() - 432.0).abs() < 0.01);
        assert_eq!(widths[0], widths[1]);
    }

    #[test]
    fn a_declared_width_of_zero_is_read_as_auto_rather_than_as_nothing() {
        // `w:tblW w:w="0" w:type="dxa"` is written by real producers for a
        // table that is anything but zero wide. Scaling the columns by zero
        // collapses every one of them to a single character per line.
        let table = Table {
            grid: vec![Twips(1440), Twips(2880)],
            props: wp_model::table::TableProps {
                width: Width::Fixed(Twips(0)),
                ..wp_model::table::TableProps::default()
            },
            rows: vec![Row {
                cells: vec![cell("a"), cell("b")],
                ..Row::new()
            }],
        };
        let widths = column_widths(&table, 432.0);
        assert_eq!(widths, [72.0, 144.0], "the grid decided");
    }

    #[test]
    fn a_table_with_no_grid_shares_the_width_out_evenly() {
        let table = Table {
            rows: vec![Row {
                cells: vec![cell("a"), cell("b"), cell("c")],
                ..Row::new()
            }],
            ..Table::new()
        };
        let widths = column_widths(&table, 300.0);
        assert_eq!(widths, [100.0, 100.0, 100.0]);
    }

    #[test]
    fn a_header_row_repeats_and_only_the_rows_that_say_so_do() {
        let header = Row {
            props: RowProps {
                header: true,
                ..RowProps::default()
            },
            cells: vec![cell("H")],
        };
        let mut rows = vec![header];
        for index in 0..12 {
            rows.push(Row {
                cells: vec![cell(&format!("r{index}"))],
                ..Row::new()
            });
        }
        let table = Table {
            grid: vec![Twips(2880)],
            rows,
            ..Table::new()
        };
        let mut document = document(vec![Block::Table(table)]);
        document.section = page_of(5);
        let pages = pages(&document);
        assert!(pages.len() >= 3);
        // The first page holds the header and four rows; the pages after it
        // hold one fewer, because the repeated header costs its height there.
        assert_eq!(pages[0].content.len(), 5);
        assert_eq!(pages[1].content.len(), 4);
    }

    #[test]
    fn a_vertically_merged_cell_holds_no_content_of_its_own() {
        let mut origin = cell("spans two");
        origin.props.v_merge = Some(VMerge::Restart);
        let mut continuation = cell("this text is not in the document");
        continuation.props.v_merge = Some(VMerge::Continue);
        assert!(is_merge_origin(&origin));
        assert!(!is_merge_origin(&continuation));

        let table = Table {
            grid: vec![Twips(2880)],
            rows: vec![
                Row {
                    cells: vec![origin],
                    ..Row::new()
                },
                Row {
                    cells: vec![continuation],
                    ..Row::new()
                },
            ],
            ..Table::new()
        };
        let mut document = document(vec![Block::Table(table)]);
        document.section = page_of(20);
        let page = &pages(&document)[0];
        let drawn: Vec<String> = page
            .content
            .iter()
            .filter_map(|p| match &p.kind {
                Placed::Line { line, .. } => Some(
                    line.fragments
                        .iter()
                        .filter_map(|f| match &f.content {
                            crate::inline::Content::Text { text, .. } => Some(text.clone()),
                            _ => None,
                        })
                        .collect::<String>(),
                ),
                _ => None,
            })
            .collect();
        assert!(drawn.iter().any(|text| text.contains("spans")));
        assert!(
            !drawn.iter().any(|text| text.contains("not in the")),
            "a continuation cell's content is the origin's: {drawn:?}"
        );
    }

    #[test]
    fn a_section_break_starts_a_new_page_with_its_own_geometry() {
        let mut first = Paragraph::of("portrait");
        let mut landscape = SectionProps::new();
        landscape.page = landscape.page.rotated();
        first.section = Some(Box::new(landscape));

        let mut document = document(vec![
            Block::Paragraph(first),
            Block::Paragraph(Paragraph::of("landscape")),
        ]);
        document.section = page_of(200);
        let pages = pages(&document);
        assert_eq!(pages.len(), 2);
        assert!(pages[0].geometry.width > pages[0].geometry.height);
        assert!(pages[1].geometry.width < pages[1].geometry.height);
        assert_eq!(pages[1].number, 2);
    }

    #[test]
    fn page_numbering_restarts_where_a_section_says_it_should() {
        let mut first = Paragraph::of("preface");
        let mut preface = SectionProps::new();
        preface.page_numbering.start = Some(1);
        first.section = Some(Box::new(preface));

        let mut document = document(vec![
            Block::Paragraph(first),
            Block::Paragraph(Paragraph::of("chapter one")),
        ]);
        document.section = page_of(20);
        document.section.page_numbering.start = Some(1);
        let pages = pages(&document);
        assert_eq!(pages[0].number, 1);
        assert_eq!(pages[1].number, 1, "the second section restarted at one");
    }

    #[test]
    fn a_header_is_placed_in_the_margin_rather_than_in_the_text_area() {
        let mut document = document(paragraphs(1));
        document.section = page_of(20);
        document.section.headers.push(wp_model::HeaderRef {
            kind: HeaderKind::Default,
            body: wp_model::HeaderId(0),
            rel: None,
        });
        document.headers.push(wp_model::HeaderFooter {
            id: wp_model::HeaderId(0),
            part: None,
            rel: None,
            footer: false,
            content: vec![Block::Paragraph(Paragraph::of("running head"))],
        });
        let page = &pages(&document)[0];
        assert_eq!(page.header.len(), 1);
        assert!(
            page.header[0].y < page.geometry.top,
            "the header sits above the top margin"
        );
    }

    #[test]
    fn a_page_number_in_a_footer_is_a_different_number_on_every_page() {
        // A footer is laid out again for every page it appears on, from the
        // same paragraphs. One field mark for all of them would answer every
        // page with the number of the last.
        let field = Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![
                    Piece::FieldStart {
                        dirty: false,
                        lock: false,
                    },
                    Piece::Instruction(" PAGE ".into()),
                    Piece::FieldSeparate,
                    Piece::FieldEnd,
                ],
                ..Run::new()
            })],
            ..Paragraph::new()
        };
        let mut document = document(paragraphs(9));
        document.section = page_of(4);
        document.section.footers.push(wp_model::HeaderRef {
            kind: HeaderKind::Default,
            body: wp_model::HeaderId(0),
            rel: None,
        });
        document.headers.push(wp_model::HeaderFooter {
            id: wp_model::HeaderId(0),
            part: None,
            rel: None,
            footer: true,
            content: vec![Block::Paragraph(field)],
        });
        let pages = pages(&document);
        assert_eq!(pages.len(), 3);
        let drawn: Vec<String> = pages
            .iter()
            .map(|page| {
                page.footer
                    .iter()
                    .filter_map(|placement| match &placement.kind {
                        Placed::Line { line, .. } => Some(
                            line.fragments
                                .iter()
                                .filter_map(|f| match &f.content {
                                    crate::inline::Content::Text { text, .. } => {
                                        Some(text.as_str())
                                    }
                                    _ => None,
                                })
                                .collect::<String>(),
                        ),
                        _ => None,
                    })
                    .collect()
            })
            .collect();
        assert_eq!(drawn, ["1", "2", "3"]);
    }

    #[test]
    fn a_footer_holding_a_table_is_measured_by_its_stack_not_by_its_cells() {
        // Summing every placement counts one row of three cells as three rows,
        // and the footer floats inches above where it belongs.
        let table = Table {
            grid: vec![Twips(1440), Twips(1440), Twips(1440)],
            rows: vec![Row {
                cells: vec![cell("a"), cell("b"), cell("c")],
                ..Row::new()
            }],
            ..Table::new()
        };
        let mut document = document(paragraphs(1));
        document.section = page_of(30);
        document.section.footers.push(wp_model::HeaderRef {
            kind: HeaderKind::Default,
            body: wp_model::HeaderId(0),
            rel: None,
        });
        document.headers.push(wp_model::HeaderFooter {
            id: wp_model::HeaderId(0),
            part: None,
            rel: None,
            footer: true,
            content: vec![Block::Table(table)],
        });
        let page = &pages(&document)[0];
        let top = page
            .footer
            .iter()
            .map(|p| p.y)
            .fold(f64::INFINITY, f64::min);
        let footer_edge = page.geometry.height - document.section.margins.footer.points();
        // One row of ten points, so its top is ten points above the edge the
        // footer is measured from — not thirty.
        assert!(
            (top - (footer_edge - 10.0)).abs() < 0.01,
            "the footer started at {top}, expected {}",
            footer_edge - 10.0
        );
    }

    #[test]
    fn a_footer_is_measured_up_from_the_bottom_edge() {
        let mut document = document(paragraphs(1));
        document.section = page_of(20);
        document.section.footers.push(wp_model::HeaderRef {
            kind: HeaderKind::Default,
            body: wp_model::HeaderId(0),
            rel: None,
        });
        document.headers.push(wp_model::HeaderFooter {
            id: wp_model::HeaderId(0),
            part: None,
            rel: None,
            footer: true,
            content: vec![Block::Paragraph(Paragraph::of("page 1"))],
        });
        let page = &pages(&document)[0];
        assert_eq!(page.footer.len(), 1);
        let bottom = page.geometry.height - page.geometry.bottom;
        assert!(
            page.footer[0].y > bottom,
            "the footer sits below the bottom margin"
        );
        assert!(page.footer[0].y < page.geometry.height);
    }

    #[test]
    fn a_numbered_list_counts_across_the_whole_document_rather_than_per_page() {
        let mut document = document(Vec::new());
        let mut definition = wp_model::AbstractNum::new(0);
        let mut level = wp_model::Level::new(0);
        level.text = "%1.".into();
        definition.set_level(level);
        document.numbering.insert_abstract(definition);
        document.numbering.insert_num(wp_model::Num::new(1, 0));

        document.body = (0..12)
            .map(|index| {
                Block::Paragraph(Paragraph {
                    props: ParaProps {
                        numbering: Some(wp_model::NumRef {
                            num_id: 1,
                            level: 0,
                        }),
                        ..ParaProps::default()
                    },
                    ..Paragraph::of(&format!("item {index}"))
                })
            })
            .collect();
        document.section = page_of(5);

        let pages = pages(&document);
        assert!(pages.len() >= 3);
        let labelled = pages
            .iter()
            .flat_map(|page| &page.content)
            .filter(|placement| match &placement.kind {
                Placed::Line { line, .. } => line
                    .fragments
                    .iter()
                    .any(|f| matches!(f.content, crate::inline::Content::Label { .. })),
                _ => false,
            })
            .count();
        assert_eq!(labelled, 12, "every item got a label");
    }

    #[test]
    fn a_document_whose_fields_have_settled_is_laid_out_once() {
        // The second pass exists to put the right page number in a `{ PAGE }`.
        // Once it *is* the right number — which is true from the moment the
        // document has been shown once — running it again produces the same
        // pages, and in an editor it would run on every keystroke.
        #[derive(Default)]
        struct Counting {
            asked: usize,
            inner: crate::shape::Fixed,
        }
        impl Shaper for Counting {
            fn metrics(&mut self, font: &crate::shape::FontRequest) -> crate::shape::Metrics {
                self.inner.metrics(font)
            }
            fn advances(
                &mut self,
                text: &str,
                font: &crate::shape::FontRequest,
                out: &mut Vec<crate::shape::Advance>,
            ) {
                self.asked += 1;
                self.inner.advances(text, font, out);
            }
        }

        let mut blocks = paragraphs(10);
        blocks.push(Block::Paragraph(Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![
                    Piece::FieldStart {
                        dirty: false,
                        lock: false,
                    },
                    Piece::Instruction(" PAGE ".into()),
                    Piece::FieldSeparate,
                    Piece::Text("1".into()),
                    Piece::FieldEnd,
                ],
                ..Run::new()
            })],
            ..Paragraph::new()
        }));
        let mut document = document(blocks);
        document.section = page_of(5);

        let theme = document.theme.clone();
        fn make<'a>(
            theme: &'a wp_model::Theme,
            document: &'a Document,
            fields: &'a crate::FieldValues,
        ) -> Context<'a> {
            Context {
                theme,
                default_tab: document.settings.default_tab_stop,
                fallback_font: "Calibri",
                show_revisions: true,
                show_hidden: false,
                fields,
                band: None,
            }
        }

        let empty = crate::FieldValues::default();
        let mut cold = Counting::default();
        let pages = layout(&document, &make(&theme, &document, &empty), &mut cold);
        let settled = evaluate(&pages, &empty);
        assert!(!settled.is_empty(), "the field was evaluated");

        let mut warm = Counting::default();
        let again = layout(&document, &make(&theme, &document, &settled), &mut warm);
        assert_eq!(again.len(), pages.len(), "and the same pages come out");
        assert!(
            warm.asked * 2 <= cold.asked,
            "a settled document measured {} times against {} cold —              the second pass is still running",
            warm.asked,
            cold.asked
        );
    }

    #[test]
    fn a_page_field_shows_the_page_it_is_on_rather_than_the_one_the_file_cached() {
        // The whole point of laying out twice. The file says "1" because that is
        // where the field was when Word last saved; the field is on page three.
        let field = |cached: &str| {
            Block::Paragraph(Paragraph {
                content: vec![Inline::Run(Run {
                    content: vec![
                        Piece::FieldStart {
                            dirty: false,
                            lock: false,
                        },
                        Piece::Instruction(" PAGE ".into()),
                        Piece::FieldSeparate,
                        Piece::Text(cached.into()),
                        Piece::FieldEnd,
                    ],
                    ..Run::new()
                })],
                ..Paragraph::new()
            })
        };
        let mut blocks = paragraphs(10);
        blocks.push(field("1"));
        let mut document = document(blocks);
        document.section = page_of(5);

        let pages = pages(&document);
        assert_eq!(pages.len(), 3);
        let drawn: String = pages[2]
            .content
            .iter()
            .filter_map(|p| match &p.kind {
                Placed::Line { line, .. } => Some(
                    line.fragments
                        .iter()
                        .filter_map(|f| match &f.content {
                            crate::inline::Content::Text { text, .. } => Some(text.clone()),
                            _ => None,
                        })
                        .collect::<String>(),
                ),
                _ => None,
            })
            .collect();
        assert!(drawn.contains("3"), "the page it is on: {drawn:?}");
        assert!(
            !drawn.contains("1"),
            "not the one the file cached: {drawn:?}"
        );
    }

    #[test]
    fn a_document_with_no_page_fields_is_laid_out_once() {
        // The second pass is not free, and a document that cannot need it must
        // not pay for it.
        let mut document = document(paragraphs(3));
        document.section = page_of(20);
        let theme = document.theme.clone();
        let mut shaper = crate::shape::Fixed;
        let values = evaluate(
            &layout_once(&document, &ctx(&theme), &mut shaper),
            &Default::default(),
        );
        assert!(values.is_empty());
    }

    #[test]
    fn an_anchored_drawing_is_placed_on_the_page_rather_than_in_a_line() {
        let drawing = wp_model::Drawing {
            source: Vec::new().into(),
            anchored: true,
            extent: (
                wp_model::Emu::from_points(100.0),
                wp_model::Emu::from_points(50.0),
            ),
            rel: Some("rId7".into()),
            name: None,
            description: None,
            wrap: wp_model::Wrap::Square,
            distance: Default::default(),
            position: None,
            behind_text: false,
        };
        let paragraph = Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![
                    Piece::Text("text".into()),
                    Piece::Drawing(Box::new(drawing)),
                ],
                ..Run::new()
            })],
            ..Paragraph::new()
        };
        let mut document = document(vec![Block::Paragraph(paragraph)]);
        document.section = page_of(20);
        let page = &pages(&document)[0];
        let drawn = page
            .content
            .iter()
            .find(|p| matches!(p.kind, Placed::Drawing { .. }))
            .expect("the drawing was placed");
        assert_eq!(drawn.width, 100.0);
        assert_eq!(drawn.height, 50.0);
        // And the text is still a line of its own, with the picture not in it.
        let line = page
            .content
            .iter()
            .find(|p| matches!(p.kind, Placed::Line { .. }))
            .expect("a line");
        assert!(line.width < 100.0);
    }

    #[test]
    fn a_centred_anchor_is_centred_on_the_text_column() {
        let geometry = PageBox {
            width: 612.0,
            height: 792.0,
            top: 72.0,
            bottom: 72.0,
            start: 72.0,
            end: 72.0,
        };
        let drawing = wp_model::Drawing {
            source: Vec::new().into(),
            anchored: true,
            extent: (
                wp_model::Emu::from_points(100.0),
                wp_model::Emu::from_points(50.0),
            ),
            rel: None,
            name: None,
            description: None,
            wrap: wp_model::Wrap::Square,
            distance: Default::default(),
            position: Some(Box::new(wp_model::doc::DrawingPosition {
                horizontal: wp_model::doc::Offset {
                    relative_to: wp_model::doc::RelativeTo::Margin,
                    offset: None,
                    align: Some(wp_model::doc::Alignment::Center),
                },
                vertical: wp_model::doc::Offset {
                    relative_to: wp_model::doc::RelativeTo::Paragraph,
                    offset: Some(wp_model::Emu::from_points(10.0)),
                    align: None,
                },
            })),
            behind_text: false,
        };
        // The column is 72..540, so its middle is 306 and a 100pt picture
        // starts at 256; the vertical offset is from where the text is.
        assert_eq!(anchor_position(&drawing, &geometry, 400.0), (256.0, 410.0));
    }

    #[test]
    fn cell_alignment_puts_short_content_where_the_cell_says() {
        assert_eq!(cell_offset(CellVAlign::Top, 20.0, 60.0), 0.0);
        assert_eq!(cell_offset(CellVAlign::Center, 20.0, 60.0), 20.0);
        assert_eq!(cell_offset(CellVAlign::Bottom, 20.0, 60.0), 40.0);
        // Content taller than the cell is not pushed off the top.
        assert_eq!(cell_offset(CellVAlign::Bottom, 80.0, 60.0), 0.0);
    }
}
