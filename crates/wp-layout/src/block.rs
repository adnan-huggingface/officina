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
//! **Stated limits**, rather than hidden ones. Text does not wrap *beside* an
//! anchored drawing: every text-anchored float that wraps — `topAndBottom`,
//! square, tight — reserves its full height in the flow and the text resumes
//! below, which is what Word does with the commonest float in the wild, the
//! column-wide or centred picture. A narrow square wrap should share its
//! lines with the text and does not yet — below is the honest stand-in,
//! never text sitting on the picture. A page- or margin-anchored float does
//! not travel with the text and stays an overlay the text runs under. Multi-column sections lay out column by column rather than
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

    /// Everything on the page, in the order a renderer must draw it: border
    /// edges after all else.
    ///
    /// Word paints shading below borders, always. In document order the two
    /// meet exactly on a table's row and column boundaries — the next row's
    /// cell fill starts where this row's hairline bottom rule was just drawn —
    /// and whichever the rasterizer rounds wider wins. On the screen that ate
    /// the borders under three white-shaded rows; in the PDF it ate a column
    /// rule. Putting every edge after every fill ends the coin-toss.
    pub fn painted(&self) -> impl Iterator<Item = &Placement> {
        let edge = |p: &&Placement| matches!(p.kind, Placed::Edge { .. });
        self.everything()
            .filter(move |p| !edge(p))
            .chain(self.everything().filter(edge))
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
    /// The part of `height` that is space-after, which Word lets vanish into
    /// the bottom margin: a line whose type fits on the page stays there even
    /// when its trailing space would not.
    pub slack: f64,
}

/// A document, flowed into items and not yet paginated.
#[derive(Debug, Clone, Default)]
pub struct Flow {
    pub items: Vec<Item>,
    /// How many paragraphs have been flowed. Counts in the same order as
    /// [`wp_model::Document::paragraphs`], so it *is* the next paragraph's index.
    pub paragraphs: usize,
    /// Set while a note's own content is being flowed, so that a note holding
    /// a reference to another note cannot send the layout round for ever.
    pub in_note: bool,
    /// Something standing beside the flow that the next paragraphs must make
    /// room for — a floating table, which is the only thing that makes one.
    /// Its depth counts down as paragraphs are laid past it.
    pub obstacle: Option<inline::Obstacle>,
    /// Where a drop cap's baseline sits inside its own float, and which line
    /// of the paragraph that follows it must stand on. Word seats the capital
    /// on the baseline of the *body* line, not on its own descent, so the
    /// float is shifted once that line's baseline is known.
    pub floating_baseline: Option<(f64, usize)>,
    /// A floating table's own drawing, waiting for an item to ride with. It is
    /// not in the flow: nothing gives it height, and pagination has to move it
    /// with the text that wraps round it rather than on its own.
    pub floating: Vec<Placement>,
    /// Word's half-point accumulator: how far the laid lines lag their ideal,
    /// in points. See [`crate::shape::Pitch`]. It runs across paragraphs and
    /// through table rows; a fresh flow — a cell, a header band — starts at
    /// whatever its creator says, usually zero.
    pub drift: f64,
    /// Whether any half-point was actually paid. Only then can resetting the
    /// accumulator at page tops change anything a second pass would see.
    pub dumped: bool,
    /// Item indices at which the accumulator resets — the first item of every
    /// page. Empty on a first pass, because pages do not exist yet; filled for
    /// the second from where the first pass broke them.
    pub resets: Vec<usize>,
    /// The previous paragraph's space-after, in points.
    ///
    /// Word does not stack the gap between two paragraphs: the space between
    /// them is the *larger* of the first's space-after and the second's
    /// space-before, like CSS margins collapsing. Measured on
    /// file-sample_100kB.docx — 11.25pt after against 12pt before came out as
    /// exactly 12. The after has already been paid into the previous item's
    /// height, so the following paragraph pays only what its before exceeds it
    /// by.
    pub last_after: f64,
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
    let plain = layout_once(document, ctx, shaper);
    // A float anchored to the page or a margin sits where only pagination
    // knows, and the lines it narrows have to be broken before that. So the
    // document is laid out once, the floats are read off the pages, and the
    // paragraphs they stand beside are laid again — the same two-pass shape a
    // `{ PAGE }` field needs, for the same reason. Once only: a wrap that
    // moved the float that caused it would never settle.
    let wraps = Wraps::of(&plain);
    let beside = Context {
        wraps: &wraps,
        ..*ctx
    };
    let (ctx, first) = if wraps.is_empty() {
        (ctx, plain)
    } else {
        let again = layout_once(document, &beside, shaper);
        (&beside, again)
    };
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
    // Paragraph indices name positions in the whole document, so the count
    // runs across sections. A per-section flow that restarted at zero made
    // every line of a second section claim a paragraph from the first — and a
    // click in that section landed pages away.
    let mut flowed = 0usize;

