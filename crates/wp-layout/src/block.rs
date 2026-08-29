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

use wp_model::doc::{Block, Break, Document, Paragraph, Piece, Scope};
use wp_model::numbering::Counters;
use wp_model::prop::Border;
use wp_model::section::{Bands, HeaderId, HeaderKind, PageBox, SectionProps};
use wp_model::style::Layers;
use wp_model::table::{CellVAlign, Table, VMerge, Width};
use wp_model::units::Twips;

use crate::inline::{self, Context, LaidParagraph, Line, ListLabel};
use crate::shape::Shaper;
use crate::FontRequest;

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
    ///
    /// Shared rather than owned: the same line travels from the flow into an
    /// item and from the item onto a page, and copying it at each step is what
    /// a long document spends its afternoon on. A renderer that needs to change
    /// one takes its own copy — see [`crate::inline::LaidParagraph`].
    Line { line: Arc<Line>, paragraph: usize },
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
        /// A shape that *is* its words, already measured. See [`ShapeWords`].
        words: Option<Box<ShapeWords>>,
    },
    /// The rule Word draws above the footnote area.
    FootnoteSeparator,
}

/// A shape's own words, sized to the shape and measured here.
///
/// **The size comes from the shape, not from the text.** WordArt has no point
/// size — Word stretches the glyphs until they fill the box — so the face is
/// measured once and scaled to the width it has to cover, capped so a short
/// word in a tall box does not grow past the box's height.
///
/// Measured in the layout rather than in each renderer because the screen, the
/// PDF and the printer must put the same glyphs in the same places, and the
/// only way three renderers agree is for one of them to decide.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeWords {
    pub text: String,
    /// The face, at the size that fills the shape.
    pub font: FontRequest,
    /// Per character, as every other measured run states it.
    pub advances: Vec<f64>,
    pub rgb: [u8; 3],
    /// Degrees clockwise, turned about the middle of the shape's box.
    pub rotation: f64,
    /// The measured line, before it is turned. The height already carries
    /// [`ShapeWords::stretch`].
    /// The width of the box that is centred in the shape.
    pub width: f64,
    pub height: f64,
    /// Baseline below the box's own top.
    pub ascent: f64,
    /// Where the pen starts, relative to the left edge of that box.
    ///
    /// **A letter does not begin where its pen does.** Word fits the drawn
    /// *outline* to the shape, so the box being centred is the ink's and the
    /// pen stands a side bearing to the left of it — negative `lead`, most of
    /// the time. Zero for a shaper that cannot see inside the glyphs, which is
    /// the same thing as centring the line box and is what this did before.
    pub lead: f64,
    /// How much taller than its own proportion the face is drawn.
    ///
    /// **WordArt is not type at a size.** The words are stretched until they
    /// fill the shape they were drawn in, across *and* down, and the two need
    /// not agree: Word's diagonal watermark on this page is Courier New set to
    /// 609 points of width in a box 152 points tall, so its letters stand half
    /// again as tall as that width would make them. One of the two is a size a
    /// shaper can measure and the other is not, so it is stated apart and every
    /// renderer applies it to the glyphs themselves — a PDF in its text matrix,
    /// the screen in the mesh it builds from the galley.
    pub stretch: f64,
}

impl ShapeWords {
    /// Where the baseline starts once the line is turned about the middle of
    /// `rect`, which is the point every renderer draws from and turns about.
    pub fn origin(&self, x: f64, y: f64, width: f64, height: f64) -> (f64, f64) {
        let (sin, cos) = self.rotation.to_radians().sin_cos();
        let (vx, vy) = (
            -self.width / 2.0 + self.lead,
            -self.height / 2.0 + self.ascent,
        );
        (
            x + width / 2.0 + vx * cos - vy * sin,
            y + height / 2.0 + vx * sin + vy * cos,
        )
    }
}

/// What a drawing's picture is fetched and cached under: its relationship,
/// with the washout it is drawn through folded in.
fn picture_name(drawing: &wp_model::Drawing) -> Option<std::sync::Arc<str>> {
    let rel = drawing.rel.as_deref()?;
    Some(wp_model::doc::picture_key(rel, drawing.tone).into())
}

