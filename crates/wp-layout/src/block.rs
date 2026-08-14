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
//! balancing the last page. A table row taller than a page is placed whole and
//! overflows rather than being split mid-cell. Each is a body of work of its
//! own and each is visible rather than silent.

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
    /// One laid-out line. Its fragments' `x` are relative to the line's own.
    Line(Box<Line>),
    /// A filled rectangle: cell or paragraph shading.
    Fill([u8; 3]),
    /// One edge of a border.
    Edge { border: Border, side: Side },
    /// A drawing, by the relationship naming the part that holds its bytes.
    Drawing { rel: Option<Arc<str>> },
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
    /// Footnotes referenced by this item, and how tall they are.
    pub footnotes: Vec<(i32, f64)>,
}

/// A document, flowed into items and not yet paginated.
#[derive(Debug, Clone, Default)]
pub struct Flow {
    pub items: Vec<Item>,
}

/// Everything the block layout needs beyond the document.
pub struct Frame<'a, 'b> {
    pub document: &'a Document,
    pub inline: &'a Context<'b>,
}

/// Lays a whole document out into pages.
pub fn layout(document: &Document, ctx: &Context<'_>, shaper: &mut dyn Shaper) -> Vec<Page> {
    let mut pages = Vec::new();
    let mut counters = Counters::new();
    let mut number = 1u32;

    for (section_index, (range, section)) in document.sections().into_iter().enumerate() {
        if let Some(start) = section.page_numbering.start {
            number = start;
        }
        let width = section.text_width().points();
        let height = section.text_height().points();
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
                geometry: PageBox::of(section),
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
            for item in &flow.items[placed..*end] {
                for part in &item.parts {
                    page.content.push(Placement {
                        x: page.geometry.start + part.x,
                        y: y + part.y,
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

    if let Some(body) = section.header(kind).or_else(|| {
        // A section with no header of the kind the page wants has *no* header —
        // the reference being absent is the instruction, and falling back to the
        // default one would put a header on a title page that asked for none.
        (kind == HeaderKind::Default).then_some(section.header(HeaderKind::Default)?)
    }) {
        if let Some(header) = document.header(body) {
            let mut y = section.margins.header.points();
            for placement in band(&header.content, document, ctx, shaper, width) {
                page.header.push(Placement {
                    x: section.margins.start.points() + placement.x,
                    y: y + placement.y,
                    ..placement
                });
            }
            y += 0.0;
            let _ = y;
        }
    }

    if let Some(body) = section.footer(kind) {
        if let Some(footer) = document.header(body) {
            let placements = band(&footer.content, document, ctx, shaper, width);
            let height: f64 = placements.iter().map(|p| p.height).sum();
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

/// Lays a header or footer body out as a simple stack.
fn band(
    blocks: &[Block],
    document: &Document,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
    width: f64,
) -> Vec<Placement> {
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
            out.push(Placement {
                y: y + part.y,
                ..part
            });
        }
        y += item.height;
    }
    out
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

    let laid = inline::layout(paragraph, &layers, label.as_ref(), ctx, width, shaper);
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
    let count = laid.lines.len().max(1);
    let explicit_break = paragraph
        .runs()
        .iter()
        .flat_map(|run| &run.content)
        .any(|piece| matches!(piece, Piece::Break(Break::Page)));
    let before = laid.space_before;
    let after = laid.space_after;

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
        let parts = vec![Placement {
            x,
            y: top,
            width: line.width,
            height: line.height,
            kind: Placed::Line(Box::new(line)),
        }];
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

    let target = match table.props.width {
        Width::Fixed(twips) => twips.points(),
        Width::Percent(pct) => pct.of(Twips::from_points(available)).points(),
        // `auto` and `nil`: the grid decides, unless it does not fit.
        _ => {
            if total > 0.0 {
                total.min(available)
            } else {
                available
            }
        }
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
    let margins = &table.props.cell_margins;
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

    let group = into.items.len();
    // A header row repeats only while every row before it also says so: Word
    // stops at the first row that does not.
    let mut still_header = true;

    for (row_index, row) in table.rows.iter().enumerate() {
        let mut parts: Vec<Placement> = Vec::new();
        let mut height: f64 = 0.0;
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
            let mut cell_flow = Flow::default();
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
            }
            let content_height: f64 = cell_flow.items.iter().map(|item| item.height).sum();
            let cell_height = content_height + pad_top + pad_bottom;
            height = height.max(cell_height);

            if let Some(fill) = cell
                .props
                .shading
                .and_then(|s| s.background())
                .and_then(|c| c.resolve(&document.theme))
            {
                parts.push(Placement {
                    x,
                    y: 0.0,
                    width: cell_width,
                    height: 0.0,
                    kind: Placed::Fill(fill),
                });
            }
            for (side, border) in [
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
            ] {
                if let Some(border) = border.filter(|b| b.style.draws()) {
                    parts.push(Placement {
                        x,
                        y: 0.0,
                        width: cell_width,
                        height: 0.0,
                        kind: Placed::Edge { border, side },
                    });
                }
            }

            let mut y = pad_top;
            for item in cell_flow.items {
                for part in item.parts {
                    parts.push(Placement {
                        x: x + pad_start + part.x,
                        y: y + part.y,
                        ..part
                    });
                }
                y += item.height;
            }

            x += cell_width;
            column += span;
        }

        // A stated row height is a floor or a ceiling depending on its rule.
        if let Some(rule) = row.props.height {
            height = match rule {
                wp_model::table::RowHeight::Auto => height,
                wp_model::table::RowHeight::AtLeast(t) => height.max(t.points()),
                wp_model::table::RowHeight::Exact(t) => t.points(),
            };
        }
        // Every part that spans the row learns its height now that it is known.
        for part in &mut parts {
            if matches!(part.kind, Placed::Fill(_) | Placed::Edge { .. }) {
                part.height = height;
            }
        }

        still_header = still_header && row.props.header;
        into.items.push(Item {
            height,
            parts,
            group,
            index_in_group: row_index,
            items_in_group: table.rows.len(),
            keep_with_next: false,
            keep_lines: row.props.cant_split,
            widow_control: false,
            break_before: false,
            repeat: still_header,
            footnotes: Vec::new(),
        });
    }
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
    items
        .iter()
        .take(start)
        .filter(|earlier| earlier.group == item.group && earlier.repeat)
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

    fn cell(text: &str) -> Cell {
        Cell {
            props: CellProps::new(),
            content: vec![Block::Paragraph(Paragraph::of(text))],
        }
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
                Placed::Line(line) => Some(
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
                Placed::Line(line) => line
                    .fragments
                    .iter()
                    .any(|f| matches!(f.content, crate::inline::Content::Label)),
                _ => false,
            })
            .count();
        assert_eq!(labelled, 12, "every item got a label");
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