    for (section_index, (range, section)) in document.sections().into_iter().enumerate() {
        if let Some(start) = section.page_numbering.start {
            number = start;
        }
        let width = section.text_width().points();
        let (top, bottom) = band_margins(document, section, ctx, shaper);
        let height = section.page.height.points() - top - bottom;
        let columns = section.columns.resolve(section.text_width());

        let entry_counters = counters.clone();
        let mut flow = Flow {
            paragraphs: flowed,
            ..Flow::default()
        };
        for block in &document.body[range.clone()] {
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
        let is_last_section = range.end >= document.body.len();
        if is_last_section {
            flow_endnotes(document, ctx, shaper, &mut counters, width, &mut flow);
        }
        let mut breaks = paginate(&flow.items, height);

        // Word restarts the half-point accumulator at every page top — the
        // same jump pattern repeats down every page of an unbroken run. Pages
        // are only known after pagination, so a flow that actually paid a
        // half-point somewhere is flowed once more with the resets in, and
        // paginated again. A document where the debt never came due pays for
        // one pass, exactly like the field pass above this one.
        if flow.dumped {
            let mut resets: Vec<usize> = Vec::with_capacity(breaks.len() + 1);
            resets.push(0);
            resets.extend(breaks.iter().copied());
            resets.dedup();
            counters = entry_counters;
            let mut second = Flow {
                resets,
                paragraphs: flowed,
                ..Flow::default()
            };
            for block in &document.body[range] {
                flow_block(
                    block,
                    document,
                    ctx,
                    shaper,
                    &mut counters,
                    width,
                    &mut second,
                );
            }
            if is_last_section {
                flow_endnotes(document, ctx, shaper, &mut counters, width, &mut second);
            }
            flow = second;
            breaks = paginate(&flow.items, height);
        }
        flowed = flow.paragraphs;
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

            // The notes referred to by the text just placed, set at the foot
            // of the page under the rule Word draws there.
            let referenced: Vec<i32> = slice
                .iter()
                .flat_map(|item| &item.footnotes)
                .map(|(id, _)| *id)
                .collect();
            if !referenced.is_empty() {
                place_notes(
                    &mut page,
                    &referenced,
                    document,
                    ctx,
                    shaper,
                    &mut counters,
                    width,
                );
            }

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

/// Sets a page's notes at its foot, under the rule that separates them.
///
/// The band grows upward from the bottom margin: Word fills the page with
/// text first and the notes take what is left, which is why pagination has
/// already made room for exactly this much.
fn place_notes(
    page: &mut Page,
    referenced: &[i32],
    document: &Document,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
    counters: &mut Counters,
    width: f64,
) {
    let mut flows = Vec::new();
    let mut total = 0.0;
    // The rule above the notes is a paragraph of the document's own, holding a
    // `<w:separator/>`. Laying it out rather than inventing one puts the rule
    // where Word puts it, indent and all.
    if let Some(note) = document
        .footnotes
        .iter()
        .find(|note| note.kind == wp_model::doc::NoteKind::Separator)
    {
        let flow = flow_note(note, document, ctx, shaper, counters, width);
        total += flow.items.iter().map(|item| item.height).sum::<f64>();
        flows.push(flow);
    }
    for id in referenced {
        let Some(note) = document.footnote(*id).filter(|n| n.kind.is_real()) else {
            continue;
        };
        let flow = flow_note(note, document, ctx, shaper, counters, width);
        total += flow.items.iter().map(|item| item.height).sum::<f64>();
        flows.push(flow);
    }
    if flows.is_empty() {
        return;
    }

    let bottom = page.geometry.height - page.geometry.bottom;
    let mut y = bottom - total;
    let mut first = true;
    for flow in flows {
        for item in flow.items {
            for part in item.parts {
                // The separator's own line draws the rule Word draws through
                // it — three points above its baseline, measured.
                if first {
                    if let Placed::Line { line, .. } = &part.kind {
                        page.footnotes.push(Placement {
                            x: page.geometry.start + part.x,
                            y: y + part.y + line.baseline - 3.0,
                            width: 0.0,
                            height: 0.0,
                            kind: Placed::FootnoteSeparator,
                        });
                        first = false;
                    }
                }
                page.footnotes.push(Placement {
                    x: page.geometry.start + part.x,
                    y: y + part.y,
                    ..part
                });
            }
            y += item.height;
        }
    }
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
            // A drop cap is a paragraph of its own that the paragraph after it
            // wraps around, which is the same thing a floating table is.
            let cap = document
                .styles
                .resolve_paragraph(&paragraph.props, None)
                .para
                .frame
                .is_some_and(|frame| frame.drop_cap.is_cap());
            if cap {
                flow_drop_cap(paragraph, document, ctx, shaper, counters, width, into);
            } else {
                flow_paragraph(paragraph, document, ctx, shaper, counters, width, 0.0, into)
            }
        }
        Block::Table(table) => {
            // A table does not collapse spacing with the text around it: the
            // paragraph above keeps its space-after whole, and the paragraph
            // below starts its space-before fresh.
            into.last_after = 0.0;
            if table.props.float.is_some() {
                flow_floating_table(table, document, ctx, shaper, counters, width, into);
            } else {
                flow_table(table, document, ctx, shaper, counters, width, into)
            }
        }
        Block::Structured(sdt) => {
            for inner in &sdt.content {
                flow_block(inner, document, ctx, shaper, counters, width, into);
            }
        }
        Block::Anchor(_) | Block::AltChunk { .. } => {}
    }
}

/// The large capital at the head of a section, which the text runs around.
///
/// Word states the whole of it in the paragraph holding the letter: an exact
/// line height as tall as the lines it displaces, and a frame that says how
/// many those are. So the letter is laid out on its own and then stood beside
/// the flow exactly as a floating table is — the paragraph that follows keeps
/// clear of it until its depth is used up.
fn flow_drop_cap(
    paragraph: &Paragraph,
    document: &Document,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
    counters: &mut Counters,
    width: f64,
    into: &mut Flow,
) {
    let lines = document
        .styles
        .resolve_paragraph(&paragraph.props, None)
        .para
        .frame
        .map(|frame| frame.lines as usize)
        .unwrap_or(0);
    let mut aside = Flow {
        paragraphs: into.paragraphs,
        ..Flow::default()
    };
    flow_paragraph(
        paragraph, document, ctx, shaper, counters, width, 0.0, &mut aside,
    );
    into.paragraphs = aside.paragraphs;

    let height: f64 = aside.items.iter().map(|item| item.height).sum();
    let extent = aside
        .items
        .iter()
        .flat_map(|item| &item.parts)
        .map(|part| part.x + part.width)
        .fold(0.0f64, f64::max);
    if height <= 0.0 || extent <= 0.0 {
        // Nothing to stand beside; better in the flow than lost.
        for item in aside.items {
            into.items.push(item);
        }
        return;
    }

    // Where the capital's own baseline falls inside the float, so the caller
    // can seat it on the right line of the paragraph that follows.
    let mut baseline = 0.0;
    let mut top = 0.0;
    for item in aside.items {
        for part in item.parts {
            if let Placed::Line { line, .. } = &part.kind {
                // Where the letter is actually *drawn*, which is the line's
                // baseline less whatever `w:position` raised the run by. Word
                // seats the drawn letter on the body line, so a capital that
                // its own paragraph lowers must not be lowered again.
                let raise = line
                    .fragments
                    .first()
                    .map(|fragment| fragment.style.raise)
                    .unwrap_or(0.0);
                baseline = top + part.y + line.baseline - raise;
            }
            into.floating.push(Placement {
                y: part.y + top,
                ..part
            });
        }
        top += item.height;
    }
    into.obstacle = Some(inline::Obstacle {
        depth: height,
        indent: extent,
        inset: 0.0,
    });
    into.floating_baseline = Some((baseline, lines.max(1)));
}

/// A table the text runs past rather than under.
///
/// `<w:tblpPr>` takes the table out of the flow and puts it at a place of its
/// own, and the paragraphs that follow are set in what measure is left beside
/// it until it is passed. Only the common case is built: anchored to the text,
/// against the left margin, with the text to its right. A float that Word
/// would put elsewhere is laid in the flow as an ordinary table, which is
/// where this reader put every one of them before.
fn flow_floating_table(
    table: &Table,
    document: &Document,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
    counters: &mut Counters,
    width: f64,
    into: &mut Flow,
) {
    let float = table.props.float.as_deref().copied().unwrap_or_default();
    // Anywhere but the left of the text column is not built; laying it in the
    // flow is wrong by less than putting it in the wrong place would be.
    let left = float.x.map(|t| t.points()).unwrap_or(0.0);
    if left > 0.5 {
        flow_table(table, document, ctx, shaper, counters, width, into);
        return;
    }

    // The table is laid out on its own so its size is known before anything is
    // set beside it.
    let mut aside = Flow {
        paragraphs: into.paragraphs,
        ..Flow::default()
    };
    flow_table(table, document, ctx, shaper, counters, width, &mut aside);
    into.paragraphs = aside.paragraphs;

    let height: f64 = aside.items.iter().map(|item| item.height).sum();
    let extent = aside
        .items
        .iter()
        .flat_map(|item| &item.parts)
        .map(|part| part.x + part.width)
        .fold(0.0f64, f64::max);
    let gap = float.right_from_text.map(|t| t.points()).unwrap_or(0.0);
    let below = float.bottom_from_text.map(|t| t.points()).unwrap_or(0.0);

    // Flattened to placements at the float's own position. Nothing here takes
    // height from the flow: the text beside it is what fills that space.
    let mut top = float.y.map(|t| t.points()).unwrap_or(0.0);
    for item in aside.items {
        for part in item.parts {
            into.floating.push(Placement {
                y: part.y + top,
                ..part
            });
        }
        top += item.height;
    }

    into.obstacle = Some(inline::Obstacle {
        depth: height + below,
        indent: extent + gap,
        inset: 0.0,
    });
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
    let layers =
        document
            .styles
            .resolve_paragraph_in(&paragraph.props, numbering.as_ref(), ctx.table_part);
    let label = reference.and_then(|r| {
        let text = counters.advance(&document.numbering, r)?;
        let level = document.numbering.level(r.num_id, r.level)?;
        let font = level.run.fonts.ascii.as_deref();
        Some(ListLabel {
            // A machine that has the symbol font itself draws Word's own
            // glyph; the translation is for the machine that does not.
            text: if font.is_some_and(|f| (ctx.has_face)(f)) {
                text
            } else {
                desymbol(&text, font)
            },
            props: level.run.clone(),
            suffix: level.suffix,
            bullet: level.format == wp_model::numbering::NumFormat::Bullet,
        })
    });

    // The notes this paragraph refers to are laid out and measured now: the
    // page they land on has that much less room for text, and pagination is
    // what decides which page that is.
    let notes = if into.in_note {
        Vec::new()
    } else {
        notes_referenced(paragraph, document, ctx, shaper, counters, width)
    };
    let index = into.paragraphs;
    // A picture standing at a margin narrows this paragraph as much as a
    // floating table would. Which paragraphs it reaches is knowledge from the
    // pass before this one — see [`Wraps`].
    if let Some(beside) = ctx.wraps.beside(index) {
        into.obstacle = Some(match into.obstacle {
            Some(already) => inline::Obstacle {
                depth: already.depth.max(beside.depth),
                indent: already.indent.max(beside.indent),
                inset: already.inset.max(beside.inset),
            },
            None => beside,
        });
    }
    let laid = inline::layout(
        paragraph,
        index,
        &layers,
        label.as_ref(),
        ctx,
        width,
        into.obstacle,
        shaper,
    );
    let first = into.items.len();
    push_paragraph(paragraph, &layers, laid, left, width, ctx.theme, into);
    // Attached to the paragraph's first item. Word attaches a note to the
    // *line* its mark sits on, so a paragraph split across a page break can
    // leave its note behind; matching that needs the mark's line, and this
    // does not have it. Stated as a limit: a note travels with the paragraph.
    if let Some(item) = into.items.get_mut(first) {
        item.footnotes = notes;
    }
}

/// Every footnote this paragraph refers to, with the height its content needs.
///
/// Endnotes are not here: they are collected at the end of the document rather
/// than at the foot of the page, so a reference to one costs the page nothing.
fn notes_referenced(
    paragraph: &Paragraph,
    document: &Document,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
    counters: &mut Counters,
    width: f64,
) -> Vec<(i32, f64)> {
    let mut out = Vec::new();
    for run in paragraph.runs() {
        for piece in &run.content {
            let Piece::FootnoteRef { id, .. } = piece else {
                continue;
            };
            if out.iter().any(|(seen, _)| seen == id) {
                continue;
            }
            let Some(note) = document.footnote(*id) else {
                continue;
            };
            if !note.kind.is_real() {
                continue;
            }
            out.push((
                *id,
                note_height(note, document, ctx, shaper, counters, width),
            ));
        }
    }
    out
}

/// How tall one note's own content is, laid out in the text column.
fn note_height(
    note: &wp_model::doc::Note,
    document: &Document,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
    counters: &mut Counters,
    width: f64,
) -> f64 {
    flow_note(note, document, ctx, shaper, counters, width)
        .items
        .iter()
        .map(|item| item.height)
        .sum()
}

/// The endnotes, set after the last of the body.
///
/// A footnote belongs to the page its mark landed on; an endnote belongs to
/// the end of the document, so it is simply more content at the end of the
/// last section and paginates like any other. Word puts the same rule above
/// them that it puts above a page's footnotes, and it is the separator entry
/// of `endnotes.xml` that draws it.
fn flow_endnotes(
    document: &Document,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
    counters: &mut Counters,
    width: f64,
    into: &mut Flow,
) {
    if !document.endnotes.iter().any(|note| note.kind.is_real()) {
        return;
    }
    // The separator entry first: it is an ordinary paragraph and its height is
    // the gap Word leaves between the body and the notes.
    for note in &document.endnotes {
        if note.kind != wp_model::doc::NoteKind::Separator {
            continue;
        }
        let mut flow = Flow {
            in_note: true,
            paragraphs: into.paragraphs,
            ..Flow::default()
        };
        for block in &note.content {
            flow_block(block, document, ctx, shaper, counters, width, &mut flow);
        }
        into.paragraphs = flow.paragraphs;
        // The same rule the foot of a page carries, drawn through the
        // separator's own line.
        if let Some(item) = flow.items.first_mut() {
            let seat = item.parts.iter().find_map(|part| match &part.kind {
                Placed::Line { line, .. } => Some((part.x, part.y + line.baseline - 3.0)),
                _ => None,
            });
            if let Some((x, y)) = seat {
                item.parts.push(Placement {
                    x,
                    y,
                    width: 0.0,
                    height: 0.0,
                    kind: Placed::FootnoteSeparator,
                });
            }
        }
        into.items.append(&mut flow.items);
        break;
    }
    for note in &document.endnotes {
        if !note.kind.is_real() {
            continue;
        }
        let mark = ctx.notes.mark(true, note.id).unwrap_or_default();
        let ctx = &Context {
            note_mark: Some(mark),
            ..*ctx
        };
        let mut flow = Flow {
            in_note: true,
            paragraphs: into.paragraphs,
            ..Flow::default()
        };
        for block in &note.content {
            flow_block(block, document, ctx, shaper, counters, width, &mut flow);
        }
        into.paragraphs = flow.paragraphs;
        into.items.append(&mut flow.items);
    }
}

/// A note's content as its own flow, which is how it is both measured and
/// placed.
fn flow_note(
    note: &wp_model::doc::Note,
    document: &Document,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
    counters: &mut Counters,
    width: f64,
) -> Flow {
    let mut flow = Flow {
        in_note: true,
        ..Flow::default()
    };
    // The note knows its own number only from where it sits in the list, so
    // it is told: `<w:footnoteRef/>` at its head draws exactly this.
    let mark = ctx.notes.mark(false, note.id).unwrap_or_default();
    let ctx = &Context {
        note_mark: Some(mark),
        ..*ctx
    };
    for block in &note.content {
        flow_block(block, document, ctx, shaper, counters, width, &mut flow);
    }
    flow
}

/// A bullet stated in a symbol font's private-use range, translated to the
/// character everyone's fonts can draw.
///
/// Word's classic bullet is `U+F0B7` *in the Symbol font* — not a Unicode
/// bullet but Symbol's own `0xB7`, parked in the private-use area. Word ships
/// Symbol and Wingdings, so it draws the real glyph; this renderer does not,
/// and without the translation every such bullet is a tofu box. The table maps
/// the codes Word's list galleries actually use to their Unicode equivalents,
/// and any other private-use character in a *label* falls back to a plain
/// bullet — a label is a bullet or a number, and a number is never in the PUA.
fn desymbol(text: &str, font: Option<&str>) -> String {
    if !text.chars().any(|c| ('\u{F000}'..='\u{F0FF}').contains(&c)) {
        return text.to_owned();
    }
    let font = font.map(str::to_ascii_lowercase);
    text.chars()
        .map(|c| {
            if !('\u{F000}'..='\u{F0FF}').contains(&c) {
                return c;
            }
            let code = (c as u32) & 0xFF;
            match (font.as_deref(), code) {
                (Some("symbol"), 0xB7) => '\u{2022}',    // •
                (Some("wingdings"), 0xA7) => '\u{25AA}', // ▪
                (Some("wingdings"), 0xFC) => '\u{2713}', // ✓
                (Some("wingdings"), 0xD8) => '\u{25BA}', // ► (Word draws ➢)
                (Some("wingdings"), 0x76) => '\u{2756}', // ❖
                _ => '\u{2022}',
            }
        })
        .collect()
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

/// How far a paragraph border stands off the text it surrounds.
///
/// Measured, not read: Word's own printed output over `w:sz` 2 to 48 and
/// `w:space` 0 to 20 puts the rule's inner edge exactly `space + 1.4pt` outside
/// the text column on the sides, and `space` plus the rule's own thickness
/// beyond the first and last line vertically. The 1.4 is Word's, has no name in
/// the format, and appears nowhere in the file.
const BORDER_STANDOFF: f64 = 1.4;

/// A border with its colour already turned into pixels.
///
/// A theme colour has to be resolved while the document's own theme is in
/// reach. A placement travels on to a printer and to a PDF writer that never
/// see the document, and resolving there means resolving against a *default*
/// theme — which draws this document's accent in some other document's blue.
fn themed(border: Border, theme: &wp_model::color::Theme) -> Border {
    Border {
        color: border.color.map(|c| match c.resolve(theme) {
            Some(rgb) => wp_model::color::Color::Rgb(rgb),
            // Word draws an automatic border black; the painters already
            // read a colour they cannot resolve that way.
            None => wp_model::color::Color::Auto,
        }),
        ..border
    }
}

/// The thickness a border draws at, in points. `w:sz` counts eighths.
fn border_thickness(border: &Border) -> f64 {
    border.size.map(|s| s.points()).unwrap_or(0.5)
}

/// The room a border takes above or below the text, thickness included.
fn border_depth(border: Option<&Border>) -> f64 {
    border
        .filter(|b| b.style.draws())
        .map(|b| f64::from(b.space.unwrap_or(0)) + border_thickness(b))
        .unwrap_or(0.0)
}

fn push_paragraph(
    paragraph: &Paragraph,
    layers: &Layers,
    laid: LaidParagraph,
    left: f64,
    width: f64,
    theme: &wp_model::color::Theme,
    into: &mut Flow,
) {
    // Named apart from the line loop's own `index` below, which shadowed this
    // one and gave every line the paragraph number *zero* — so every click
    // landed in the first paragraph of the document.
    let paragraph_index = into.paragraphs;

    // A float that displaces text reserves its height here, before the anchor
    // paragraph's first line: Word puts the picture at the top of the paragraph
    // it is anchored to and starts the paragraph below it. Measured on
    // file-sample_500kB.docx — image bottom 516.5, anchor line 543.0, following
    // heading 581.5 — and the reservation reproduces all three.
    //
    // An *empty* line directly above the float is pushed below it, and the
    // slot it vacated stays blank — Word renders the sample's preceding empty
    // paragraph at 516.5, exactly where the picture ends, with dead space
    // where the line would have been. A line with text stays where it is.
    let mut displaced: Vec<Item> = Vec::new();
    if anchored(paragraph)
        .iter()
        .any(|(_, drawing)| displaces(drawing))
    {
        while into.items.last().is_some_and(is_empty_line) {
            displaced.push(into.items.pop().expect("just checked"));
        }
        displaced.reverse();
    }
    let mut dead: f64 = displaced.iter().map(|item| item.height).sum();
    for (nth, drawing) in anchored(paragraph) {
        if !displaces(drawing) {
            continue;
        }
        let (dist_top, _, dist_bottom, _) = drawing.distance;
        let above = std::mem::take(&mut dead);
        into.items.push(Item {
            height: above + dist_top.points() + drawing.extent.1.points() + dist_bottom.points(),
            parts: vec![Placement {
                x: left,
                y: above + dist_top.points(),
                width: drawing.extent.0.points(),
                height: drawing.extent.1.points(),
                kind: Placed::Drawing {
                    rel: drawing.rel.clone(),
                    anchor: Some(Box::new(drawing.clone())),
                    paragraph: paragraph_index,
                    nth,
                },
            }],
            // Counted from the top so it cannot collide with a group already
            // assigned to an item put back below the float.
            group: usize::MAX - into.items.len(),
            index_in_group: 0,
            items_in_group: 1,
            // The picture must not sit at the bottom of one page with its
            // paragraph at the top of the next.
            keep_with_next: true,
            keep_lines: false,
            widow_control: false,
            break_before: false,
            repeat: false,
            table: None,
            footnotes: Vec::new(),
            slack: 0.0,
        });
    }
    into.items.append(&mut displaced);

    let group = into.items.len();
    into.paragraphs += 1;
    // The float stands beside this paragraph and whatever follows until its
    // depth is used up. What the paragraph consumes is taken off here, before
    // its lines are turned into items.
    let laid_height: f64 = laid.lines.iter().map(|line| line.height).sum();
    if let Some(obstacle) = &mut into.obstacle {
        obstacle.depth -= laid_height + laid.space_before;
        if obstacle.depth <= 0.01 {
            into.obstacle = None;
        }
    }
    let count = laid.lines.len().max(1);
    // Each line's height and where its baseline sits inside it, so a drop cap
    // riding with this paragraph can be seated on the right one.
    let lines_ahead: Vec<(f64, f64)> = laid
        .lines
        .iter()
        .map(|line| (line.height, line.baseline))
        .collect();
    let explicit_break = paragraph
        .runs()
        .iter()
        .flat_map(|run| &run.content)
        .any(|piece| matches!(piece, Piece::Break(Break::Page)));
    // See [`Flow::last_after`]: the gap between paragraphs is the larger of
    // the two spacings, and the previous one's share is already placed.
    let before = (laid.space_before - into.last_after).max(0.0);
    let after = laid.space_after;
    into.last_after = after;

    // Any other anchored drawing is placed on the page rather than in the line,
    // so it rides with the paragraph's first item and is positioned from there.
    let floats: Vec<Placement> = anchored(paragraph)
        .into_iter()
        .filter(|(_, drawing)| !displaces(drawing))
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

    // A paragraph border stands off the text by a measured amount and takes
    // that room from the page: every line below a bordered paragraph moves
    // down by the rule and its gap. See [`BORDER_STANDOFF`].
    let borders = layers.para.borders.as_deref();
    let drawn = |edge: Option<Border>| edge.filter(|b| b.style.draws()).map(|b| themed(b, theme));
    let (bdr_top, bdr_bottom, bdr_start, bdr_end) = match borders {
        Some(b) => (drawn(b.top), drawn(b.bottom), drawn(b.start), drawn(b.end)),
        None => (None, None, None, None),
    };
    let above = border_depth(bdr_top.as_ref());
    let below = border_depth(bdr_bottom.as_ref());
    // How far past the text column the box reaches on each side: the standoff
    // alone where there is no side rule, and the rule's own gap and thickness
    // where there is one.
    let reach = |edge: Option<&Border>| match edge {
        Some(b) => f64::from(b.space.unwrap_or(0)) + BORDER_STANDOFF + border_thickness(b),
        None => BORDER_STANDOFF,
    };
    let box_left = left - reach(bdr_start.as_ref());
    let box_right = left + width + reach(bdr_end.as_ref());
    // The thickness of each side rule, which shading stops short of.
    let start_rule = bdr_start.as_ref().map(border_thickness).unwrap_or(0.0);
    let end_rule = bdr_end.as_ref().map(border_thickness).unwrap_or(0.0);
    let shading = layers
        .para
        .shading
        .and_then(|s| s.background())
        .and_then(|c| c.resolve(theme));

    for (index, line) in laid.lines.into_iter().enumerate() {
        let mut line = line;
        // The half-point dance: lines are laid a hair off their exact height,
        // the debt accumulates, and the line that tips it half a point pays.
        // Measured from Word to the twip — thirty lines of Verdana at 12.083pt
        // with a 12.583pt line every seventh, averaging the design height.
        if into.resets.binary_search(&into.items.len()).is_ok() {
            into.drift = 0.0;
        }
        if line.ideal != line.height {
            into.drift += line.ideal - line.height;
            // The epsilon keeps a debt built from inexact tenths from missing
            // its own due date; no real font's drift sits on the knife edge.
            if into.drift >= 0.5 - 1e-9 {
                line.height += 0.5;
                into.drift -= 0.5;
                into.dumped = true;
            } else if into.drift <= -0.5 + 1e-9 {
                line.height -= 0.5;
                into.drift += 0.5;
                into.dumped = true;
            }
        }
        let is_first = index == 0;
        let is_last = index + 1 == count;
        let mut height = line.height;
        let mut top = 0.0;
        if is_first {
            height += before + above;
            top = before + above;
        }
        if is_last {
            height += after + below;
        }
        let ends_page = line.ended_by == Some(Break::Page);
        let x = left + line.x;
        let mut parts = floats.take().unwrap_or_default();
        // A floating table rides with the first line set beside it, so that
        // pagination moves the two together rather than leaving the table on
        // one page and its text on the next.
        if is_first && !into.floating.is_empty() {
            let carried = std::mem::take(&mut into.floating);
            // A drop cap stands on a line of *this* paragraph rather than on
            // its own descent, so it is dropped by whatever the two differ by.
            let seat = into
                .floating_baseline
                .take()
                .and_then(|(baseline, nth)| {
                    let mut y = 0.0;
                    for (index, line) in lines_ahead.iter().enumerate() {
                        if index + 1 == nth {
                            return Some(y + line.1 - baseline);
                        }
                        y += line.0;
                    }
                    None
                })
                .unwrap_or(0.0);
            parts.extend(carried.into_iter().map(|part| Placement {
                x: left + part.x,
                y: top + part.y + seat,
                ..part
            }));
        }

        // Each rule is placed as the named edge of a flattened box, which is
        // what [`Placed::Edge`] strokes down the middle of — so the box edge
        // sits half a thickness inside the gap the border was given.
        let line_top = top;
        let line_bottom = top + line.height;
        // A paragraph's shading fills the *inside* of its border box, line by
        // line — Word paints one rectangle per line, from the left standoff to
        // the right one, and lets the border rules sit outside it.
        if let Some(rgb) = shading {
            let fill_top = if is_first { line_top - above } else { line_top };
            let fill_bottom = if is_last {
                line_bottom + below
            } else {
                line_bottom
            };
            parts.push(Placement {
                x: box_left + start_rule,
                y: fill_top,
                width: (box_right - end_rule) - (box_left + start_rule),
                height: fill_bottom - fill_top,
                kind: Placed::Fill(rgb),
            });
        }
        if is_first {
            if let Some(border) = bdr_top {
                let gap = f64::from(border.space.unwrap_or(0)) + border_thickness(&border) / 2.0;
                parts.push(Placement {
                    x: box_left,
                    y: line_top - gap,
                    width: box_right - box_left,
                    height: 0.0,
                    kind: Placed::Edge {
                        border,
                        side: Side::Top,
                    },
                });
            }
        }
        if is_last {
            if let Some(border) = bdr_bottom {
                let gap = f64::from(border.space.unwrap_or(0)) + border_thickness(&border) / 2.0;
                parts.push(Placement {
                    x: box_left,
                    y: line_bottom + gap,
                    width: box_right - box_left,
                    height: 0.0,
                    kind: Placed::Edge {
                        border,
                        side: Side::Bottom,
                    },
                });
            }
        }
        // The sides run the whole height of every line, so that a bordered
        // paragraph of many lines is boxed rather than striped.
        for (edge, side) in [(bdr_start, Side::Start), (bdr_end, Side::End)] {
            let Some(border) = edge else { continue };
            let inset = f64::from(border.space.unwrap_or(0))
                + BORDER_STANDOFF
                + border_thickness(&border) / 2.0;
            let at = match side {
                Side::Start => left - inset,
                _ => left + width + inset,
            };
            parts.push(Placement {
                x: at,
                y: if is_first { line_top - above } else { line_top },
                width: 0.0,
                height: line.height
                    + if is_first { above } else { 0.0 }
                    + if is_last { below } else { 0.0 },
                kind: Placed::Edge { border, side },
            });
        }

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
            // The space-after may sink into the bottom margin rather than
            // pushing this line to the next page.
            slack: if is_last { after } else { 0.0 },
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
                slack: 0.0,
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
            slack: 0.0,
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
        // A percentage is a *preferred* width, and the automatic layout a
        // table gets unless it says `fixed` sizes from the grid instead.
        // Measured across the demonstration document: a table asking for 70%
        // of the column is drawn at its grid's 71.6%, and the nested one
        // asking for 80% is drawn at its grid's 27.9%. Where the grid is the
        // wider of the two the clamp above still holds it to the column.
        Width::Percent(pct)
            if pct.0 > 0 && table.props.layout == wp_model::table::TableLayout::Fixed =>
        {
            pct.of(Twips::from_points(available)).points()
        }
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
    // Not simply `w:tblInd`: an indent that is stated at all is measured to
    // the text inside the first cell, so the table's edge hangs left of the
    // margin by the cell's own padding. See `resolve_table_indent`.
    let indent = document
        .styles
        .resolve_table_indent(&table.props, Twips::from_points(available));
    // What a table has to lay its columns in, which is *not* the text column.
    // Its cell text spans the column and its own edges hang outside it by the
    // cells' padding, so a grid wider than the column by exactly that much
    // fits and must not be squeezed into it. Measured: Word lays the
    // demonstration document's widest table at its grid's 478.8pt in a 468pt
    // column, and asked over COM for its columns answers with the grid to the
    // twip.
    let room = (available - indent + pad_end).max(1.0);
    let widths = column_widths(table, room);
    // `<w:jc>` on a table moves the whole table within the text column rather
    // than the text within its cells. A centred table is what the
    // demonstration document's nested one is, and without this it sat against
    // the left margin sixty-six points from where Word draws it.
    // A table placed by its justification is measured from the text column and
    // not from the indent: Word centres the demonstration document's nested
    // table on 306, the middle of the column, with no sign of the hang that
    // `w:tblInd` otherwise gives it.
    let indent = match table.props.justify {
        Some(wp_model::prop::Justify::Center) => {
            let laid: f64 = column_widths(table, room).iter().sum();
            ((available - laid) / 2.0).max(0.0)
        }
        Some(wp_model::prop::Justify::End) => {
            let laid: f64 = column_widths(table, room).iter().sum();
            (available - laid).max(0.0)
        }
        _ => indent,
    };
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

    let row_count = table.rows.len();
    // What a vertically merged cell still has to be given room for.
    //
    // A cell that spans rows is not a reason for the *first* of them to be
    // tall enough to hold all of it: Word gives each row the height its own
    // cells need and lets the merged text run on down through them, and only
    // the last row of the span grows to cover whatever is left. Measured
    // against the two-by-two nested table of the demonstration document, whose
    // merged cell holds two paragraphs: Word puts the second beside the second
    // row's own text, where charging the first row for both put it a whole row
    // lower and moved everything below the table with it.
    let mut owed: Vec<Owed> = Vec::new();
    // The bottom rule of the row before this one, which the two rows share.
    let mut rule_from_above: f64 = 0.0;
    for (row_index, row) in table.rows.iter().enumerate() {
        let is_last_row = row_index + 1 == row_count;
        let mut cells: Vec<CellPlan> = Vec::new();
        // Each cell's accumulator state on the way out, so the flow after the
        // row can continue from the cell that decided the row's height.
        let mut exits: Vec<(f64, bool)> = Vec::new();
        let mut column = row.props.grid_before;
        let mut x = indent
            + widths
                .iter()
                .take(row.props.grid_before as usize)
                .sum::<f64>();

        for cell in &row.cells {
            let span = cell.props.span();
            // What the table's style says about a cell in this position: the
            // header row's fill, the stripe's rules, the doubled line above a
            // total. Direct properties are laid over it below.
            let styled = document.styles.resolve_table_cell(
                &table.props,
                wp_model::banding::CellAt {
                    row: row_index,
                    rows: row_count,
                    column: column as usize,
                    columns: widths.len(),
                },
            );
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
                // A cell's accumulator starts at half a quantum. Measured: a
                // thirty-line cell pays its first half-point on line four,
                // which places the entry debt at a quarter point — where a
                // fresh flow's would take eight lines to come due.
                drift: 0.25,
                ..Flow::default()
            };
            if !is_continuation {
                // Everything in the cell is laid out knowing which part of the
                // table style covers it, so a header row's text is the header
                // row's colour without the cell having to say so.
                let ctx = &Context {
                    table_part: Some(&styled),
                    ..*ctx
                };
                for (at, block) in cell.content.iter().enumerate() {
                    // The paragraph a cell must end with after a table is
                    // punctuation, not a line: the format forbids a cell
                    // ending in a table, so Word writes an empty paragraph
                    // there and gives it no height whatever — measured, its
                    // spacing before included. It is still a paragraph the
                    // document counts, so the numbering steps over it.
                    if closes_a_cell(&cell.content, at) {
                        cell_flow.paragraphs += 1;
                        continue;
                    }
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

            exits.push((cell_flow.drift, cell_flow.dumped));
            cells.push(CellPlan {
                x,
                width: cell_width,
                align: match cell.props.v_align {
                    CellVAlign::Top => styled.cell_v_align.unwrap_or(CellVAlign::Top),
                    stated => stated,
                },
                // The cell's own fill, then the table's, then whatever the
                // style gives a cell in this position — the header's green,
                // the stripe's pale blue.
                fill: cell
                    .props
                    .shading
                    .or(table.props.shading)
                    .or(styled.cell_shading)
                    .and_then(|s| s.background())
                    .and_then(|c| c.resolve(&document.theme)),
                borders: {
                    // A cell on the outside of the table takes the outer rule;
                    // one inside takes the rule that runs between. Most
                    // specific first, and direct formatting ahead of the style
                    // at each level.
                    let outer_top = row_index == 0;
                    let outer_bottom = is_last_row;
                    let outer_start = column == 0;
                    let outer_end = column + span >= widths.len() as u32;
                    let pick = |side: Side| -> Option<Border> {
                        let (direct_cell, direct_table, style_cell, style_table) = match side {
                            Side::Top => (
                                cell.props.borders.top,
                                if outer_top {
                                    table.props.borders.top
                                } else {
                                    table.props.borders.inside_h
                                },
                                styled.cell_borders.top,
                                if outer_top {
                                    styled.borders.top
                                } else {
                                    styled.borders.inside_h
                                },
                            ),
                            Side::Bottom => (
                                cell.props.borders.bottom,
                                if outer_bottom {
                                    table.props.borders.bottom
                                } else {
                                    table.props.borders.inside_h
                                },
                                styled.cell_borders.bottom,
                                if outer_bottom {
                                    styled.borders.bottom
                                } else {
                                    styled.borders.inside_h
                                },
                            ),
                            Side::Start => (
                                cell.props.borders.start,
                                if outer_start {
                                    table.props.borders.start
                                } else {
                                    table.props.borders.inside_v
                                },
                                styled.cell_borders.start,
                                if outer_start {
                                    styled.borders.start
                                } else {
                                    styled.borders.inside_v
                                },
                            ),
                            Side::End => (
                                cell.props.borders.end,
                                if outer_end {
                                    table.props.borders.end
                                } else {
                                    table.props.borders.inside_v
                                },
                                styled.cell_borders.end,
                                if outer_end {
                                    styled.borders.end
                                } else {
                                    styled.borders.inside_v
                                },
                            ),
                        };
                        direct_cell.or(direct_table).or(style_cell).or(style_table)
                    };
                    [Side::Top, Side::Start, Side::Bottom, Side::End]
                        .map(|side| (side, pick(side).map(|b| themed(b, &document.theme))))
                },
                content: y,
                spans: table.merge_height(row_index, column),
                lines,
            });

            x += cell_width;
            column += span;
        }

        let tallest = cells
            .iter()
            .filter(|cell| cell.spans <= 1)
            .map(|c| c.content)
            .fold(0.0f64, f64::max);
        // A span that ends here is what this row must finally cover.
        let tallest = owed
            .iter()
            .filter(|debt| debt.last == row_index)
            .fold(tallest, |tallest, debt| tallest.max(debt.remaining));
        // The body's own accumulator is not advanced by the row — each cell
        // ran its own, from the quarter-point entry. Only the fact that a
        // half-point was paid somewhere matters to the page-reset pass.
        into.dumped |= exits.iter().any(|(_, dumped)| *dumped);
        // Word's horizontal rules occupy their thickness: a row is taller by
        // the rule above it and its content starts below the rule. Measured:
        // a 2pt-bordered table starts its text 2pt lower and pitches every
        // row 2pt taller than the borderless twin.
        let rule = |side: Side| -> f64 {
            cells
                .iter()
                .flat_map(|cell| &cell.borders)
                .filter(|(s, _)| *s == side)
                .filter_map(|(_, border)| border.filter(|b| b.style.draws()))
                .map(|border| border.size.map(|s| s.points()).unwrap_or(0.5))
                .fold(0.0f64, f64::max)
        };
        // The rule between two rows is one line and is paid for once, by the
        // row below it, and it is as thick as the heavier of the two edges
        // that meet there. A header row whose style rules three points under
        // it and nothing above the row that follows is three points of height
        // that belongs to nobody unless the row below claims it — measured on
        // the demonstration document, where every row after the header sat
        // three points too high. A calendar whose rows rule a hairline under
        // one and over the next must not pay for it twice.
        let rule_above = rule(Side::Top).max(rule_from_above);
        let rule_below = if is_last_row { rule(Side::Bottom) } else { 0.0 };
        rule_from_above = rule(Side::Bottom);
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
        for cell in cells.iter().filter(|cell| cell.spans > 1) {
            owed.push(Owed {
                remaining: cell.content,
                last: row_index + cell.spans - 1,
            });
        }
        for debt in &mut owed {
            debt.remaining -= inner_height;
        }
        owed.retain(|debt| debt.last > row_index && debt.remaining > 0.0);
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
                + if is_first { rule_above + pad_top } else { 0.0 }
                + if is_last {
                    pad_bottom + rule_below
                } else {
                    0.0
                };
            let above = if is_first { rule_above + pad_top } else { 0.0 };
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
                    // A cell that spans rows keeps its lines here, in the row
                    // that starts the merge, and draws the ones past this
                    // row's own height over the rows below — which is where
                    // they belong, because between them there is no rule and
                    // no cell edge, only the one tall cell a reader sees.
                    let runs_on = is_last && cell.spans > 1;
                    if line.top < top - EPSILON || (!runs_on && line.top >= bottom - EPSILON) {
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
                slack: 0.0,
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
    /// How many rows this cell covers, a vertical merge counted from the cell
    /// that starts it. One for every ordinary cell.
    spans: usize,
    lines: Vec<CellLine>,
}

/// Whether this block is the empty paragraph a cell has to end a table with.
///
/// Word draws nothing for it — not the line, not the spacing before it — and
/// a cell whose table is followed by one is exactly as tall as the table. Put
/// a single letter in it and the row grows by a whole line, so it is emptiness
/// and position together that make it disappear, not the paragraph itself.
fn closes_a_cell(content: &[Block], at: usize) -> bool {
    at > 0
        && at + 1 == content.len()
        && matches!(content[at - 1], Block::Table(_))
        && matches!(&content[at], Block::Paragraph(p) if p.is_empty())
}

/// A vertically merged cell's content, and where it must have run out.
struct Owed {
    /// How much of the cell's height the rows so far have not covered.
    remaining: f64,
    /// The last row of the span, which is the one that has to make it up.
    last: usize,
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
        // The foot of the page belongs to whatever notes the text on it
        // refers to, so every line that carries one takes its own height and
        // the note's out of the page at once.
        let mut notes = 0.0;
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
            let mut wants = notes;
            for (_, note) in &item.footnotes {
                if wants == 0.0 {
                    wants += SEPARATOR_LINES * item.height;
                }
                wants += note;
            }
            if y + item.height - item.slack + wants > height + 0.01 && end > start {
                break;
            }
            y += item.height;
            notes = wants;
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

/// How much of the page the rule above the notes costs, as a multiple of the
/// line the reference sits on.
///
/// Measured on the demonstration document, whose body is set 12pt on a 15.86pt
/// line: the last body baseline sits at 647.02 and the separator rule at
/// 680.38, with the notes running to a text bottom of 720. The separator is a
/// paragraph of the body's own size and Word keeps a second such line clear
/// above it — two lines, not one, which is why a page that reserved a single
/// line fitted one line of text too many. Expressed against the line rather
/// than in points so that a document set in some other size keeps the
/// proportion.
const SEPARATOR_LINES: f64 = 2.0;

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
/// Which paragraphs a float that does not travel with the text stands beside.
///
/// A picture anchored to the page or to a margin is placed by the page's own
/// geometry, so where it sits — and therefore which lines it narrows — is not
/// known until the document has been paginated once. This is what that first
/// pass learned, keyed by the paragraph the float is anchored to; the flow
/// carries the obstacle on from there into whatever follows, exactly as it
/// does for a floating table.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Wraps {
    beside: std::collections::HashMap<usize, inline::Obstacle>,
}

impl Wraps {
    /// Reads the floats off a laid-out document.
    ///
    /// A float narrows whatever *lands beside it*, not whatever it is anchored
    /// to. The demonstration document proves the difference: its two arrows
    /// stand at the two margins of one page, and the right-hand one is
    /// anchored to a paragraph five lines below the text it narrows. So the
    /// float is taken as a rectangle on the page and every paragraph that
    /// starts inside its band is told about it.
    pub fn of(pages: &[Page]) -> Wraps {
        let mut wraps = Wraps::default();
        for page in pages {
            let tops = paragraph_tops(page);
            for placement in &page.content {
                let Placed::Drawing {
                    anchor: Some(drawing),
                    ..
                } = &placement.kind
                else {
                    continue;
                };
                if !stands_aside(drawing) {
                    continue;
                }
                let (x, y) = anchor_position(drawing, &page.geometry, placement.y);
                let (above, left, below, right) = drawing.distance;
                let width = drawing.extent.0.points();
                let top = y - above.points();
                let bottom = y + drawing.extent.1.points() + below.points();
                let start = page.geometry.start;
                let end = page.geometry.width - page.geometry.end;
                // Which margin it stands at, and so which side of the measure
                // it takes. A float in the middle of the column would leave
                // text on both sides of it, which is not modelled: the wider
                // side keeps the text.
                let (indent, inset) = if x - start <= end - (x + width) {
                    ((x + width + right.points() - start).max(0.0), 0.0)
                } else {
                    (0.0, (end - x + left.points()).max(0.0))
                };
                for &(paragraph, paragraph_top) in &tops {
                    // A paragraph that began above the float keeps its full
                    // measure: the obstacle is depth from a paragraph's top,
                    // and it has no way to say "from the fourth line down".
                    // Stated rather than approximated.
                    if paragraph_top < top - 0.01 || paragraph_top >= bottom - 0.01 {
                        continue;
                    }
                    wraps.add(
                        paragraph,
                        inline::Obstacle {
                            depth: bottom - paragraph_top,
                            indent,
                            inset,
                        },
                    );
                }
            }
        }
        wraps
    }

    /// What narrows this paragraph, if anything does.
    pub fn beside(&self, paragraph: usize) -> Option<inline::Obstacle> {
        self.beside.get(&paragraph).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.beside.is_empty()
    }

    /// A page may hold one picture at each margin, and the text between them
    /// is narrowed by both.
    fn add(&mut self, paragraph: usize, beside: inline::Obstacle) {
        let slot = self.beside.entry(paragraph).or_default();
        slot.depth = slot.depth.max(beside.depth);
        slot.indent = slot.indent.max(beside.indent);
        slot.inset = slot.inset.max(beside.inset);
    }
}

/// Whether text is set *beside* this drawing rather than below it.
///
/// A float that travels with the text reserves its height instead — see
/// [`displaces`], which is the other half of this decision and the stated
/// limit above. This is the other case: anchored to the page or to a margin,
/// where it stays put and the text has to go round.
fn stands_aside(drawing: &wp_model::Drawing) -> bool {
    use wp_model::doc::Wrap;
    if drawing.behind_text || displaces(drawing) {
        return false;
    }
    matches!(drawing.wrap, Wrap::Square | Wrap::Tight)
}

/// Where each paragraph on the page starts, in the order they were placed.
fn paragraph_tops(page: &Page) -> Vec<(usize, f64)> {
    let mut tops: Vec<(usize, f64)> = Vec::new();
    for placement in &page.content {
        let Placed::Line { paragraph, .. } = &placement.kind else {
            continue;
        };
        match tops.iter_mut().find(|(which, _)| which == paragraph) {
            Some((_, top)) => *top = top.min(placement.y),
            None => tops.push((*paragraph, placement.y)),
        }
    }
    tops
}

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
                RelativeTo::Margin
                | RelativeTo::TopMargin
                | RelativeTo::BottomMargin
                | RelativeTo::InsideMargin
                | RelativeTo::OutsideMargin => (page.top, page.height - page.bottom),
                // Relative to the paragraph or the line there is no band to
                // align within, only the place the text is: `top` means "at
                // the paragraph", and centre and bottom collapse to the same
                // spot rather than to the page margins.
                _ => (line_top, line_top + height),
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

/// Whether an anchored drawing takes its height out of the text flow.
///
/// Word wraps text *beside* a square-wrapped float when half an inch of
/// usable measure remains on a side; setting text beside a float is still a
/// stated limit here, so every text-anchored square float displaces instead
/// and the text resumes below it. That is Word's own behaviour for the
/// commonest float in the wild — the column-wide or centred picture, which
/// leaves no side worth setting into — and for the rest it is the honest
/// reading of the limit: a float that stopped reserving its height would sit
/// *under* the text, which is how shrinking a picture once put the words on
/// top of it. A float positioned relative to the page or a margin does not
/// travel with the text, so its space cannot be reserved mid-flow and it
/// stays an overlay.
fn displaces(drawing: &wp_model::Drawing) -> bool {
    use wp_model::doc::{RelativeTo, Wrap};
    let with_text = match &drawing.position {
        None => true,
        Some(position) => matches!(
            position.vertical.relative_to,
            RelativeTo::Paragraph | RelativeTo::Line | RelativeTo::Character | RelativeTo::Column
        ),
    };
    if !with_text {
        return false;
    }
    match drawing.wrap {
        Wrap::TopAndBottom | Wrap::Square | Wrap::Tight => true,
        Wrap::None => false,
    }
}

/// Whether an item is a single line with nothing on it — an empty paragraph.
fn is_empty_line(item: &Item) -> bool {
    item.parts.len() == 1
        && !item.break_before
        && matches!(&item.parts[0].kind,
            Placed::Line { line, .. } if line.fragments.is_empty())
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
            styles: Box::leak(Box::new(wp_model::style::StyleTable::default())),
            notes: Box::leak(Box::new(crate::notes::NoteMarks::default())),
            note_mark: None,
            table_part: None,
            default_tab: Twips(720),
            fallback_font: "test",
            has_face: |_| false,
            show_revisions: true,
            show_hidden: false,
            fields: Box::leak(Box::new(crate::field::FieldValues::default())),
            band: None,
            wraps: Box::leak(Box::new(Wraps::default())),
        }
    }

    fn pages(document: &Document) -> Vec<Page> {
        let theme = document.theme.clone();
        let mut shaper = crate::shape::Fixed;
        layout(document, &ctx(&theme), &mut shaper)
    }

    /// The fixed shaper with Word's half-point dance switched on: lines laid a
    /// tenth of a point short of their ideal, so the debt comes due every
    /// fifth line.
    struct Danced;

    impl Shaper for Danced {
        fn metrics(&mut self, font: &crate::shape::FontRequest) -> crate::shape::Metrics {
            crate::shape::Fixed.metrics(font)
        }

        fn advances(
            &mut self,
            text: &str,
            font: &crate::shape::FontRequest,
            into: &mut Vec<crate::shape::Advance>,
        ) {
            crate::shape::Fixed.advances(text, font, into)
        }

        fn pitch(&mut self, font: &crate::shape::FontRequest) -> crate::shape::Pitch {
            crate::shape::Pitch {
                base: font.size - 0.1,
                ideal: font.size,
            }
        }
    }

    #[test]
    fn the_half_point_debt_is_paid_by_the_line_that_tips_it() {
        // Ten-point lines laid at 9.9: drift reaches 0.5 on the fifth line,
        // which is laid at 10.4, and again on the tenth.
        let mut document = document(paragraphs(12));
        document.section = page_of(30);
        let theme = document.theme.clone();
        let mut shaper = Danced;
        let pages = layout(&document, &ctx(&theme), &mut shaper);
        let ys: Vec<f64> = pages[0]
            .content
            .iter()
            .filter(|p| matches!(p.kind, Placed::Line { .. }))
            .map(|p| p.y - pages[0].geometry.top)
            .collect();
        assert!(
            (ys[1] - 9.9).abs() < 1e-9,
            "line two sits at one short pitch"
        );
        assert!(
            (ys[5] - (4.0 * 9.9 + 10.4)).abs() < 1e-9,
            "the fifth line paid the half point: {ys:?}"
        );
        assert!(
            (ys[10] - (8.0 * 9.9 + 2.0 * 10.4)).abs() < 1e-9,
            "and the tenth paid again: {ys:?}"
        );
    }

    #[test]
    fn the_dance_restarts_at_the_top_of_every_page() {
        // Seven lines per page: page one pays on its fifth line. If the
        // accumulator carried over, page two would pay on its third; Word
        // starts every page from zero, so it pays on its fifth as well.
        let mut document = document(paragraphs(14));
        document.section = page_of(7);
        let theme = document.theme.clone();
        let mut shaper = Danced;
        let pages = layout(&document, &ctx(&theme), &mut shaper);
        assert!(pages.len() >= 2);
        for page in pages.iter().take(2) {
            let ys: Vec<f64> = page
                .content
                .iter()
                .filter(|p| matches!(p.kind, Placed::Line { .. }))
                .map(|p| p.y - page.geometry.top)
                .collect();
            assert!(
                (ys[5] - (4.0 * 9.9 + 10.4)).abs() < 1e-9,
                "each page pays on its own fifth line: {ys:?}"
            );
        }
    }

    #[test]
    fn a_tables_horizontal_rules_occupy_their_thickness() {
        // A half-point border: the first line starts half a point lower and
        // every row is half a point taller than the borderless twin. Measured
        // from Word: a 2pt-bordered table shifts its text down 2pt exactly.
        use wp_model::prop::{Border, BorderStyle};
        use wp_model::units::Eighth;
        let bordered = || {
            let mut table = Table {
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
            let rule = Border {
                style: BorderStyle::Single,
                size: Some(Eighth(4)),
                space: None,
                color: None,
                shadow: false,
            };
            table.props.borders.top = Some(rule);
            table.props.borders.inside_h = Some(rule);
            table.props.borders.bottom = Some(rule);
            table
        };
        let mut plain = document(vec![Block::Table(Table {
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
        })]);
        plain.section = page_of(20);
        let mut boxed = document(vec![Block::Table(bordered())]);
        boxed.section = page_of(20);

        let tops = |document: &Document| -> Vec<f64> {
            let page = &pages(document)[0];
            page.content
                .iter()
                .filter(|p| matches!(p.kind, Placed::Line { .. }))
                .map(|p| p.y - page.geometry.top)
                .collect()
        };
        let plain_tops = tops(&plain);
        let boxed_tops = tops(&boxed);
        assert!(
            (boxed_tops[0] - (plain_tops[0] + 0.5)).abs() < 1e-9,
            "the top rule displaces the first line: {boxed_tops:?} vs {plain_tops:?}"
        );
        assert!(
            (boxed_tops[1] - (plain_tops[1] + 1.0)).abs() < 1e-9,
            "the rule between the rows displaces the second again"
        );
    }

    #[test]
    fn borders_paint_after_every_fill_so_shading_cannot_eat_a_rule() {
        // The corpus sample's table shades every cell white and rules every
        // row boundary with a quarter-point hairline. In document order the
        // next row's fill begins exactly where this row's bottom rule was just
        // drawn, and whichever the rasterizer rounds wider wins: on the screen
        // three row rules vanished; in the PDF a column rule did. Word never
        // rolls that die — shading is painted below borders, always.
        use wp_model::color::Color;
        use wp_model::prop::{Border, BorderStyle, Shading};
        use wp_model::units::Eighth;
        let rule = Border {
            style: BorderStyle::Single,
            size: Some(Eighth(2)),
            space: None,
            color: None,
            shadow: false,
        };
        let white = Shading {
            fill: Some(Color::Rgb([255, 255, 255])),
            ..Shading::default()
        };
        let row = || {
            let mut cell = cell("a");
            cell.props.shading = Some(white);
            cell.props.borders.top = Some(rule);
            cell.props.borders.bottom = Some(rule);
            cell.props.borders.start = Some(rule);
            Row {
                cells: vec![cell],
                ..Row::new()
            }
        };
        let mut document = document(vec![Block::Table(Table {
            grid: vec![Twips(1440)],
            rows: vec![row(), row(), row()],
            ..Table::new()
        })]);
        document.section = page_of(20);
        let page = &pages(&document)[0];
        let order: Vec<u8> = page
            .painted()
            .filter_map(|p| match p.kind {
                Placed::Fill(_) => Some(0),
                Placed::Edge { .. } => Some(1),
                _ => None,
            })
            .collect();
        assert!(order.contains(&0) && order.contains(&1));
        assert!(
            order.windows(2).all(|w| w[0] <= w[1]),
            "every fill before every edge: {order:?}"
        );
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
        // With no table style to carry Word's own cell margins, a bare table
        // has none — the text meets the cell edge, as Word draws it. The
        // second column follows an inch across.
        assert_eq!(xs[0], page.geometry.start);
        assert_eq!(xs[1], page.geometry.start + 72.0);
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

    /// Where the lines of every cell in a table sit, top first.
    fn cell_lines(document: &Document) -> Vec<(f64, String)> {
        let mut out = Vec::new();
        for page in pages(document) {
            for placement in &page.content {
                if let Placed::Line { line, .. } = &placement.kind {
                    let text: String = line
                        .fragments
                        .iter()
                        .filter_map(|f| match &f.content {
                            crate::inline::Content::Text { text, .. } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect();
                    if !text.is_empty() {
                        out.push((placement.y, text));
                    }
                }
            }
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }

    #[test]
    fn a_merged_cell_runs_down_the_rows_it_spans_rather_than_swelling_the_first() {
        // Word gives each row the height its own cells need and lets the
        // merged text run on down through them: the demonstration document's
        // two-by-two nested table draws "Three" beside "Four", not a row
        // below it. See the note on `Owed`.
        let mut merged = Cell {
            props: CellProps::new(),
            content: vec![
                Block::Paragraph(Paragraph::of("one")),
                Block::Paragraph(Paragraph::of("three")),
            ],
        };
        merged.props.v_merge = Some(VMerge::Restart);
        let mut below = cell("");
        below.props.v_merge = Some(VMerge::Continue);
        let table = Table {
            grid: vec![Twips(2880), Twips(2880)],
            rows: vec![
                Row {
                    cells: vec![merged, cell("two")],
                    ..Row::new()
                },
                Row {
                    cells: vec![below, cell("four")],
                    ..Row::new()
                },
            ],
            ..Table::new()
        };
        let document = document(vec![Block::Table(table)]);
        let lines = cell_lines(&document);
        let at = |want: &str| {
            lines
                .iter()
                .find(|(_, text)| text == want)
                .map(|(y, _)| *y)
                .unwrap_or_else(|| panic!("{want} was never drawn: {lines:?}"))
        };
        assert_eq!(at("one"), at("two"), "the first row holds both firsts");
        assert_eq!(
            at("three"),
            at("four"),
            "and the merged cell's second paragraph is beside the second row"
        );
        assert!(at("three") > at("one"));
    }

    #[test]
    fn the_paragraph_a_cell_must_end_a_table_with_takes_no_height() {
        // The format forbids a cell ending in a table, so Word writes an empty
        // paragraph after it and gives it no height at all — spacing before
        // included. Measured: put one letter in it and the row grows by a
        // whole line.
        let nested = Table {
            grid: vec![Twips(1440)],
            rows: vec![Row {
                cells: vec![cell("inner")],
                ..Row::new()
            }],
            ..Table::new()
        };
        let closing = |empty: bool| {
            let mut mark = Paragraph::new();
            if !empty {
                mark = Paragraph::of("x");
            }
            let outer = Table {
                grid: vec![Twips(2880)],
                rows: vec![Row {
                    cells: vec![Cell {
                        props: CellProps::new(),
                        content: vec![Block::Table(nested.clone()), Block::Paragraph(mark)],
                    }],
                    ..Row::new()
                }],
                ..Table::new()
            };
            let document = document(vec![
                Block::Table(outer),
                Block::Paragraph(Paragraph::of("after")),
            ]);
            let lines = cell_lines(&document);
            lines
                .iter()
                .find(|(_, text)| text == "after")
                .map(|(y, _)| *y)
                .expect("the paragraph after the table")
        };
        assert!(
            closing(false) > closing(true),
            "an empty closing paragraph costs the cell nothing, a filled one a line"
        );
    }

    #[test]
    fn the_rule_between_two_rows_is_paid_for_once() {
        // A header row that rules three points under it and a row below that
        // rules nothing above it share one line, and the space it takes is
        // charged to the row below. Without it every row after the header sat
        // three points too high — measured on the demonstration document.
        let ruled = |thickness: Option<i32>| {
            let mut header = cell("head");
            if let Some(eighths) = thickness {
                header.props.borders.bottom = Some(Border {
                    style: wp_model::prop::BorderStyle::Single,
                    size: Some(wp_model::units::Eighth(eighths)),
                    ..Border::default()
                });
            }
            let table = Table {
                grid: vec![Twips(2880)],
                rows: vec![
                    Row {
                        cells: vec![header],
                        ..Row::new()
                    },
                    Row {
                        cells: vec![cell("body")],
                        ..Row::new()
                    },
                ],
                ..Table::new()
            };
            let document = document(vec![Block::Table(table)]);
            let lines = cell_lines(&document);
            lines
                .iter()
                .find(|(_, text)| text == "body")
                .map(|(y, _)| *y)
                .expect("the second row")
        };
        let bare = ruled(None);
        assert_eq!(
            ruled(Some(24)) - bare,
            3.0,
            "the header's three-point rule pushes the row below it down by three"
        );
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
    fn a_symbol_bullet_is_drawn_as_the_character_it_stands_for() {
        // Word's classic bullet is U+F0B7 *in the Symbol font* — the glyph at
        // Symbol's own 0xB7, parked in the private-use area. Word ships the
        // font; this renderer translates instead, because a reader shown a
        // tofu box was told nothing.
        let mut document = document(Vec::new());
        let mut definition = wp_model::AbstractNum::new(0);
        let mut level = wp_model::Level::new(0);
        level.text = "\u{F0B7}".into();
        level.run.fonts.ascii = Some("Symbol".into());
        definition.set_level(level);
        document.numbering.insert_abstract(definition);
        document.numbering.insert_num(wp_model::Num::new(1, 0));
        document.body = vec![Block::Paragraph(Paragraph {
            props: ParaProps {
                numbering: Some(wp_model::NumRef {
                    num_id: 1,
                    level: 0,
                }),
                ..ParaProps::default()
            },
            ..Paragraph::of("bulleted")
        })];

        let label = pages(&document)
            .iter()
            .flat_map(|page| &page.content)
            .find_map(|placement| match &placement.kind {
                Placed::Line { line, .. } => line.fragments.iter().find_map(|f| match &f.content {
                    crate::inline::Content::Label { text, .. } => Some(text.clone()),
                    _ => None,
                }),
                _ => None,
            })
            .expect("a label");
        assert_eq!(label, "\u{2022}", "the bullet, not the private-use code");

        // A machine that has Symbol itself keeps the private-use character
        // and draws Word's own glyph from the real file — same diameter, same
        // position — instead of a stand-in dot from a fallback face.
        let theme = document.theme.clone();
        let having = Context {
            has_face: |name| name.eq_ignore_ascii_case("Symbol"),
            ..ctx(&theme)
        };
        let mut shaper = crate::shape::Fixed;
        let kept = layout(&document, &having, &mut shaper)
            .iter()
            .flat_map(|page| &page.content)
            .find_map(|placement| match &placement.kind {
                Placed::Line { line, .. } => line.fragments.iter().find_map(|f| match &f.content {
                    crate::inline::Content::Label { text, .. } => Some(text.clone()),
                    _ => None,
                }),
                _ => None,
            })
            .expect("a label");
        assert_eq!(kept, "\u{F0B7}", "the real glyph when the face is real");

        // A numbered label passes through untouched — numbers are never PUA.
        assert_eq!(desymbol("3.", Some("Arial")), "3.");
        // The other gallery glyphs Word uses, and the fallback for a symbol
        // code nobody recognises.
        assert_eq!(desymbol("\u{F0A7}", Some("Wingdings")), "\u{25AA}");
        assert_eq!(desymbol("\u{F0FC}", Some("Wingdings")), "\u{2713}");
        assert_eq!(desymbol("\u{F0E4}", Some("Marlett")), "\u{2022}");
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
            marks: &'a crate::notes::NoteMarks,
        ) -> Context<'a> {
            Context {
                theme,
                styles: &document.styles,
                notes: marks,
                note_mark: None,
                table_part: None,
                default_tab: document.settings.default_tab_stop,
                fallback_font: "Calibri",
                has_face: |_| false,
                show_revisions: true,
                show_hidden: false,
                fields,
                band: None,
                wraps: Box::leak(Box::new(Wraps::default())),
            }
        }

        let empty = crate::FieldValues::default();
        let mut cold = Counting::default();
        let marks = crate::notes::NoteMarks::of(&document);
        let pages = layout(
            &document,
            &make(&theme, &document, &empty, &marks),
            &mut cold,
        );
        let settled = evaluate(&pages, &empty);
        assert!(!settled.is_empty(), "the field was evaluated");

        let mut warm = Counting::default();
        let again = layout(
            &document,
            &make(&theme, &document, &settled, &marks),
            &mut warm,
        );
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
            chart: None,
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
    fn a_column_wide_square_float_reserves_its_height_before_its_paragraph() {
        // The float from file-sample_500kB.docx, shrunk: as wide as the text
        // column, square wrap, anchored to the paragraph. Word resumes the
        // text below it, so the flow must hold its height open.
        let mut section = SectionProps::new();
        let column = section.text_width().points();
        section.page.height = Twips::from_points(2000.0);
        let drawing = wp_model::Drawing {
            source: Vec::new().into(),
            anchored: true,
            extent: (
                wp_model::Emu::from_points(column),
                wp_model::Emu::from_points(50.0),
            ),
            rel: Some("rId7".into()),
            chart: None,
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
                    Piece::Drawing(Box::new(drawing)),
                    Piece::Text("below".into()),
                ],
                ..Run::new()
            })],
            ..Paragraph::new()
        };
        let mut document = document(vec![Block::Paragraph(paragraph)]);
        document.section = section;
        let page = &pages(&document)[0];
        let drawn = page
            .content
            .iter()
            .find(|p| matches!(p.kind, Placed::Drawing { .. }))
            .expect("the drawing was placed");
        let line = page
            .content
            .iter()
            .find(|p| matches!(p.kind, Placed::Line { .. }))
            .expect("a line");
        // The paragraph's own line starts below the reserved height, not
        // beside or under the picture.
        assert_eq!(drawn.height, 50.0);
        assert!(
            line.y >= drawn.y + drawn.height,
            "line at {} should sit below the float ending at {}",
            line.y,
            drawn.y + drawn.height
        );
    }

    #[test]
    fn a_square_float_resized_narrow_still_holds_the_text_below() {
        // The float from file-sample_500kB.docx after the user drags it
        // smaller. Text cannot be set beside a float yet, so the honest
        // rendering keeps the text below — the moment the float stopped
        // reserving its height, the words sat on top of the picture.
        let mut section = SectionProps::new();
        section.page.height = Twips::from_points(2000.0);
        let drawing = wp_model::Drawing {
            source: Vec::new().into(),
            anchored: true,
            extent: (
                wp_model::Emu::from_points(120.0),
                wp_model::Emu::from_points(80.0),
            ),
            rel: Some("rId7".into()),
            chart: None,
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
                    Piece::Drawing(Box::new(drawing)),
                    Piece::Text("below".into()),
                ],
                ..Run::new()
            })],
            ..Paragraph::new()
        };
        let mut document = document(vec![Block::Paragraph(paragraph)]);
        document.section = section;
        let page = &pages(&document)[0];
        let drawn = page
            .content
            .iter()
            .find(|p| matches!(p.kind, Placed::Drawing { .. }))
            .expect("the drawing was placed");
        let line = page
            .content
            .iter()
            .find(|p| matches!(p.kind, Placed::Line { .. }))
            .expect("a line");
        assert!(
            line.y >= drawn.y + drawn.height,
            "line at {} should sit below the float ending at {}",
            line.y,
            drawn.y + drawn.height
        );
    }

    #[test]
    fn a_paragraph_anchored_top_alignment_hangs_from_the_line() {
        // `<wp:positionV relativeFrom="paragraph"><wp:align>top</wp:align>` —
        // the picture's top is the paragraph's top, not the page margin.
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
            chart: None,
            name: None,
            description: None,
            wrap: wp_model::Wrap::Square,
            distance: Default::default(),
            position: Some(Box::new(wp_model::doc::DrawingPosition {
                horizontal: wp_model::doc::Offset {
                    relative_to: wp_model::doc::RelativeTo::Column,
                    offset: None,
                    align: Some(wp_model::doc::Alignment::Center),
                },
                vertical: wp_model::doc::Offset {
                    relative_to: wp_model::doc::RelativeTo::Paragraph,
                    offset: None,
                    align: Some(wp_model::doc::Alignment::Top),
                },
            })),
            behind_text: false,
        };
        let (_, y) = anchor_position(&drawing, &geometry, 400.0);
        assert_eq!(y, 400.0);
    }