/// Measures a shape's words at the size that fills it.
///
/// **A shape says which of two things to do with its words**, and they look
/// nothing alike — see [`wp_model::doc::ShapeText::stretch`].
///
/// *Stretched*: what fills the shape is the ink, not the line box. Word,
/// measured: a WordArt shape 400 by 200 points draws "CONFIDENTIAL", "gypsy",
/// "Hg" and "xxxx" each with their outlines spanning 400 by 200 to a fiftieth
/// of a point — so four strings of wildly different proportions all come out
/// exactly as tall as the shape. Fitting the face's ascent-plus-descent
/// instead makes every string the same height as every other and leaves an
/// all-capitals watermark two thirds the size Word draws it. A shaper that
/// cannot see inside the glyphs falls back to the line box, which is what
/// this always did.
///
/// *Not stretched*, which is what a watermark is: the size is the one whose
/// *advances* fill the shape's width — the pen starts at the shape's left edge
/// and the last letter's advance ends at its right — and the only thing done
/// down the page is that one em is scaled to the shape's height, with the
/// baseline set a descent above the shape's foot. Measured against Word's own
/// diagonal `CONFIDENTIAL`, a Courier New shape 609.10 by 152.25 points: the
/// advance came out at 50.758 points, which is that face's 1229/2048 em at
/// 84.583, and every letter's drawn outline matched the face's own bounds at
/// that width and at 152.25 of height, to a hundredth of a point.
pub fn shape_words(
    drawing: &wp_model::Drawing,
    shaper: &mut dyn Shaper,
) -> Option<Box<ShapeWords>> {
    /// The size the face is measured at before it is scaled to the box. Large
    /// enough that the answer is not dominated by hinting.
    const MEASURED_AT: f64 = 100.0;

    let shape = drawing.text.as_deref()?;
    let text = shape.text.trim();
    if text.is_empty() {
        return None;
    }
    let mut request = FontRequest {
        family: shape.font.clone().unwrap_or_else(|| "Arial".into()),
        size: MEASURED_AT,
        bold: shape.bold,
        italic: shape.italic,
    };
    let ink = shaper.ink(text, &request);
    let metrics = shaper.metrics(&request);
    // What has to reach the shape's width: the ink when the words are pulled
    // about to fill it, and the advances when they are only sized to it.
    let natural = match (&ink, shape.stretch) {
        (Some(ink), true) => ink.width(),
        _ => shaper.width(text, &request),
    };
    let tall = match &ink {
        Some(ink) => ink.height(),
        None => metrics.ascent + metrics.descent,
    };
    if natural <= 0.0 || tall <= 0.0 {
        return None;
    }
    let box_width = drawing.extent.0.points();
    let box_height = drawing.extent.1.points();
    // Across fills the box's width. The size is that, because a size is the
    // only thing a shaper can be asked for; whatever the down direction wants
    // beyond it is the stretch, which the renderers apply to the glyphs.
    let across = (box_width / natural).max(0.01);
    request.size = MEASURED_AT * across;
    // Stretched, one em covers the box's height; unstretched, the ink does.
    let stretch = match shape.stretch {
        true => (box_height / tall).max(0.01) / across,
        false => (box_height / request.size).max(0.01),
    };

    let mut measured = Vec::new();
    shaper.advances(text, &request, &mut measured);
    let metrics = shaper.metrics(&request);
    // The box the renderers centre in the shape: the ink's when the glyphs
    // could be read and are being fitted to the shape, and the advances' when
    // they are not — a watermark's pen starts at the shape's own left edge,
    // and its baseline stands a descent above the shape's foot.
    let fitted = shaper.ink(text, &request).filter(|_| shape.stretch);
    let (width, height, ascent, lead) = match &fitted {
        Some(ink) => (
            ink.width(),
            ink.height() * stretch,
            ink.top * stretch,
            -ink.left,
        ),
        None if shape.stretch => (
            measured.iter().map(|advance| advance.width).sum(),
            (metrics.ascent + metrics.descent) * stretch,
            metrics.ascent * stretch,
            0.0,
        ),
        None => (
            measured.iter().map(|advance| advance.width).sum(),
            box_height,
            box_height - metrics.descent * stretch,
            0.0,
        ),
    };
    Some(Box::new(ShapeWords {
        text: text.to_owned(),
        advances: measured.iter().map(|advance| advance.width).collect(),
        width,
        height,
        ascent,
        lead,
        stretch,
        rgb: match shape.color {
            Some(wp_model::Color::Rgb(rgb)) => rgb,
            // Word's own watermark grey, for a shape that states no fill.
            _ => [0xC0, 0xC0, 0xC0],
        },
        rotation: shape.rotation,
        font: request,
    }))
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
    /// Which header body this page's band was laid out from, and which footer
    /// body its own was.
    ///
    /// **A band's paragraph numbers are its own.** The header is flowed from
    /// zero, so a `Placed::Line` in [`Page::header`] names a paragraph of that
    /// header and not of the document — and which header it is depends on the
    /// page, since a section can name a different one for its first page and
    /// for its even ones. Without this the numbers are an index into nothing,
    /// which is exactly how a click on the body once selected the page frame
    /// drawn in the header.
    pub header_body: Option<HeaderId>,
    pub footer_body: Option<HeaderId>,
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

    /// The placements of one of the document's flows on this page.
    ///
    /// What an editor working in `scope` may put a caret in, and nothing else:
    /// a page that shows a different header than the one being edited answers
    /// with nothing, so a click there cannot land on a paragraph of a header
    /// that is not open.
    pub fn placements(&self, scope: Scope) -> &[Placement] {
        match scope {
            Scope::Body => &self.content,
            Scope::Chrome(id) if self.header_body == Some(id) => &self.header,
            Scope::Chrome(id) if self.footer_body == Some(id) => &self.footer,
            Scope::Chrome(_) => &[],
        }
    }

    /// Which flow a band belongs to, for a caller holding one of the vectors.
    pub fn header_scope(&self) -> Option<Scope> {
        self.header_body.map(Scope::Chrome)
    }

    pub fn footer_scope(&self) -> Option<Scope> {
        self.footer_body.map(Scope::Chrome)
    }

    /// Everything on the page, in the order a renderer must draw it: shapes
    /// that belong under the words first, border edges after all else.
    ///
    /// Word paints shading below borders, always. In document order the two
    /// meet exactly on a table's row and column boundaries — the next row's
    /// cell fill starts where this row's hairline bottom rule was just drawn —
    /// and whichever the rasterizer rounds wider wins. On the screen that ate
    /// the borders under three white-shaded rows; in the PDF it ate a column
    /// rule. Putting every edge after every fill ends the coin-toss.
    ///
    /// **The header and footer are a layer under the body, not a band beside
    /// it.** Every shape anchored in one is drawn before the page's own words,
    /// whatever the shape itself says about being behind the text — which is
    /// what stops a watermark from striking out the page it stamps.
    /// Each placement arrives with the flow it belongs to, because a line's
    /// paragraph number means nothing without it — the selection a renderer
    /// paints over paragraph three has to know which body's third paragraph
    /// that is. Footnotes answer [`Scope::Body`]: they are the body's own
    /// apparatus and a header is not.
    pub fn painted(&self) -> impl Iterator<Item = (Scope, &Placement)> {
        fn layer(placement: &Placement, in_band: bool) -> u8 {
            match &placement.kind {
                Placed::Edge { .. } => 2,
                Placed::Drawing {
                    anchor: Some(drawing),
                    ..
                } if in_band || drawing.behind_text => 0,
                _ => 1,
            }
        }
        let header = self.header_scope().unwrap_or_default();
        let footer = self.footer_scope().unwrap_or_default();
        let bands = self
            .header
            .iter()
            .map(move |placement| (header, placement))
            .chain(self.footer.iter().map(move |placement| (footer, placement)));
        let mut out: Vec<(u8, Scope, &Placement)> = self
            .content
            .iter()
            .map(|placement| (layer(placement, false), Scope::Body, placement))
            .chain(bands.map(|(scope, placement)| (layer(placement, true), scope, placement)))
            .chain(
                self.footnotes
                    .iter()
                    .map(|placement| (layer(placement, false), Scope::Body, placement)),
            )
            .collect();
        // Stable, so within a layer the page is still drawn in the order it
        // was laid out: content, header, footer, notes.
        out.sort_by_key(|(layer, _, _)| *layer);
        out.into_iter()
            .map(|(_, scope, placement)| (scope, placement))
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
    /// The space above this item that a page will not give it.
    ///
    /// **Word does not space a paragraph away from the top of a page it fell
    /// onto.** Measured: a paragraph set 24 points before begins at the top
    /// margin exactly when the page above it simply ran out of room, and keeps
    /// all 24 when the writer typed the break himself — the break is then part
    /// of the paragraph, and the space follows it as it would follow anything
    /// else. Word's own compatibility list has an option to suppress that
    /// second case as well, which is the plainest evidence that it is not
    /// suppressed by default. A paragraph at the very start of the document,
    /// where no page ended at all, also keeps its space.
    ///
    /// Which page an item opens is not known until the flow has been
    /// paginated, so the space is flowed in like any other and taken off
    /// again afterwards — by [`paginate`], which has to know the heights it is
    /// breaking on, and by the placement, which draws them.
    pub space_before: f64,
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
    // The cache spans the whole of this layout rather than one pass of it: the
    // second pass a page number or a float asks for lays the same paragraphs
    // again, and answering those from what the first pass settled is most of
    // what makes the second pass cheap.
    if let Some(memo) = ctx.memo {
        memo.begin(ctx);
    }
    let pages = laid(document, ctx, shaper);
    if let Some(memo) = ctx.memo {
        memo.commit();
    }
    pages
}

fn laid(document: &Document, ctx: &Context<'_>, shaper: &mut dyn Shaper) -> Vec<Page> {
    let plain = layout_once(document, ctx, shaper);
    // A float anchored to the page or a margin sits where only pagination
    // knows, and the lines it narrows have to be broken before that. So the
    // document is laid out once, the floats are read off the pages, and the
    // paragraphs they stand beside are laid again — the same two-pass shape a
    // `{ PAGE }` field needs, for the same reason. Once only: a wrap that
    // moved the float that caused it would never settle.
    let wraps = Wraps::of(&plain);
    let beside = Context {
        memo: None,
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
    // Resolved once for the document: which band each section really shows,
    // with "Link to Previous" followed back through the ones before it.
    let bands = document.bands();

    for (section_index, (range, section)) in document.sections().into_iter().enumerate() {
        if let Some(start) = section.page_numbering.start {
            number = start;
        }
        let width = section.text_width().points();
        let shown = bands.get(section_index).copied().unwrap_or_default();
        let (top, bottom) = band_margins(document, section, shown, ctx, shaper);
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
        let opens_document = pages.is_empty();
        let mut breaks = paginate(&flow.items, height, opens_document);

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
            breaks = paginate(&flow.items, height, opens_document);
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
                header_body: None,
                footer_body: None,
            };
            // A multi-column section runs its items down the first column and
            // then the next. Balancing the last page is a separate problem and
            // is not attempted.
            let column = columns.first().map(|c| c.width.points()).unwrap_or(width);
            let _ = column;
            let slice = &flow.items[placed..*end];
            // The space above the first item is not on this page — the item is
            // started that much higher and everything it holds comes with it.
            let mut y = page.geometry.top
                - slice.first().map_or(0.0, |item| {
                    dropped_space(item, true, opens_document && page_index == 0)
                });
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
                shown,
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
    shown: Bands,
    ctx: &Context<'_>,
    shaper: &mut dyn Shaper,
) -> (f64, f64) {
    let width = section.text_width().points();
    let mut top = section.margins.top.points();
    let mut bottom = section.margins.bottom.points();
    if let Some(body) = shown.header(HeaderKind::Default) {
        if let Some(header) = document.header(body) {
            let (_, height) = band(&header.content, document, ctx, shaper, width);
            top = top.max(section.margins.header.points() + height);
        }
    }
    if let Some(body) = shown.footer(HeaderKind::Default) {
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
    shown: Bands,
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

    // A section that names no header of the kind the page wants shows the one
    // the section before it does — Word's "Link to Previous" — and if no
    // section back to the first names one either, the page has no header. What
    // is never done is falling back to the *default* kind: a title page that
    // asked for a first-page header and has none is a page with no header, and
    // stamping the ordinary one on it is the one thing Word does not do.
    if let Some(body) = shown.header(kind) {
        if let Some(header) = document.header(body) {
            let y = section.margins.header.points();
            page.header_body = Some(body);
            for placement in band(&header.content, document, ctx, shaper, width).0 {
                page.header.push(Placement {
                    x: section.margins.start.points() + placement.x,
                    y: y + placement.y,
                    ..placement
                });
            }
        }
    }

    if let Some(body) = shown.footer(kind) {
        if let Some(footer) = document.header(body) {
            let (placements, height) = band(&footer.content, document, ctx, shaper, width);
            page.footer_body = Some(body);
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
        from: 0.0,
        depth: height,
        indent: extent,
        inset: 0.0,
        hole: None,
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
        from: 0.0,
        depth: height + below,
        indent: extent + gap,
        inset: 0.0,
        hole: None,
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
            // A level that names a picture draws it in place of the glyph.
            // The glyph stays in the label as the text it reads as — Word
            // leaves one in `<w:lvlText>` for exactly that reason.
            picture: level
                .picture_bullet
                .and_then(|id| document.numbering.picture_bullet(id))
                .cloned(),
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
    // A picture standing in the column narrows this paragraph as much as a
    // floating table would. Which paragraphs it reaches is knowledge from the
    // pass before this one — see [`Wraps`].
    //
    // The body's alone. `Wraps` is gathered from `page.content`, so its keys
    // are body paragraph numbers, and a header, a footer and a note each number
    // their own from zero: without this the first line of every header on a
    // page would be narrowed by whatever stands beside the body's first
    // paragraph. The same reason the memo below is the body's alone.
    let own_flow = ctx.band.is_none() && ctx.note_mark.is_none() && !into.in_note;
    if let Some(beside) = ctx.wraps.beside(index).filter(|_| own_flow) {
        into.obstacle = Some(match into.obstacle {
            Some(already) => inline::Obstacle {
                // The band the two of them cover between them, and the worse
                // of each side. A hole is kept only where nothing else already
                // narrows the line: a line cannot be parted twice here, and a
                // drop cap that also has a float beside it is one line in three
                // pieces, which nothing measures and nothing draws.
                from: already.from.min(beside.from),
                depth: already.depth.max(beside.depth),
                indent: already.indent.max(beside.indent),
                inset: already.inset.max(beside.inset),
                hole: already.hole.or(beside.hole),
            },
            None => beside,
        });
    }
    // A header, a footer and a note each number their paragraphs from zero in
    // a flow of their own, so their indices are the body's indices and mean
    // something else entirely. Only the body is remembered.
    let memo = ctx
        .memo
        .filter(|_| ctx.band.is_none() && ctx.note_mark.is_none() && !into.in_note);
    let in_contents = ctx.contents.holds(index);
    let recalled = memo.and_then(|memo| {
        memo.recall(
            index,
            paragraph,
            &layers,
            label.as_ref(),
            width,
            into.obstacle,
            in_contents,
        )
    });
    let laid = match recalled {
        Some(laid) => laid,
        None => {
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
            if let Some(memo) = memo {
                memo.remember(
                    index,
                    paragraph,
                    &layers,
                    label.as_ref(),
                    width,
                    into.obstacle,
                    in_contents,
                    &laid,
                );
            }
            laid
        }
    };
    let first = into.items.len();
    push_paragraph(
        paragraph, &layers, laid, left, width, ctx.theme, shaper, into,
    );
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

#[allow(clippy::too_many_arguments)]
fn push_paragraph(
    paragraph: &Paragraph,
    layers: &Layers,
    laid: LaidParagraph,
    left: f64,
    width: f64,
    theme: &wp_model::color::Theme,
    shaper: &mut dyn Shaper,
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
        .any(|(_, drawing)| displaces(drawing, width))
    {
        while into.items.last().is_some_and(is_empty_line) {
            displaced.push(into.items.pop().expect("just checked"));
        }
        displaced.reverse();
    }
    let mut dead: f64 = displaced.iter().map(|item| item.height).sum();
    for (nth, drawing) in anchored(paragraph) {
        if !displaces(drawing, width) {
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
                    // The tone rides in the name, so the same bytes drawn
                    // plainly and washed out as a watermark stay two
                    // pictures — see `wp_model::doc::picture_key`.
                    rel: picture_name(drawing),
                    anchor: Some(Box::new(drawing.clone())),
                    paragraph: paragraph_index,
                    nth,
                    words: shape_words(drawing, shaper),
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
            space_before: 0.0,
        });
    }
    into.items.append(&mut displaced);

    let group = into.items.len();
    // The group the lines being pushed belong to, which starts again after
    // every hand-written page break. `group` itself stays where it was, as the
    // mark for whether this paragraph put anything down at all.
    let mut piece_group = group;
    into.paragraphs += 1;
    // The float stands beside this paragraph and whatever follows until its
    // depth is used up. What the paragraph consumes is taken off here, before
    // its lines are turned into items.
    let laid_height: f64 = laid.lines.iter().map(|line| line.height).sum();
    if let Some(obstacle) = &mut into.obstacle {
        let spent = laid_height + laid.space_before;
        obstacle.depth -= spent;
        obstacle.from = (obstacle.from - spent).max(0.0);
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
    // A page break the author put in by hand divides the paragraph for the
    // keep rules. Whatever "keep lines together" asks for, the lines before
    // the break and the lines after it are on different pages, so a group that
    // reaches across it has nothing left to hold together — and pulling the
    // lines above the break forward only spends a page on them. Measured on
    // the demonstration document, whose headings keep their lines together and
    // begin with a page break: the empty line the break ends took a whole page
    // of its own, and the heading's own text landed on the page after that.
    let pieces: Vec<(usize, usize)> = {
        let mut out = Vec::with_capacity(count);
        let mut from = 0;
        for index in 0..count {
            let breaks_here = laid
                .lines
                .get(index)
                .is_some_and(|line| line.ended_by == Some(Break::Page));
            if breaks_here || index + 1 == count {
                let size = index + 1 - from;
                out.extend((0..size).map(|within| (within, size)));
                from = index + 1;
            }
        }
        out
    };
    // See [`Flow::last_after`]: the gap between paragraphs is the larger of
    // the two spacings, and the previous one's share is already placed.
    let before = (laid.space_before - into.last_after).max(0.0);
    let after = laid.space_after;
    into.last_after = after;

    // Any other anchored drawing is placed on the page rather than in the line,
    // so it rides with the paragraph's first item and is positioned from there.
    let floats: Vec<Placement> = anchored(paragraph)
        .into_iter()
        .filter(|(_, drawing)| !displaces(drawing, width))
        .map(|(nth, drawing)| Placement {
            x: 0.0,
            y: 0.0,
            width: drawing.extent.0.points(),
            height: drawing.extent.1.points(),
            kind: Placed::Drawing {
                rel: picture_name(drawing),
                anchor: Some(Box::new(drawing.clone())),
                paragraph: paragraph_index,
                nth,
                words: shape_words(drawing, shaper),
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

    // A page break with nothing in front of it takes the whole paragraph with
    // it — the space above included, because there is nothing left behind for
    // that space to sit under. Word starts such a paragraph a clear twelve
    // points below the header of the page it breaks to, which is the heading's
    // own space before; left on the page the break came from, under an empty
    // line nobody can see, it is spent where it does no good and every line of
    // the new page sits twelve points high.
    //
    // A paragraph that is *only* a break keeps its line: it has no second one
    // to carry the break instead.
    let opens_with_break = count > 1
        && laid
            .lines
            .first()
            .is_some_and(|line| line.fragments.is_empty() && line.ended_by == Some(Break::Page));

    for (index, line) in laid.lines.into_iter().enumerate() {
        if opens_with_break && index == 0 {
            continue;
        }
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
            //
            // The line is shared, so paying takes a copy of it. Only the line
            // that actually tips the debt pays, which is why the copy is rare
            // and the sharing survives: a face whose laid height is its ideal —
            // every fixed shaper, and most real ones — never reaches here.
            if into.drift >= 0.5 - 1e-9 {
                Arc::make_mut(&mut line).height += 0.5;
                into.drift -= 0.5;
                into.dumped = true;
            } else if into.drift <= -0.5 + 1e-9 {
                Arc::make_mut(&mut line).height -= 0.5;
                into.drift += 0.5;
                into.dumped = true;
            }
        }
        // The first line to be *placed*, which is the one after the break when
        // the break opened the paragraph: it carries the space above.
        let is_first = index == usize::from(opens_with_break);
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
                line,
                paragraph: paragraph_index,
            },
        });
        into.items.push(Item {
            height,
            parts,
            group: piece_group,
            index_in_group: pieces[index].0,
            items_in_group: pieces[index].1,
            keep_with_next: is_last && layers.para.keep_next.unwrap_or(false),
            keep_lines: layers.para.keep_lines.unwrap_or(false),
            // Word's default is on. A document that says nothing gets widow and
            // orphan control, which is why a single line of a paragraph almost
            // never sits alone at the top of a page in a Word document.
            widow_control: layers.para.widow_control.unwrap_or(true),
            break_before: is_first
                && (layers.para.page_break_before.unwrap_or(false) || opens_with_break),
            repeat: false,
            table: None,
            footnotes: Vec::new(),
            // The space-after may sink into the bottom margin rather than
            // pushing this line to the next page.
            slack: if is_last { after } else { 0.0 },
            // The border's stand-off is not space between paragraphs and is
            // not dropped with it: it is where the paragraph's own rule goes.
            // Nor is anything dropped from a paragraph that opens with a break
            // of its own — see [`Item::space_before`].
            space_before: if is_first && !opens_with_break {
                before
            } else {
                0.0
            },
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
                group: piece_group,
                index_in_group: pieces[index].0,
                items_in_group: pieces[index].1,
                keep_with_next: false,
                keep_lines: false,
                widow_control: false,
                break_before: true,
                repeat: false,
                table: None,
                footnotes: Vec::new(),
                slack: 0.0,
                space_before: 0.0,
            });
            // Everything after the break is a group of its own.
            piece_group = into.items.len();
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
            space_before: 0.0,
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
    // Merged cells whose alignment cannot be settled yet — see `Aligning`.
    let mut aligning: Vec<Aligning> = Vec::new();
    // The bottom rule of the row before this one, which the two rows share.
    let mut rule_from_above: f64 = 0.0;
    // And what that rule was, column by column, because the row below is the
    // one that draws it. Kept per column rather than per row: a row whose
    // cells rule differently leaves a boundary that changes weight along its
    // length, and the columns are the only place that difference survives.
    let grid_columns = widths.len() as u32;
    let mut above_bottom: Vec<Option<Border>> = vec![None; widths.len()];
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
                align: match holds_a_float(&cell.content) {
                    // **Word does not vertically align a cell that holds a
                    // floating shape.** The demonstration document's
                    // letterhead is the case: the cell its address sits in
                    // also anchors the page frame and the watermark, and Word
                    // leaves its three lines at the top of a merge that runs
                    // the whole table deep, where centring them would put them
                    // nine points lower. Take the shapes out of that one cell
                    // and Word centres it. The setting named for this —
                    // `<w:doNotVertAlignCellWithSp/>`, which the document also
                    // carries — turns out not to govern it: taken out, and
                    // with the document put in Word 2013's compatibility mode
                    // besides, Word still leaves the cell alone.
                    true => CellVAlign::Top,
                    false => match cell.props.v_align {
                        CellVAlign::Top => styled.cell_v_align.unwrap_or(CellVAlign::Top),
                        stated => stated,
                    },
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
                edges: {
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
                    // Between the rows of a vertical merge there is no edge:
                    // the reader sees one tall cell, and Word draws neither
                    // the rule below the cell whose merge carries on nor the
                    // one above the cell that carries it. The demonstration
                    // document's nested table ruled a line straight through
                    // the cell holding "One" and "Three" until this was here.
                    // The rest of the row still rules — the height a row pays
                    // for its rules is the widest cell's, not this one's, and
                    // that is why the undrawn rule survives into `rules`.
                    let merges_down = table.merge_height(row_index, column) > 1;
                    // What the row above left at this cell's top edge. The
                    // heaviest of the columns it covers, because a rule the
                    // row above drew in pieces of different weights arrives
                    // here as one line and Word gives a contested edge to the
                    // heavier of the two.
                    let from_above = above_bottom
                        .get(column as usize..(column + span).min(grid_columns) as usize)
                        .unwrap_or_default()
                        .iter()
                        .copied()
                        .fold(None, heavier);
                    [Side::Top, Side::Start, Side::Bottom, Side::End].map(|side| {
                        let hidden = match side {
                            Side::Top => is_continuation,
                            Side::Bottom => merges_down,
                            _ => false,
                        };
                        let rules = pick(side).map(|b| themed(b, &document.theme));
                        let cuts = (!hidden).then_some(rules).flatten();
                        // The row above owns nothing: it has already gone by
                        // when this row is laid out, so the shared rule is
                        // drawn here, once, as the heavier of the two edges
                        // that meet. The cell to the left is still at hand, so
                        // that pair is settled below instead, after the whole
                        // row is built.
                        let draws = match side {
                            Side::Top if row_index > 0 => heavier(cuts, from_above),
                            Side::Bottom if !is_last_row => None,
                            _ => cuts,
                        };
                        CellEdge {
                            side,
                            rules,
                            draws,
                            cuts,
                        }
                    })
                },
                content: y,
                spans: table.merge_height(row_index, column),
                aligning: None,
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
        // The rule down a column is one line, and the two cells it separates
        // both name it. The left one draws it, as the heavier of the two, and
        // the right one stands down — drawn twice it is the same stroke laid
        // over itself, which paper hides and a screen does not: the second
        // pass darkens the first one's soft edges until a hairline between
        // columns reads as heavy as the table's own frame.
        //
        // Only cells that actually touch: a row that begins late or ends
        // early leaves a gap the grid still counts, and the rule on either
        // side of it belongs to nobody but the cell that states it.
        for i in 1..cells.len() {
            let (left, right) = cells.split_at_mut(i);
            let (left, right) = (&mut left[i - 1], &mut right[0]);
            if (left.x + left.width - right.x).abs() > EPSILON {
                continue;
            }
            let shared = right
                .edges
                .iter()
                .find(|edge| edge.side == Side::Start)
                .and_then(|edge| edge.draws);
            for edge in &mut right.edges {
                if edge.side == Side::Start {
                    edge.draws = None;
                }
            }
            for edge in &mut left.edges {
                if edge.side == Side::End {
                    edge.draws = heavier(edge.draws, shared);
                }
            }
        }
        // Word's horizontal rules occupy their thickness: a row is taller by
        // the rule above it and its content starts below the rule. Measured:
        // a 2pt-bordered table starts its text 2pt lower and pitches every
        // row 2pt taller than the borderless twin.
        let rule = |side: Side| -> f64 {
            cells
                .iter()
                .flat_map(|cell| &cell.edges)
                .filter(|edge| edge.side == side)
                .filter_map(|edge| edge.rules.filter(|b| b.style.draws()))
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
        // And what the row below will have to draw, column by column. What
        // this row would have drawn, not what merely rules here: a merge runs
        // through its own rule and the row under it must not put back the
        // line the merge exists to remove.
        above_bottom.iter_mut().for_each(|slot| *slot = None);
        let mut at = row.props.grid_before;
        for (cell, plan) in row.cells.iter().zip(&cells) {
            let bottom = plan
                .edges
                .iter()
                .find(|edge| edge.side == Side::Bottom)
                .and_then(|edge| edge.cuts);
            let span = cell.props.span();
            for slot in above_bottom
                .iter_mut()
                .skip(at as usize)
                .take(span as usize)
            {
                *slot = bottom;
            }
            at += span;
        }
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
        // A merge already under way is that much taller for this row. The
        // rule above it counts: there is no line drawn between the rows of a
        // merge, and the room the line would have taken is the cell's.
        for pending in aligning.iter_mut().filter(|p| p.started < row_index) {
            pending.available += height + rule_above;
        }
        // Vertical alignment is a shift of the cell's lines within the row's
        // final height, which is why it can only be applied once that is known.
        for (index, cell) in cells.iter_mut().enumerate() {
            // **A cell that covers more than one row is aligned in the whole
            // of what it covers**, and the rows below it have not been laid
            // out yet. So the shift waits for the last of them; the lines go
            // down where they are and are moved once the room is known.
            if cell.spans > 1 && cell.align != CellVAlign::Top {
                cell.aligning = Some(aligning.len());
                aligning.push(Aligning {
                    started: row_index,
                    last: row_index + cell.spans - 1,
                    align: cell.align,
                    content: cell.content,
                    available: inner_height,
                    parts: Vec::new(),
                });
                let _ = index;
                continue;
            }
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
                for CellEdge {
                    side, draws, cuts, ..
                } in cell.edges
                {
                    // The row's top edge is drawn once, above the first band,
                    // and its bottom edge once, below the last. Drawing either
                    // on every band would rule a line across the middle of a
                    // cell wherever a page break happened to fall.
                    if (side == Side::Top && !is_first) || (side == Side::Bottom && !is_last) {
                        // Unless a page break cuts the row right here — then
                        // Word closes the fragment with the cell's border.
                        // Pagination knows where the cuts land; the edge goes
                        // along as a maybe.
                        if let Some(border) = cuts.filter(|b| b.style.draws()) {
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
                    if let Some(border) = draws.filter(|b| b.style.draws()) {
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
                        if let Some(pending) = cell.aligning {
                            aligning[pending]
                                .parts
                                .push((into.items.len(), parts.len()));
                        }
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
                space_before: 0.0,
            });
        }
        // The last row of a merge is where its room is finally known, so this
        // is where the lines it was holding go to their place.
        for pending in aligning.iter().filter(|p| p.last == row_index) {
            let offset = cell_offset(pending.align, pending.content, pending.available);
            if offset <= 0.0 {
                continue;
            }
            for &(item, part) in &pending.parts {
                if let Some(placement) = into
                    .items
                    .get_mut(item)
                    .and_then(|item| item.parts.get_mut(part))
                {
                    placement.y += offset;
                }
            }
        }
        aligning.retain(|pending| pending.last > row_index);
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

/// One edge of a cell, and the three different questions asked of it.
///
/// They come apart because a rule is not the property of one cell. It runs
/// between two of them, both of which have an opinion about it, and Word
/// answers each question from a different one.
#[derive(Clone, Copy)]
struct CellEdge {
    side: Side,
    /// What rules here, drawn or not — which is what the row's height is paid
    /// against. A vertical merge draws no line between its own rows and Word
    /// still spaces them as though it did: the letterhead of the demonstration
    /// document rules its cells at a point and a half and its table between
    /// them at a half, and every row under the merged first column sat a point
    /// too high until the undrawn rule was counted here.
    rules: Option<Border>,
    /// What this cell puts down. Nothing where a merge runs through the edge,
    /// and nothing on an edge the row above or the cell to the left has
    /// already drawn: one line is one stroke. Drawn from both sides it is the
    /// same line twice, which on paper is invisible and on a screen darkens
    /// its own anti-aliased edges into a visibly heavier rule.
    draws: Option<Border>,
    /// What closes the edge when a page cut lands on it rather than the row
    /// below. A fragment left on a page has no neighbour to share with, so it
    /// rules itself shut.
    cuts: Option<Border>,
}

/// Of the two borders that meet on one rule, the one Word draws.
///
/// Weight decides it, and a border that draws at all beats one that does not
/// however thick it claims to be — `<w:sz>` survives on a border of style
/// `none`, and a cell that states "no rule here" against a neighbour's hairline
/// is asking for the hairline, not for six points of nothing.
fn heavier(a: Option<Border>, b: Option<Border>) -> Option<Border> {
    let weight = |border: &Option<Border>| match border {
        Some(border) if border.style.draws() => {
            border.size.map(|s| s.points()).unwrap_or(0.5).max(0.0)
        }
        _ => -1.0,
    };
    match weight(&b) > weight(&a) {
        true => b,
        false => a,
    }
}

/// One cell of a row, flowed but not yet placed.
struct CellPlan {
    x: f64,
    width: f64,
    align: CellVAlign,
    fill: Option<[u8; 3]>,
    edges: [CellEdge; 4],
    /// How tall the cell's own content came out.
    content: f64,
    /// How many rows this cell covers, a vertical merge counted from the cell
    /// that starts it. One for every ordinary cell.
    spans: usize,
    /// Which entry of the table's `aligning` list this cell's lines belong to,
    /// for a merge whose alignment is settled once its last row is laid.
    aligning: Option<usize>,
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

/// A merged cell waiting to learn how much room it has to be aligned in.
///
/// **A cell that covers several rows is aligned in all of them**, and a table
/// is laid one row at a time, so the room is not known until the last of them
/// has been measured. The lines are put down where they fall and moved
/// afterwards: nothing else in the row depends on where they sit, and the
/// alternative — measuring the rows twice — would flow every cell of the
/// table a second time.
struct Aligning {
    /// The row the merge starts in, so a row already gone by is not counted
    /// twice into the room.
    started: usize,
    /// The last row of the span, which is where the shift happens.
    last: usize,
    align: CellVAlign,
    content: f64,
    /// The room the merge covers, so far.
    available: f64,
    /// Where the cell's lines were put, by item and by part inside it.
    parts: Vec<(usize, usize)>,
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
/// **Both sides of a cut have to hold a line of their own.** A row's content
/// box is taller than its lines — the cell's own padding stands below the last
/// of them — so the bottom of the last line is a boundary like any other, and
/// cutting there leaves a piece with nothing in it but that padding. Measured
/// against Word on the demonstration document: a table whose heading row very
/// nearly fitted at the foot of a page had all of its words placed there and
/// one and nine tenths of a point of padding sent over, where Word moved the
/// whole row to the next page. A page break is a thing a reader sees, and it
/// has to fall between two things worth seeing.
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
    let lines = |above: bool, at: i64| {
        cells.iter().any(|cell| {
            cell.lines.iter().any(|line| match above {
                true => key(line.top + line.height) <= at,
                false => key(line.top) >= at,
            })
        })
    };
    let points: Vec<f64> = offsets
        .into_iter()
        .filter(|at| {
            *at <= total
                && (*at == 0
                    || *at == total
                    || (!inside(*at) && lines(true, *at) && lines(false, *at)))
        })
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
pub fn paginate(items: &[Item], height: f64, opens_document: bool) -> Vec<usize> {
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
            // See [`Item::space_before`]: an item that opens a page loses it,
            // unless no page ended above it because the document starts here.
            y += item.height - dropped_space(item, end == start, opens_document && start == 0);
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

/// The space above an item that this page will not give it. See
/// [`Item::space_before`].
fn dropped_space(item: &Item, opens_page: bool, opens_document: bool) -> f64 {
    match opens_page && !opens_document {
        true => item.space_before,
        false => 0.0,
    }
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
                let column = page.geometry.width - page.geometry.start - page.geometry.end;
                if !stands_aside(drawing, column) {
                    continue;
                }
                let (x, y) = anchor_position(drawing, &page.geometry, (placement.x, placement.y));
                // Clockwise from the top, which is how `wp:anchor` states its
                // standoffs and how the model keeps them: distT, distR, distB,
                // distL. Read here as top-left-bottom-right, the two sides came
                // out swapped — invisible so far only because every float in the
                // corpus stands off its two sides by the same amount.
                let (above, right, below, left) = drawing.distance;
                let width = drawing.extent.0.points();
                let top = y - above.points();
                let bottom = y + drawing.extent.1.points() + below.points();
                let start = page.geometry.start;
                let end = page.geometry.width - page.geometry.end;
                // The stretch of the measure the float covers, standoffs and
                // all. Where it reaches a margin that is an indent or an inset
                // — the line is simply narrower — and where it stands clear of
                // both it is a hole, with text on either side of it. Word sets
                // both channels of such a line, and choosing the wider side
                // instead threw every word of the narrow one across the page:
                // 320 points, on `floating-image-wrap.docx`.
                let covers = (
                    (x - left.points() - start).max(0.0),
                    (x + width + right.points() - start).min(end - start),
                );
                let (indent, inset, hole) = match covers {
                    (from, to) if from <= 0.01 => (to, 0.0, None),
                    (from, to) if to >= end - start - 0.01 => (0.0, end - start - from, None),
                    (from, to) => (0.0, 0.0, Some((from, to))),
                };
                for &(paragraph, paragraph_top) in &tops {
                    if paragraph_top >= bottom - 0.01 {
                        continue;
                    }
                    wraps.add(
                        paragraph,
                        inline::Obstacle {
                            // A paragraph may well have begun above the float —
                            // the picture in `floating-image-wrap.docx` is
                            // anchored a hundred points into the paragraph that
                            // holds it — and saying so is what lets the float
                            // stop reserving its height in the flow.
                            from: (top - paragraph_top).max(0.0),
                            depth: bottom - paragraph_top,
                            indent,
                            inset,
                            hole,
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
    /// Two floats beside one paragraph, as the one band they come to.
    ///
    /// Every field, and the entry starts empty — so a field left out here is a
    /// float's whole effect quietly discarded, which is what happened to the
    /// band's start and its hole when they were added to [`inline::Obstacle`]
    /// and not to this.
    fn add(&mut self, paragraph: usize, beside: inline::Obstacle) {
        let slot = self.beside.entry(paragraph).or_insert(beside);
        slot.from = slot.from.min(beside.from);
        slot.depth = slot.depth.max(beside.depth);
        slot.indent = slot.indent.max(beside.indent);
        slot.inset = slot.inset.max(beside.inset);
        // A line cannot be parted twice: nothing here draws a line in three
        // pieces, and nothing measures one. The first float to ask for a hole
        // keeps it, and a second alongside it narrows the line instead.
        slot.hole = slot.hole.or(beside.hole);
    }
}

/// Whether text is set *beside* this drawing rather than below it.
///
/// A float that travels with the text reserves its height instead — see
/// [`displaces`], which is the other half of this decision and the stated
/// limit above. This is the other case: anchored to the page or to a margin,
/// where it stays put and the text has to go round.
fn stands_aside(drawing: &wp_model::Drawing, measure: f64) -> bool {
    use wp_model::doc::Wrap;
    if drawing.behind_text || displaces(drawing, measure) {
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

/// Where an anchored drawing sits on the page.
///
/// `origin` is where the paragraph the drawing hangs off begins: the left of
/// the column it is set in, and the top of its first line. **Both are needed
/// because "the column" is not always the page's text column** — a shape
/// anchored inside a table cell measures from that cell's text, which is how
/// Word draws the page frame of a document whose letterhead is a table. The
/// page's own margins answer for everything measured against the page.
pub fn anchor_position(
    drawing: &wp_model::Drawing,
    page: &PageBox,
    origin: (f64, f64),
) -> (f64, f64) {
    use wp_model::doc::{Alignment, RelativeTo};

    let (column, line_top) = origin;
    let Some(position) = &drawing.position else {
        return (column, line_top);
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
                // The column the anchoring paragraph is set in, which is the
                // page's text column everywhere but inside a table.
                RelativeTo::Column => column,
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
pub fn anchor_base(drawing: &wp_model::Drawing, page: &PageBox, origin: (f64, f64)) -> (f64, f64) {
    use wp_model::doc::RelativeTo;

    let (column, line_top) = origin;
    let Some(position) = &drawing.position else {
        return (column, line_top);
    };
    let x = match position.horizontal.relative_to {
        RelativeTo::Page => 0.0,
        RelativeTo::RightMargin => page.width - page.end,
        RelativeTo::Column => column,
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
/// **Only where there is no measure left to set text into.** A square- or
/// tight-wrapped float is a rectangle the text goes round, which is the one
/// mechanism Word has and the one this engine now has; reserving its height in
/// the flow instead is a second mechanism, and it is right only where going
/// round is impossible. A picture as wide as the column is such a case, and it
/// is the commonest float in the wild — `file-sample_500kB.docx` fills its 468
/// point column to the point and Word does set the text below it.
///
/// A float that leaves room and reserved anyway put the whole of its paragraph
/// a picture's height too low, its own inline neighbours included: 120 points,
/// on `floating-image-wrap.docx`, which is exactly the picture's height.
///
/// **Where the boundary really lies is not measured.** The evidence brackets it
/// and no more: at −18 points of leftover measure Word sets the text below, and
/// at +290 it sets it beside. Anything between is a guess, so the least is
/// claimed — any room at all is room — rather than inventing a threshold the
/// oracle has never been asked about. `tools/word-probe` and a document built
/// by `corpus/generate.ps1` would settle it.
///
/// A float positioned relative to the page or a margin does not travel with the
/// text, so its space cannot be reserved mid-flow and it stays an overlay.
fn displaces(drawing: &wp_model::Drawing, measure: f64) -> bool {
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
        // Above and below only, whatever room it leaves: that is what the wrap
        // says, and ECMA-376 Part 1 §20.4.2 keeps it apart from the three that
        // mean the text goes round.
        Wrap::TopAndBottom => true,
        Wrap::Square | Wrap::Tight => !room_beside(drawing, measure),
        Wrap::None => false,
    }
}

/// Whether a float leaves any measure at all to set text into beside it.
///
/// The float's own width and both its standoffs against the measure it stands
/// in, rather than the room on either particular side of it: a centred picture
/// leaves half its leftover on each hand, and asking after one side alone
/// understates it by half. What this answers is the only question
/// [`displaces`] needs — whether going round is possible.
fn room_beside(drawing: &wp_model::Drawing, measure: f64) -> bool {
    let (_, right, _, left) = drawing.distance;
    measure - drawing.extent.0.points() - left.points() - right.points() > 0.0
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

/// Whether a cell holds a shape that floats, at any depth.
fn holds_a_float(blocks: &[Block]) -> bool {
    blocks.iter().any(|block| match block {
        Block::Paragraph(paragraph) => !anchored(paragraph).is_empty(),
        Block::Table(table) => table
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .any(|cell| holds_a_float(&cell.content)),
        Block::Structured(sdt) => holds_a_float(&sdt.content),
        _ => false,
    })
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
            contents: Box::leak(Box::new(crate::field::Contents::default())),
            default_tab: Twips(720),
            no_leading: false,
            no_tab_for_hanging_indent: false,
            fallback_font: "test",
            has_face: |_| false,
            show_revisions: true,
            show_hidden: false,
            fields: Box::leak(Box::new(crate::field::FieldValues::default())),
            band: None,
            memo: None,
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
            .filter_map(|(_, p)| match p.kind {
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
    fn a_page_break_before_anything_else_takes_the_whole_paragraph_with_it() {
        // There is nothing in front of the break to leave behind, so Word does
        // not leave an empty line on the page it came from — and the space
        // above the paragraph is spent where the paragraph now is. Measured on
        // the demonstration document: its headings begin with a page break and
        // keep their lines together, and every line of the page after one sat
        // twelve points high, under a page of its own holding one blank line.
        let broken = |space: f64| {
            let mut paragraph = Paragraph {
                content: vec![Inline::Run(Run {
                    content: vec![Piece::Break(Break::Page), Piece::Text("omega".into())],
                    ..Run::new()
                })],
                ..Paragraph::new()
            };
            paragraph.props.spacing.before = Some(wp_model::units::Twips((space * 20.0) as i32));
            paragraph.props.keep_lines = Some(true);
            let mut document = document(vec![
                Block::Paragraph(Paragraph::of("alpha")),
                Block::Paragraph(paragraph),
            ]);
            document.section = page_of(400);
            let pages = pages(&document);
            let where_is = |want: &str| {
                pages
                    .iter()
                    .enumerate()
                    .flat_map(|(n, page)| page.content.iter().map(move |p| (n, p)))
                    .find_map(|(n, placement)| match &placement.kind {
                        Placed::Line { line, .. }
                            if line.fragments.iter().any(|f| {
                                matches!(&f.content,
                                crate::inline::Content::Text { text, .. } if text.contains(want))
                            }) =>
                        {
                            Some((n, placement.y))
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("{want} was never drawn"))
            };
            (pages.len(), where_is("alpha"), where_is("omega"))
        };
        let (count, first, after) = broken(24.0);
        assert_eq!(count, 2, "one break makes one new page, not two");
        assert_eq!(first.0, 0, "and what came before it stays where it was");
        assert_eq!(after.0, 1);
        let (_, _, tight) = broken(0.0);
        assert!(
            (after.1 - tight.1 - 24.0).abs() < 0.01,
            "the space above the paragraph is spent on the page it moved to, \
             not under the empty line it would have left behind: {} against {}",
            after.1,
            tight.1
        );
    }

    #[test]
    fn a_page_does_not_space_away_from_its_top_what_simply_fell_onto_it() {
        // Word, measured: a paragraph set twenty-four points before begins at
        // the top margin exactly when the page above it ran out of room, keeps
        // all twenty-four when the writer typed the break himself, and keeps
        // them again at the very start of the document, where no page ended.
        // Read off `table_render_test.doc`, whose headings are all set twelve
        // points before: the two that carry their own page break stand twelve
        // points down its page and the two that fell there stand at the top.
        let spaced = |text: &str| {
            let mut paragraph = Paragraph::of(text);
            paragraph.props.spacing.before = Some(Twips::from_points(20.0));
            Block::Paragraph(paragraph)
        };
        let mut blocks = vec![spaced("first")];
        blocks.extend(paragraphs(5));
        blocks.push(spaced("fell"));
        let mut document = document(blocks);
        document.section = page_of(6);
        let pages = pages(&document);
        assert_eq!(pages.len(), 2, "six lines of room and seven paragraphs");
        assert_eq!(
            pages[0].content[0].y,
            pages[0].geometry.top + 20.0,
            "the document's own first paragraph keeps its space"
        );
        assert_eq!(
            pages[1].content[0].y, pages[1].geometry.top,
            "and the one that only fell onto the second page does not"
        );
    }

    #[test]
    fn a_row_is_not_cut_where_one_side_of_the_cut_would_hold_no_line() {
        // The bottom of the last line is a boundary like any other, and it is
        // not a place to break: below it is the cell's padding and nothing
        // else. Word, measured on `table_render_test.doc`: a heading row that
        // very nearly fitted at the foot of page five moved whole to page six,
        // where this had placed all of its words and sent one and nine tenths
        // of a point of padding over.
        let cell = |heights: &[f64]| {
            let mut top = 0.0;
            CellPlan {
                x: 0.0,
                width: 100.0,
                align: wp_model::table::CellVAlign::Top,
                fill: None,
                edges: std::array::from_fn(|n| CellEdge {
                    side: [Side::Top, Side::Start, Side::Bottom, Side::End][n],
                    rules: None,
                    draws: None,
                    cuts: None,
                }),
                content: heights.iter().sum(),
                spans: 1,
                aligning: None,
                lines: heights
                    .iter()
                    .map(|height| {
                        let line = CellLine {
                            top,
                            height: *height,
                            parts: Vec::new(),
                        };
                        top += height;
                        line
                    })
                    .collect(),
            }
        };
        assert_eq!(
            split_points(&[cell(&[12.0])], 16.0),
            vec![0.0, 16.0],
            "one line and some padding is a row that travels whole"
        );
        assert_eq!(
            split_points(&[cell(&[12.0, 12.0])], 28.0),
            vec![0.0, 12.0, 28.0],
            "two lines may be parted between them, and still not below the second"
        );
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

    /// A table whose first column is merged down both rows and holds one
    /// line, beside a second column of one line and then three.
    fn merged_over_two_rows(content: Vec<Block>, align: CellVAlign) -> Document {
        let mut merged = Cell {
            props: CellProps::new(),
            content,
        };
        merged.props.v_merge = Some(VMerge::Restart);
        merged.props.v_align = align;
        let mut below = cell("");
        below.props.v_merge = Some(VMerge::Continue);
        let tall = Cell {
            props: CellProps::new(),
            content: vec![
                Block::Paragraph(Paragraph::of("a")),
                Block::Paragraph(Paragraph::of("b")),
                Block::Paragraph(Paragraph::of("c")),
            ],
        };
        let table = Table {
            grid: vec![Twips(2880), Twips(2880)],
            rows: vec![
                Row {
                    cells: vec![merged, cell("two")],
                    ..Row::new()
                },
                Row {
                    cells: vec![below, tall],
                    ..Row::new()
                },
            ],
            ..Table::new()
        };
        document(vec![Block::Table(table)])
    }

    #[test]
    fn a_merged_cell_is_centred_in_all_the_rows_it_covers() {
        // Measured against Word on the demonstration document's letterhead:
        // its `ENGINEERING SPECIFICATIONS` covers the first two rows of the
        // table and Word centres it in both of them together, four and two
        // thirds of a point below where centring it in the first row alone
        // leaves it. Here the second row is three lines to the first row's
        // one, so a line centred in all four sits exactly between the second
        // row's first and second lines.
        let document = merged_over_two_rows(
            vec![Block::Paragraph(Paragraph::of("one"))],
            CellVAlign::Center,
        );
        let lines = cell_lines(&document);
        let at = |want: &str| {
            lines
                .iter()
                .find(|(_, text)| text == want)
                .map(|(y, _)| *y)
                .unwrap_or_else(|| panic!("{want} was never drawn: {lines:?}"))
        };
        let middle = (at("a") + at("b")) / 2.0;
        assert!(
            (at("one") - middle).abs() < 0.01,
            "the merged line is at {} where the middle of its two rows is {middle}",
            at("one")
        );
    }

    #[test]
    fn a_cell_that_holds_a_floating_shape_is_not_aligned_at_all() {
        // Word leaves such a cell's content at the top whatever the cell says
        // — the demonstration document's letterhead anchors its page frame and
        // its watermark in the cell its address sits in, and Word leaves the
        // address at the top of a merge four rows deep. Take the shapes out
        // of that one cell and Word centres it.
        let drawing = wp_model::Drawing {
            source: Vec::new().into(),
            anchored: true,
            extent: (
                wp_model::Emu::from_points(10.0),
                wp_model::Emu::from_points(10.0),
            ),
            rel: Some("rId7".into()),
            chart: None,
            name: None,
            description: None,
            wrap: wp_model::Wrap::None,
            distance: Default::default(),
            position: None,
            behind_text: true,
            text: None,
            tone: None,
            outline: None,
        };
        let paragraph = Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![Piece::Text("one".into()), Piece::Drawing(Box::new(drawing))],
                ..Run::new()
            })],
            ..Paragraph::new()
        };
        let document = merged_over_two_rows(vec![Block::Paragraph(paragraph)], CellVAlign::Center);
        let lines = cell_lines(&document);
        let at = |want: &str| {
            lines
                .iter()
                .find(|(_, text)| text == want)
                .map(|(y, _)| *y)
                .unwrap_or_else(|| panic!("{want} was never drawn: {lines:?}"))
        };
        assert!(
            (at("one") - at("two")).abs() < 0.01,
            "the shape's cell was aligned to {} where the row starts at {}",
            at("one"),
            at("two")
        );
    }

    #[test]
    fn no_rule_is_drawn_between_the_rows_of_a_vertical_merge() {
        // A merged cell is one tall cell to the reader, so the edge where its
        // rows meet is not drawn at either end of it — while the column beside
        // it is ruled as usual. The demonstration document says so in words:
        // "To the left is a table inside a table, with some cells merged",
        // beside a cell whose "One" and "Three" had a line between them.
        let border = Border {
            style: wp_model::prop::BorderStyle::Single,
            size: Some(wp_model::units::Eighth(4)),
            color: None,
            space: None,
            shadow: false,
        };
        let mut merged = cell("one");
        merged.props.v_merge = Some(VMerge::Restart);
        let mut below = cell("");
        below.props.v_merge = Some(VMerge::Continue);
        let mut table = Table {
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
        table.props.borders.top = Some(border);
        table.props.borders.bottom = Some(border);
        table.props.borders.inside_h = Some(border);
        let document = document(vec![Block::Table(table)]);
        let mut across: std::collections::BTreeMap<i64, usize> = std::collections::BTreeMap::new();
        for page in pages(&document) {
            for placement in &page.content {
                if matches!(
                    placement.kind,
                    Placed::Edge {
                        side: Side::Top | Side::Bottom,
                        ..
                    }
                ) {
                    *across.entry(placement.x.round() as i64).or_default() += 1;
                }
            }
        }
        let counts: Vec<usize> = across.values().copied().collect();
        assert_eq!(
            counts.len(),
            2,
            "one column of horizontal rules each: {across:?}"
        );
        assert_eq!(
            counts[0], 2,
            "the merged column is closed above and below and nowhere between"
        );
        assert!(
            counts[1] > counts[0],
            "while the column beside it keeps the rule between its rows"
        );
    }

    #[test]
    fn the_rule_between_two_cells_is_drawn_once_and_by_the_heavier_of_them() {
        // Drawn from both sides it is the same line twice, which paper hides
        // and a screen does not: the second stroke darkens the first one's
        // anti-aliased edges until a hairline between columns reads as heavy
        // as the frame around them. Word draws one line and gives a contested
        // edge to the heavier border, so the point and a half the row above
        // states beats the half point the table would have ruled.
        let weight = |eighths| Border {
            style: wp_model::prop::BorderStyle::Single,
            size: Some(wp_model::units::Eighth(eighths)),
            color: None,
            space: None,
            shadow: false,
        };
        let heavy = |text: &str| {
            let mut cell = cell(text);
            cell.props.borders.bottom = Some(weight(12));
            cell.props.borders.end = Some(weight(12));
            cell
        };
        let mut table = Table {
            grid: vec![Twips(2880), Twips(2880)],
            rows: vec![
                Row {
                    cells: vec![heavy("one"), heavy("two")],
                    ..Row::new()
                },
                Row {
                    cells: vec![cell("three"), cell("four")],
                    ..Row::new()
                },
            ],
            ..Table::new()
        };
        table.props.borders.inside_h = Some(weight(4));
        table.props.borders.inside_v = Some(weight(4));
        let document = document(vec![Block::Table(table)]);
        // Where each stroke actually lands, at the eighth of a point, so that
        // two laid on one another are one key with two weights against it.
        let mut seen: std::collections::BTreeMap<(bool, i64, i64), Vec<f64>> = Default::default();
        for page in pages(&document) {
            for placement in &page.content {
                let Placed::Edge { border, side } = &placement.kind else {
                    continue;
                };
                let upright = matches!(side, Side::Start | Side::End);
                let (x, y) = match side {
                    Side::Start => (placement.x, placement.y),
                    Side::End => (placement.x + placement.width, placement.y),
                    Side::Top => (placement.x, placement.y),
                    Side::Bottom => (placement.x, placement.y + placement.height),
                };
                let at = ((x * 8.0).round() as i64, (y * 8.0).round() as i64);
                seen.entry((upright, at.0, at.1))
                    .or_default()
                    .push(border.size.map(|s| s.points()).unwrap_or(0.5));
            }
        }
        assert!(
            seen.values().all(|at| at.len() == 1),
            "no two rules are laid on one another: {seen:?}"
        );
        assert!(
            seen.iter()
                .any(|((upright, ..), at)| *upright && at == &[1.5]),
            "the column between the two cells is ruled at the heavier of the \
             two edges that meet down it: {seen:?}"
        );
        assert!(
            seen.iter()
                .any(|((upright, ..), at)| !*upright && at == &[1.5]),
            "and so is the edge between the two rows: {seen:?}"
        );
    }

    #[test]
    fn a_row_is_spaced_by_the_rule_above_it_even_where_a_merge_draws_none() {
        // Word charges a row for the heaviest border on its top edge across
        // every cell of it, including one whose rule is not drawn at all
        // because a vertical merge runs through it. Measured on the letterhead
        // of the demonstration document, which rules its cells at a point and
        // a half and its table between them at a half: every row under the
        // merged first column sat a point too high while the undrawn rule was
        // going uncounted.
        let weight = |eighths| Border {
            style: wp_model::prop::BorderStyle::Single,
            size: Some(wp_model::units::Eighth(eighths)),
            color: None,
            space: None,
            shadow: false,
        };
        let laid_out = |merged_rules: Option<Border>| {
            let mut merged = cell("");
            merged.props.v_merge = Some(VMerge::Restart);
            merged.props.borders.bottom = merged_rules;
            let mut below = cell("");
            below.props.v_merge = Some(VMerge::Continue);
            below.props.borders.top = merged_rules;
            let mut table = Table {
                grid: vec![Twips(2880), Twips(2880)],
                rows: vec![
                    Row {
                        cells: vec![merged, cell("above")],
                        ..Row::new()
                    },
                    Row {
                        cells: vec![below, cell("below")],
                        ..Row::new()
                    },
                ],
                ..Table::new()
            };
            table.props.borders.inside_h = Some(weight(4));
            let document = document(vec![Block::Table(table)]);
            let lines = cell_lines(&document);
            let at = |want: &str| {
                lines
                    .iter()
                    .find(|(_, text)| text == want)
                    .map(|(y, _)| *y)
                    .unwrap_or_else(|| panic!("{want} was never drawn: {lines:?}"))
            };
            at("below") - at("above")
        };
        let hairline = laid_out(None);
        let heavy = laid_out(Some(weight(12)));
        assert!(
            (heavy - hairline - 1.0).abs() < 0.01,
            "a point and a half where the merge hides it, against the half \
             point the table rules, is a point further down: {hairline} then {heavy}"
        );
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
        // The band is flowed from zero, so its line's paragraph number means
        // nothing without the body it was counted through.
        assert_eq!(page.header_body, Some(wp_model::HeaderId(0)));
        assert_eq!(page.footer_body, None);
        let scope = wp_model::Scope::Chrome(wp_model::HeaderId(0));
        assert_eq!(page.placements(scope).len(), 1, "the band is its own flow");
        assert!(
            page.placements(wp_model::Scope::Body)
                .iter()
                .all(|placement| placement.y >= page.geometry.top),
            "and none of it is the body's"
        );
    }

    #[test]
    fn a_section_that_names_no_header_shows_the_one_the_section_before_it_does() {
        // Word's "Link to Previous", on the wire: asked to link, Word writes a
        // `<w:sectPr>` with no reference in it at all — so a reader that takes
        // silence for "no header" leaves every page after the first section
        // break bare. Measured against a two-section document Word itself
        // wrote and this application read back.
        let mut opening = Paragraph::of("section one");
        let mut first = page_of(20);
        first.headers.push(wp_model::HeaderRef {
            kind: HeaderKind::Default,
            body: wp_model::HeaderId(0),
            rel: None,
        });
        opening.section = Some(Box::new(first));

        let mut document = document(vec![
            Block::Paragraph(opening),
            Block::Paragraph(Paragraph::of("section two")),
        ]);
        // The trailing section is the linked one: it names nothing.
        document.section = page_of(20);
        document.headers.push(wp_model::HeaderFooter {
            id: wp_model::HeaderId(0),
            part: None,
            rel: None,
            footer: false,
            content: vec![Block::Paragraph(Paragraph::of("running head"))],
        });

        let pages = pages(&document);
        assert_eq!(pages.len(), 2, "one page per section");
        assert_eq!(pages[0].header_body, Some(wp_model::HeaderId(0)));
        assert_eq!(
            pages[1].header_body,
            Some(wp_model::HeaderId(0)),
            "the linked section carries the same running head"
        );
        assert_eq!(pages[1].header.len(), 1, "and it is drawn there");
    }

    #[test]
    fn unlinking_one_kind_of_band_leaves_the_other_two_inherited() {
        // What Word writes when only the primary header is unlinked: one
        // `<w:headerReference w:type="default">` for that section and nothing
        // else, with the first-page and even-page bands still the previous
        // section's.
        let mut opening = Paragraph::of("section one");
        let mut first = page_of(20);
        first.title_page = true;
        first.headers.push(wp_model::HeaderRef {
            kind: HeaderKind::Default,
            body: wp_model::HeaderId(0),
            rel: None,
        });
        first.headers.push(wp_model::HeaderRef {
            kind: HeaderKind::First,
            body: wp_model::HeaderId(1),
            rel: None,
        });
        opening.section = Some(Box::new(first));

        let mut document = document(vec![
            Block::Paragraph(opening),
            Block::Paragraph(Paragraph::of("section two")),
        ]);
        document.section = page_of(20);
        document.section.title_page = true;
        document.section.headers.push(wp_model::HeaderRef {
            kind: HeaderKind::Default,
            body: wp_model::HeaderId(2),
            rel: None,
        });
        for id in 0..3u32 {
            document.headers.push(wp_model::HeaderFooter {
                id: wp_model::HeaderId(id),
                part: None,
                rel: None,
                footer: false,
                content: vec![Block::Paragraph(Paragraph::of("head"))],
            });
        }

        let shown = document.bands();
        assert_eq!(
            shown[1].header(HeaderKind::Default),
            Some(wp_model::HeaderId(2))
        );
        assert_eq!(
            shown[1].header(HeaderKind::First),
            Some(wp_model::HeaderId(1)),
            "the first-page band is still linked"
        );
        assert_eq!(
            shown[1].header(HeaderKind::Even),
            None,
            "and nothing names one"
        );
    }

    #[test]
    fn a_paragraph_holding_a_page_number_is_laid_again_and_never_recalled() {
        // What a `{ PAGE }` draws is settled by the page it lands on, and that
        // is decided after the paragraph has been laid. A memo that kept it
        // would draw the number of wherever it used to be — which is the one
        // way this cache could be wrong without any paragraph having changed.
        let field = Paragraph {
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
        };
        let mut blocks = paragraphs(9);
        blocks.insert(6, Block::Paragraph(field));
        let mut document = document(blocks);
        document.section = page_of(4);

        let theme = document.theme.clone();
        let memo = crate::Memo::new();
        let ctx = Context {
            memo: Some(&memo),
            ..ctx(&theme)
        };
        let mut shaper = crate::shape::Fixed;
        let first = layout(&document, &ctx, &mut shaper);
        let again = layout(&document, &ctx, &mut shaper);
        assert_eq!(again, first, "the same document, laid twice");
        assert!(
            first
                .iter()
                .any(|page| drawn(page).contains(&"2".to_string())),
            "the field drew the page it landed on"
        );
        let (hits, misses) = memo.tally();
        assert!(hits >= 9, "the nine plain paragraphs were recalled: {hits}");
        assert!(
            misses > 0,
            "the field's own paragraph was laid again rather than recalled"
        );
    }

    /// Every string of text drawn on a page, in placement order.
    fn drawn(page: &Page) -> Vec<String> {
        let mut out = Vec::new();
        for placement in page.everything() {
            let Placed::Line { line, .. } = &placement.kind else {
                continue;
            };
            for fragment in &line.fragments {
                if let crate::inline::Content::Text { text, .. } = &fragment.content {
                    out.push(text.clone());
                }
            }
        }
        out
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
            contents: &'a crate::field::Contents,
        ) -> Context<'a> {
            Context {
                theme,
                styles: &document.styles,
                notes: marks,
                note_mark: None,
                contents,
                table_part: None,
                default_tab: document.settings.default_tab_stop,
                no_leading: document.settings.no_leading,
                no_tab_for_hanging_indent: document.settings.no_tab_for_hanging_indent,
                fallback_font: "Calibri",
                has_face: |_| false,
                show_revisions: true,
                show_hidden: false,
                fields,
                band: None,
                memo: None,
                wraps: Box::leak(Box::new(Wraps::default())),
            }
        }

        let empty = crate::FieldValues::default();
        let mut cold = Counting::default();
        let marks = crate::notes::NoteMarks::of(&document);
        let contents = crate::field::Contents::of(&document);
        let pages = layout(
            &document,
            &make(&theme, &document, &empty, &marks, &contents),
            &mut cold,
        );
        let settled = evaluate(&pages, &empty);
        assert!(!settled.is_empty(), "the field was evaluated");

        let mut warm = Counting::default();
        let again = layout(
            &document,
            &make(&theme, &document, &settled, &marks, &contents),
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
            text: None,
            tone: None,
            outline: None,
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
            text: None,
            tone: None,
            outline: None,
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
    fn a_square_float_with_room_beside_it_keeps_the_text_out_of_it() {
        // The float from file-sample_500kB.docx after the user drags it
        // smaller, so that there is measure left beside it. It no longer
        // reserves its height and the text is set alongside — but the fault
        // this guards has not changed since text went *below* instead: the
        // moment a float stops reserving its height without narrowing the
        // lines it stands in, the words are drawn on top of the picture.
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
            text: None,
            tone: None,
            outline: None,
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
        // Beside it: the line stands within the float's band rather than
        // below it, which is the height that is no longer reserved.
        assert!(
            line.y < drawn.y + drawn.height,
            "line at {} should stand beside the float, which ends at {}",
            line.y,
            drawn.y + drawn.height
        );
        // And clear of it. This is the assertion that matters — a line in the
        // band that was not narrowed would put every word over the picture.
        assert!(
            line.x >= drawn.x + drawn.width - 0.01,
            "line starts at {} and the float ends at {}",
            line.x,
            drawn.x + drawn.width
        );
    }

    #[test]
    fn word_art_is_stretched_to_fill_the_shape_it_was_drawn_in() {
        // The fixed shaper's face is half its size wide and exactly its size
        // tall, so "abcd" at 100 points measures 200 by 100. A box 200 wide
        // and 300 tall therefore fits the width at its own size and is three
        // times too short for its height — which is what WordArt does with the
        // difference: it stretches. Word's own watermark on the document this
        // was measured against is Courier New filling 609 points of width in a
        // box 152 points tall, and its letters stand half again as tall as
        // that width alone would make them.
        let mut drawing = wp_model::Drawing {
            source: Vec::new().into(),
            anchored: true,
            extent: (
                wp_model::Emu::from_points(200.0),
                wp_model::Emu::from_points(300.0),
            ),
            rel: None,
            chart: None,
            name: None,
            description: None,
            wrap: wp_model::Wrap::None,
            distance: Default::default(),
            position: None,
            behind_text: true,
            text: None,
            tone: None,
            outline: None,
        };
        drawing.text = Some(Box::new(wp_model::doc::ShapeText {
            text: "abcd".into(),
            font: Some("any".into()),
            color: None,
            bold: false,
            italic: false,
            stretch: true,
            rotation: 315.0,
        }));
        let words = shape_words(&drawing, &mut crate::shape::Fixed).expect("a shape of words");
        assert_eq!(words.font.size, 100.0, "the size fills the width");
        assert_eq!(words.stretch, 3.0, "and the stretch fills the height");
        assert_eq!(words.height, 300.0, "so the words are as tall as the box");
        assert_eq!(words.lead, 0.0, "a shaper with no glyphs fits the line box");
    }

    /// A shaper whose glyphs are known to be inset: half the point size wide
    /// like [`crate::shape::Fixed`], but with a tenth of the size of side
    /// bearing at each end and ink reaching only from the baseline to
    /// six tenths of the size.
    #[derive(Default)]
    struct Inked;

    impl Shaper for Inked {
        fn metrics(&mut self, font: &FontRequest) -> crate::shape::Metrics {
            crate::shape::Fixed.metrics(font)
        }

        fn advances(
            &mut self,
            text: &str,
            font: &FontRequest,
            into: &mut Vec<crate::shape::Advance>,
        ) {
            crate::shape::Fixed.advances(text, font, into)
        }

        fn ink(&mut self, text: &str, font: &FontRequest) -> Option<crate::shape::Ink> {
            let run = text.chars().count() as f64 * font.size * 0.5;
            Some(crate::shape::Ink {
                left: font.size * 0.1,
                right: run - font.size * 0.1,
                top: font.size * 0.6,
                bottom: 0.0,
            })
        }
    }

    #[test]
    fn word_art_fills_the_shape_with_its_ink_and_not_with_its_line_box() {
        // Word, measured: a WordArt shape 400 by 200 points draws
        // "CONFIDENTIAL", "gypsy", "Hg" and "xxxx" with their *outlines*
        // spanning 400 by 200 in every case — so the box that fills the shape
        // is the ink's, and a face's ascent-plus-descent has nothing to do
        // with it. Fitting the line box instead left an all-capitals
        // watermark two thirds the height Word draws it.
        let mut drawing = wp_model::Drawing {
            source: Vec::new().into(),
            anchored: true,
            extent: (
                wp_model::Emu::from_points(400.0),
                wp_model::Emu::from_points(200.0),
            ),
            rel: None,
            chart: None,
            name: None,
            description: None,
            wrap: wp_model::Wrap::None,
            distance: Default::default(),
            position: None,
            behind_text: true,
            text: None,
            tone: None,
            outline: None,
        };
        drawing.text = Some(Box::new(wp_model::doc::ShapeText {
            text: "abcd".into(),
            font: Some("any".into()),
            color: None,
            bold: false,
            italic: false,
            stretch: true,
            rotation: 0.0,
        }));
        let words = shape_words(&drawing, &mut Inked).expect("a shape of words");

        // Four letters at half the size each, less a tenth at either end:
        // 1.8 sizes of ink, so 400 points of box wants a size of 222.2.
        assert!((words.font.size - 400.0 / 1.8).abs() < 1e-9);
        assert!(
            (words.width - 400.0).abs() < 1e-9,
            "the ink is as wide as the shape"
        );
        assert!(
            (words.height - 200.0).abs() < 1e-9,
            "and as tall as the shape"
        );
        assert!(
            (words.ascent - 200.0).abs() < 1e-9,
            "with the whole of it above the baseline, because this ink has no descender"
        );

        // The pen stands a side bearing to the left of the ink, so the box
        // that gets centred is the letters and not the pen's run.
        let (x, baseline) = words.origin(0.0, 0.0, 400.0, 200.0);
        assert!(
            (x + words.font.size * 0.1).abs() < 1e-9,
            "the pen starts left of the shape by one side bearing"
        );
        assert!(
            (baseline - 200.0).abs() < 1e-9,
            "and the baseline is the ink's own bottom"
        );
    }

    #[test]
    fn a_watermark_is_set_to_the_shapes_width_and_only_pulled_down_it() {
        // The other kind — see [`wp_model::doc::ShapeText::stretch`]. Word,
        // measured on `table_render_test.doc`: a Courier New shape 609.10 by
        // 152.25 points draws `CONFIDENTIAL` with an advance of 50.758 points
        // a letter, which is that face's em at 84.583, and with every letter's
        // outline matching the face's own bounds at that width and at 152.25
        // of height. Twelve of those advances is 609.10 exactly — the shape's
        // width — and 152.25 over 84.583 is the 1.8 the glyphs are drawn tall.
        //
        // `Inked` sets four letters at half the size each, so a box 400 wide
        // wants a size of 200, and a box 300 tall draws them one and a half
        // times as tall as that.
        let mut drawing = wp_model::Drawing {
            source: Vec::new().into(),
            anchored: true,
            extent: (
                wp_model::Emu::from_points(400.0),
                wp_model::Emu::from_points(300.0),
            ),
            rel: None,
            chart: None,
            name: None,
            description: None,
            wrap: wp_model::Wrap::None,
            distance: Default::default(),
            position: None,
            behind_text: true,
            text: None,
            tone: None,
            outline: None,
        };
        drawing.text = Some(Box::new(wp_model::doc::ShapeText {
            text: "abcd".into(),
            font: Some("any".into()),
            color: None,
            bold: false,
            italic: false,
            stretch: false,
            rotation: 0.0,
        }));
        let words = shape_words(&drawing, &mut Inked).expect("a shape of words");
        assert!(
            (words.font.size - 200.0).abs() < 1e-9,
            "the advances fill it"
        );
        assert!(
            (words.stretch - 1.5).abs() < 1e-9,
            "one em is the box's height"
        );
        assert!(
            (words.width - 400.0).abs() < 1e-9 && words.lead == 0.0,
            "the pen starts at the shape's own left edge"
        );

        // `Inked` borrows the fixed shaper's metrics, whose descent is a
        // quarter of the size: fifty points at two hundred, drawn one and a
        // half times as tall, so seventy-five above the shape's foot.
        let (x, baseline) = words.origin(0.0, 0.0, 400.0, 300.0);
        assert!(x.abs() < 1e-9, "which is where the pen goes");
        assert!(
            (baseline - 225.0).abs() < 1e-9,
            "and the baseline stands a descent above the foot: {baseline}"
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
            text: None,
            tone: None,
            outline: None,
        };
        let (_, y) = anchor_position(&drawing, &geometry, (72.0, 400.0));
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
            text: None,
            tone: None,
            outline: None,
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
            text: None,
            tone: None,
            outline: None,
        };
        // The column is 72..540, so its middle is 306 and a 100pt picture
        // starts at 256; the vertical offset is from where the text is.
        assert_eq!(
            anchor_position(&drawing, &geometry, (72.0, 400.0)),
            (256.0, 410.0)
        );
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