    #[test]
    fn a_picture_at_the_margin_narrows_the_text_that_lands_beside_it() {
        // Both of the demonstration document's arrows stand at a margin, at
        // the top of the page, and the text between them is narrowed from both
        // sides — the right-hand one by a paragraph it is not anchored to at
        // all. Where such a float sits is only known once the pages exist, so
        // the layout runs twice; see `Wraps`.
        let margin_arrow = |left: bool| wp_model::Drawing {
            source: Vec::new().into(),
            anchored: true,
            extent: (
                wp_model::Emu::from_points(80.0),
                wp_model::Emu::from_points(60.0),
            ),
            rel: None,
            chart: None,
            name: None,
            description: None,
            wrap: wp_model::Wrap::Square,
            distance: Default::default(),
            position: Some(Box::new(wp_model::doc::DrawingPosition {
                horizontal: wp_model::doc::Offset {
                    relative_to: wp_model::doc::RelativeTo::Margin,
                    offset: None,
                    align: Some(if left {
                        wp_model::doc::Alignment::Left
                    } else {
                        wp_model::doc::Alignment::Right
                    }),
                },
                vertical: wp_model::doc::Offset {
                    relative_to: wp_model::doc::RelativeTo::Margin,
                    offset: None,
                    align: Some(wp_model::doc::Alignment::Top),
                },
            })),
            behind_text: false,
        };
        let words = "wrap ".repeat(300);
        let holder = Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![
                    Piece::Drawing(Box::new(margin_arrow(true))),
                    Piece::Drawing(Box::new(margin_arrow(false))),
                    Piece::Text(words.into()),
                ],
                ..Run::new()
            })],
            ..Paragraph::new()
        };
        let document = document(vec![Block::Paragraph(holder)]);
        let pages = pages(&document);
        let lines: Vec<&Placement> = pages[0]
            .content
            .iter()
            .filter(|p| matches!(p.kind, Placed::Line { .. }))
            .collect();
        // The first lines are set between the two pictures; the ones below
        // them have the whole column again.
        let narrow = lines.first().expect("a first line");
        let wide = lines.last().expect("a last line");
        assert!(
            narrow.x >= pages[0].geometry.start + 80.0 - 0.01,
            "the left picture pushed the first line clear of it: {}",
            narrow.x
        );
        let column_end = pages[0].geometry.width - pages[0].geometry.end;
        assert!(
            narrow.x + narrow.width <= column_end - 80.0 + 0.01,
            "and the right one took the other end: {} reaches {}",
            narrow.width,
            narrow.x + narrow.width
        );
        let widest = lines.iter().map(|line| line.width).fold(0.0f64, f64::max);
        assert!(
            widest > narrow.width + 100.0,
            "the lines below the pictures have the whole column: {widest}"
        );
        assert!((wide.x - pages[0].geometry.start).abs() < 0.01);
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
            chart: None,
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
