//! The document surface: pages on a desk, a caret, and a selection.
//!
//! Not built from egui layout. The pages are laid out by `wp-layout` and painted
//! directly (`DESIGN.md` §6), because a document's geometry is decided by the
//! document rather than by the widget it happens to be inside.
//!
//! **The page is white whatever the chrome does.** A document is paper. Tinting
//! it to match a dark application theme shows the user a document they did not
//! make, and text the file says is black then vanishes. That is Calx's lesson
//! about the cell canvas, and it is the same lesson.

use ui_kit::egui;
use wp_layout::block::{Page, Placed, Placement};
use wp_layout::inline::Content;
use wp_layout::shape::Shaper;
use wp_model::{Document, Scope};

use crate::drawings::Picked;
use crate::edit::{Caret, Selection};
use crate::shaper::Egui;

/// Space between two pages on the desk, in points.
const GAP: f32 = 16.0;
/// The desk the pages sit on.
const DESK: egui::Color32 = egui::Color32::from_rgb(0x62, 0x62, 0x66);
const PAPER: egui::Color32 = egui::Color32::WHITE;
const EDGE: egui::Color32 = egui::Color32::from_rgb(0xB0, 0xB0, 0xB4);
const SELECTION: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x2A, 0x5C, 0xAA, 0x50);
/// Every match of the find bar's query, Word's own yellow.
const MATCH: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x92, 0x84, 0x28, 0x60);
/// The outline and grips of a selected drawing.
const HANDLE: egui::Color32 = egui::Color32::from_rgb(0x2A, 0x5C, 0xAA);
/// The dashed rule and tag marking an open header or footer.
const BAND: egui::Color32 = egui::Color32::from_rgb(0x7A, 0x7A, 0x82);

/// What one point of the page takes on the glass at 100% — Word's hundred
/// per cent, which is not a point per point.
///
/// Word draws a document inch as 96 device-independent pixels: the logical
/// inch Windows scales up per monitor, so the page grows on a monitor set to
/// 150% exactly as Word's does. egui's points *are* those device-independent
/// pixels — the DPI guard keeps them honest when the window changes monitor —
/// so multiplying the document's 72-to-the-inch points by this makes 100%
/// the size Word shows, on whichever monitor the window is on.
pub const SCALE: f64 = 96.0 / 72.0;

/// The laid-out document, and how it is being looked at.
pub struct View {
    /// 1.0 is 100%: a document inch on 96 logical pixels, as Word's zoom
    /// means it. The screen mapping is this times [`SCALE`].
    pub zoom: f64,
    pub show_marks: bool,
    pub show_revisions: bool,
    pages: Vec<wp_layout::block::Page>,
    /// What the fields came out as last time. See `refresh`.
    settled: wp_layout::FieldValues,
    /// The document revision the pages were laid out for. Laying a hundred pages
    /// out per frame is what makes an editor feel slow, so it is done when the
    /// document changes and not otherwise.
    stamp: u64,
    /// The lines of the paragraphs the last keystroke did not touch.
    ///
    /// Laying out only when the document changes is not enough on its own: the
    /// document changes on every keystroke, and a long one costs a third of a
    /// second to lay. This is what makes the second keystroke cheap. See
    /// [`wp_layout::Memo`].
    memo: wp_layout::Memo,
}

impl Default for View {
    fn default() -> Self {
        View {
            zoom: 1.0,
            show_marks: false,
            show_revisions: true,
            pages: Vec::new(),
            settled: wp_layout::FieldValues::default(),
            stamp: u64::MAX,
            memo: wp_layout::Memo::new(),
        }
    }
}

impl View {
    pub fn pages(&self) -> &[wp_layout::block::Page] {
        &self.pages
    }

    pub fn is_stale(&self, stamp: u64) -> bool {
        self.stamp != stamp
    }

    /// Lays the document out, if it has changed since the last time.
    pub fn refresh(
        &mut self,
        document: &Document,
        fields: &wp_layout::FieldValues,
        stamp: u64,
        shaper: &mut Egui,
    ) {
        if self.stamp == stamp {
            return;
        }
        let theme = document.theme.clone();
        let notes = wp_layout::NoteMarks::of(document);
        let contents = wp_layout::field::Contents::of(document);
        // The application's own strings — file name, author, today's date — are
        // always the fresh ones; only the page numbers are carried over.
        let mut carried = self.settled.clone();
        carried.today = fields.today.clone();
        carried.now = fields.now.clone();
        carried.file_name = fields.file_name.clone();
        carried.author = fields.author.clone();
        carried.title = fields.title.clone();
        let ctx = wp_layout::inline::Context {
            theme: &theme,
            styles: &document.styles,
            notes: &notes,
            note_mark: None,
            contents: &contents,
            table_part: None,
            default_tab: document.settings.default_tab_stop,
            no_leading: document.settings.no_leading,
            close_up_justified: document.settings.compatibility_mode >= 15,
            no_tab_for_hanging_indent: document.settings.no_tab_for_hanging_indent,
            // The font of last resort when nothing in the document names one.
            // Word's is Calibri only because every document it writes carries
            // docDefaults saying so; a file whose defaults are silent — the
            // Google Docs dialect — falls back to Word's ancient default,
            // Times New Roman, and Word renders it that way.
            fallback_font: if document.styles.doc_defaults().run.fonts.ascii.is_some() {
                "Calibri"
            } else {
                "Times New Roman"
            },
            // Symbol and Wingdings bullets keep their private-use characters
            // when the machine's own font files were registered — the same
            // glyphs Word draws — and are translated to Unicode otherwise.
            has_face: |name| ui_kit::fonts::exact_face(name, false, false).is_some(),
            show_revisions: self.show_revisions,
            show_hidden: self.show_marks,
            fields: match self.settled.is_empty() {
                true => fields,
                false => &carried,
            },
            band: None,
            memo: Some(&self.memo),
            wraps: &wp_layout::block::Wraps::default(),
        };
        let pages = wp_layout::block::layout(document, &ctx, shaper);
        self.pages = pages;
        // Remember what the fields came out as, so the next layout starts from
        // the numbers this one arrived at. A `{ PAGE }` that is already right
        // costs one pass instead of two, on every keystroke.
        self.settled = wp_layout::block::evaluate(&self.pages, fields);
        self.stamp = stamp;
    }

    /// Forces the next `refresh` to lay out again.
    ///
    /// A different document's page numbers are not this one's, so the carried
    /// values go with it.
    pub fn invalidate(&mut self) {
        self.stamp = u64::MAX;
        self.settled = wp_layout::FieldValues::default();
        // Another document's lines are not this one's, and its paragraphs stand
        // at the same indices. The memo would answer with them.
        self.memo.forget();
    }

    /// The size of the whole stack of pages, in points before zoom.
    pub fn extent(&self) -> (f64, f64) {
        let width = self
            .pages
            .iter()
            .map(|page| page.geometry.width)
            .fold(0.0f64, f64::max);
        let height: f64 = self
            .pages
            .iter()
            .map(|page| page.geometry.height + GAP as f64)
            .sum();
        (width, height)
    }

    /// Where a page's top-left sits, in points from the top of the stack.
    pub fn page_origin(&self, index: usize) -> (f64, f64) {
        let width = self.extent().0;
        let mut y = GAP as f64;
        for page in self.pages.iter().take(index) {
            y += page.geometry.height + GAP as f64;
        }
        let page_width = self
            .pages
            .get(index)
            .map(|page| page.geometry.width)
            .unwrap_or(width);
        ((width - page_width) / 2.0, y)
    }
}

/// A point in the document's own coordinates: which page, and where on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spot {
    pub page: usize,
    pub x: f64,
    pub y: f64,
}

/// Turns a click into a position in the text.
///
/// The line nearest the click vertically, then the character nearest it
/// horizontally — nearest, not containing, because a click past the end of a
/// line has to land at the end of the line rather than nowhere.
pub fn caret_at(view: &View, scope: Scope, spot: Spot) -> Option<Caret> {
    let page = view.pages.get(spot.page)?;
    let mut best: Option<((f64, f64), &Placement, usize)> = None;
    for placement in page.placements(scope) {
        let Placed::Line { paragraph, .. } = &placement.kind else {
            continue;
        };
        // Vertically first, then horizontally: the row the click is level
        // with decides, and the cell under the pointer breaks the tie.
        // Vertical distance alone sent a click in a right-hand table cell to
        // the left-hand cell whose line shares the same y.
        let score = (
            distance_to(spot.y, placement.y, placement.height),
            distance_to(spot.x, placement.x, placement.width),
        );
        if best.as_ref().is_none_or(|(best, _, _)| score < *best) {
            best = Some((score, placement, *paragraph));
        }
    }
    let (_, placement, paragraph) = best?;
    let Placed::Line { line, .. } = &placement.kind else {
        return None;
    };
    Some(caret_in(view, scope, line, paragraph, spot.x - placement.x))
}

/// Where along `line` a caret goes for a point `x` measured from the line's
/// own left edge.
fn caret_in(
    view: &View,
    scope: Scope,
    line: &wp_layout::inline::Line,
    paragraph: usize,
    x: f64,
) -> Caret {
    let offset = offset_in(line, x);
    // A point past the end of a line that *wrapped* belongs before the space the
    // wrap ate: its byte is on this line and its caret would draw at the start of
    // the next, so the click would look like it did nothing. The last line of a
    // paragraph has no next line and its trailing space is real text the caret
    // has to be able to sit after — which is where Word puts a click out there,
    // and where this used to refuse to go.
    let offset = match offset == line_range(line).1 && wraps_after(view, scope, paragraph, offset) {
        true => before_wrap_space(line, offset),
        false => offset,
    };
    Caret { paragraph, offset }
}

/// The character `spot` is on, named by the offset it starts at.
///
/// Two things separate this from [`caret_at`], and both of them are the
/// difference between "where would a caret go" and "what is the pointer on".
/// A click anywhere on the page has to land somewhere, so `caret_at` takes the
/// nearest line and the nearest place between two letters; a pointer out in
/// the margin is on nothing, and the right-hand half of a letter is still on
/// that letter. Asking the first question of a hyperlink answers that the
/// pointer is past the end of the link whenever it is on the second half of
/// the link's last letter.
pub fn character_over(view: &View, scope: Scope, spot: Spot) -> Option<Caret> {
    let page = view.pages.get(spot.page)?;
    let found = page.placements(scope).iter().find(|placement| {
        matches!(placement.kind, Placed::Line { .. })
            && (placement.x..=placement.x + placement.width).contains(&spot.x)
            && (placement.y..=placement.y + placement.height).contains(&spot.y)
    })?;
    let Placed::Line { line, paragraph } = &found.kind else {
        return None;
    };
    Some(Caret {
        paragraph: *paragraph,
        offset: character_in(line, spot.x - found.x),
    })
}

/// Where the character drawn at `x` starts, without rounding to the nearer
/// side of it. See [`character_over`].
fn character_in(line: &wp_layout::inline::Line, x: f64) -> usize {
    for fragment in &line.fragments {
        let Some(source) = fragment.source else {
            continue;
        };
        if x < fragment.x {
            return source.start;
        }
        if x <= fragment.x + fragment.width {
            let Content::Text { text, advances, .. } = &fragment.content else {
                // A tab or a drawing is one thing and the whole of it is what
                // the pointer is on.
                return source.start;
            };
            let mut at = fragment.x;
            for (index, (byte, _)) in text.char_indices().enumerate() {
                at += advances.get(index).copied().unwrap_or(0.0);
                if x < at {
                    return source.start + byte;
                }
            }
            return source.end;
        }
    }
    line_range(line).1
}

/// Whether another line of the same paragraph carries on from `end`.
fn wraps_after(view: &View, scope: Scope, paragraph: usize, end: usize) -> bool {
    view.pages.iter().any(|page| {
        page.placements(scope).iter().any(|placement| {
            matches!(&placement.kind, Placed::Line { line, paragraph: p }
                if *p == paragraph && line_range(line) != (end, end) && line_range(line).0 == end)
        })
    })
}

/// `end` backed up over the spaces a wrap swallowed.
fn before_wrap_space(line: &wp_layout::inline::Line, end: usize) -> usize {
    let mut end = end;
    for fragment in line.fragments.iter().rev() {
        let (Content::Text { text, .. }, Some(source)) = (&fragment.content, fragment.source)
        else {
            continue;
        };
        if source.end != end {
            break;
        }
        while end > source.start && text.as_bytes().get(end - source.start - 1) == Some(&b' ') {
            end -= 1;
        }
        break;
    }
    end
}

/// How far `at` sits outside the span from `from` to `from + size` — zero
/// anywhere inside it.
fn distance_to(at: f64, from: f64, size: f64) -> f64 {
    if at < from {
        from - at
    } else {
        (at - (from + size)).max(0.0)
    }
}

/// The byte offset of the character nearest `x` along a line.
fn offset_in(line: &wp_layout::inline::Line, x: f64) -> usize {
    let mut last = None;
    for fragment in &line.fragments {
        // A list label is drawn beside the text and is not *in* it: no offset
        // belongs to it, so a click on the bullet falls through to the text.
        let Some(source) = fragment.source else {
            continue;
        };
        if x < fragment.x {
            return source.start;
        }
        if x <= fragment.x + fragment.width {
            return match &fragment.content {
                // Inside a piece of text: walk the advances to the nearest
                // character boundary.
                Content::Text { text, advances, .. } => {
                    let mut at = fragment.x;
                    for (index, (byte, _)) in text.char_indices().enumerate() {
                        let width = advances.get(index).copied().unwrap_or(0.0);
                        if x < at + width / 2.0 {
                            return source.start + byte;
                        }
                        at += width;
                    }
                    // The end the *paragraph* counts, not the end of the drawn
                    // string: small capitals draw a text whose bytes are not the
                    // document's.
                    source.end
                }
                // A tab or a drawing is one thing, and the caret goes to
                // whichever side of it the click was nearer.
                _ => match x < fragment.x + fragment.width / 2.0 {
                    true => source.start,
                    false => source.end,
                },
            };
        }
        last = Some(source.end);
    }
    // Past the end of the line: after its last piece of text.
    last.unwrap_or_else(|| line_range(line).0)
}

/// The line placement the caret belongs to, and which page it is on.
///
/// A wrapped line's end is byte-identical to the next line's start, and the
/// caret there belongs to the line *below*: Down and Home land on that offset,
/// and drawing it at the end of the line above looks like the caret refused to
/// move. Only the paragraph's true end — which no later line shares — keeps
/// the caret on the line that ends there.
fn line_holding(view: &View, scope: Scope, caret: Caret) -> Option<(usize, &Placement)> {
    line_holding_on(view, scope, caret, None)
}

/// The same, looking on one page before any other.
///
/// **A header stands on every page that shows it.** The caret in one therefore
/// has as many places on the screen as there are pages, and the first of them
/// is the wrong one whenever the band was opened from somewhere else in the
/// document: double-clicking the running head on page nine would scroll the
/// window back to page one to show the very words the pointer was already on.
fn line_holding_on(
    view: &View,
    scope: Scope,
    caret: Caret,
    prefer: Option<usize>,
) -> Option<(usize, &Placement)> {
    if let Some(page) = prefer {
        if let Some(found) = view
            .pages
            .get(page)
            .and_then(|_| line_on(view, scope, caret, page))
        {
            return Some(found);
        }
    }
    let mut at_end: Option<(usize, &Placement)> = None;
    for (index, page) in view.pages.iter().enumerate() {
        for placement in page.placements(scope) {
            let Placed::Line { line, paragraph } = &placement.kind else {
                continue;
            };
            if *paragraph != caret.paragraph {
                continue;
            }
            let (start, end) = line_range(line);
            if caret.offset >= start && caret.offset < end {
                return Some((index, placement));
            }
            if caret.offset >= start && caret.offset == end && at_end.is_none() {
                at_end = Some((index, placement));
            }
        }
    }
    at_end
}

/// Where a caret sits on the page, as a vertical stroke.
/// One page's own answer, for [`line_holding_on`].
fn line_on(view: &View, scope: Scope, caret: Caret, index: usize) -> Option<(usize, &Placement)> {
    let page = view.pages.get(index)?;
    let mut at_end: Option<(usize, &Placement)> = None;
    for placement in page.placements(scope) {
        let Placed::Line { line, paragraph } = &placement.kind else {
            continue;
        };
        if *paragraph != caret.paragraph {
            continue;
        }
        let (start, end) = line_range(line);
        if caret.offset >= start && caret.offset < end {
            return Some((index, placement));
        }
        if caret.offset >= start && caret.offset == end && at_end.is_none() {
            at_end = Some((index, placement));
        }
    }
    at_end
}

pub fn caret_rect(view: &View, scope: Scope, caret: Caret) -> Option<(usize, egui::Rect)> {
    caret_rect_on(view, scope, caret, None)
}

/// The same, preferring one page — see [`line_holding_on`].
pub fn caret_rect_on(
    view: &View,
    scope: Scope,
    caret: Caret,
    prefer: Option<usize>,
) -> Option<(usize, egui::Rect)> {
    if let Some((index, placement)) = line_holding_on(view, scope, caret, prefer) {
        if let Placed::Line { line, .. } = &placement.kind {
            let x = x_of(line, caret.offset).unwrap_or(0.0) + placement.x;
            return Some((
                index,
                egui::Rect::from_min_size(
                    egui::pos2(x as f32, placement.y as f32),
                    egui::vec2(1.0, placement.height as f32),
                ),
            ));
        }
    }
    // A paragraph with no text at all still has a line, and the caret goes at
    // its left edge.
    let order = prefer
        .into_iter()
        .chain(0..view.pages.len())
        .filter_map(|index| Some((index, view.pages.get(index)?)));
    for (index, page) in order {
        for placement in page.placements(scope) {
            if let Placed::Line { paragraph, .. } = &placement.kind {
                if *paragraph == caret.paragraph {
                    return Some((
                        index,
                        egui::Rect::from_min_size(
                            egui::pos2(placement.x as f32, placement.y as f32),
                            egui::vec2(1.0, placement.height as f32),
                        ),
                    ));
                }
            }
        }
    }
    None
}

/// The byte range of the *visual* line the caret sits on — what Home and End
/// mean in a paragraph that wraps. The paragraph's own ends are Ctrl+Home and
/// Ctrl+End's business.
pub fn line_span(view: &View, scope: Scope, caret: Caret) -> Option<(usize, usize)> {
    let (_, placement) = line_holding(view, scope, caret)?;
    let Placed::Line { line, .. } = &placement.kind else {
        return None;
    };
    Some(line_range(line))
}

/// The caret nearest the point `dy` points above or below where `caret` is
/// drawn — measured down the whole stack of pages, so a step can cross from the
/// last line of one page onto the first line of the next.
///
/// A line is only weighed if it is past the one the caret is on, in the
/// direction of travel. Nearest on its own trapped the caret on any page whose
/// text stopped short of the bottom: the point one line below the last line of
/// such a page is nearer to that line than to anything on the page after it, so
/// every further press of Down chose the line it had just come from. The first
/// page of the demonstration document is half empty and the caret could not
/// leave it.
///
/// Arrow keys and Page Up/Down both come here: the only difference is the size
/// of the step.
pub fn step_from(view: &View, scope: Scope, caret: Caret, dy: f64) -> Option<Caret> {
    let (page, rect) = caret_rect(view, scope, caret)?;
    let (page_x, page_y) = view.page_origin(page);
    let x = page_x + rect.min.x as f64;
    let from = page_y + rect.center().y as f64;
    let want = from + dy;
    let down = dy > 0.0;
    let mut best: Option<((f64, f64), &Placement, usize, f64)> = None;
    for (index, page) in view.pages.iter().enumerate() {
        let (origin_x, origin_y) = view.page_origin(index);
        for placement in page.placements(scope) {
            let Placed::Line { paragraph, .. } = &placement.kind else {
                continue;
            };
            let top = origin_y + placement.y;
            let middle = top + placement.height / 2.0;
            // Half a point of slack, because two cells of one row are level
            // with each other whether or not their lines are exactly so: a
            // step down out of one of them wants the row below, not its
            // neighbour.
            if (middle - from).abs() < 0.5 || down != (middle > from) {
                continue;
            }
            // Vertically first, then horizontally — the same ordering a click
            // is judged by, and for the same reason.
            let score = (
                distance_to(want, top, placement.height),
                distance_to(x, origin_x + placement.x, placement.width),
            );
            if best.as_ref().is_none_or(|(best, ..)| score < *best) {
                best = Some((score, placement, *paragraph, origin_x));
            }
        }
    }
    let (_, placement, paragraph, origin_x) = best?;
    let Placed::Line { line, .. } = &placement.kind else {
        return None;
    };
    Some(caret_in(
        view,
        scope,
        line,
        paragraph,
        x - origin_x - placement.x,
    ))
}

/// The byte range of the paragraph text a line covers.
fn line_range(line: &wp_layout::inline::Line) -> (usize, usize) {
    let mut start = usize::MAX;
    let mut end = 0usize;
    for fragment in &line.fragments {
        if let Some(source) = fragment.source {
            start = start.min(source.start);
            end = end.max(source.end);
        }
    }
    if start == usize::MAX {
        (0, 0)
    } else {
        (start, end)
    }
}

/// How far along a line a byte offset sits.
fn x_of(line: &wp_layout::inline::Line, offset: usize) -> Option<f64> {
    for fragment in &line.fragments {
        let Some(source) = fragment.source else {
            continue;
        };
        if offset < source.start || offset > source.end {
            continue;
        }
        let Content::Text { text, advances, .. } = &fragment.content else {
            // A tab or a drawing has no inside: the caret is at one edge of it
            // or the other.
            return Some(match offset >= source.end && source.end > source.start {
                true => fragment.x + fragment.width,
                false => fragment.x,
            });
        };
        let mut at = fragment.x;
        for (index, (byte, _)) in text.char_indices().enumerate() {
            if source.start + byte >= offset {
                return Some(at);
            }
            at += advances.get(index).copied().unwrap_or(0.0);
        }
        return Some(at);
    }
    line.fragments.first().map(|fragment| fragment.x)
}

/// Every image relationship the view draws, for the decoder to work through.
///
/// Both kinds: a drawing anchored on the page, and one sitting inline in a line
/// of text like an outsized letter.
pub fn image_rels(view: &View) -> Vec<String> {
    let mut rels = Vec::new();
    for page in &view.pages {
        for placement in page.everything() {
            match &placement.kind {
                Placed::Drawing { rel: Some(rel), .. } => rels.push(rel.to_string()),
                Placed::Line { line, .. } => {
                    for fragment in &line.fragments {
                        if let wp_layout::inline::Content::Object { rel: Some(rel), .. } =
                            &fragment.content
                        {
                            rels.push(rel.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    rels
}

/// Every chart relationship the view draws, for the reader to work through.
pub fn chart_rels(view: &View) -> Vec<String> {
    let mut rels = Vec::new();
    for page in &view.pages {
        for placement in page.everything() {
            match &placement.kind {
                // An anchored drawing keeps the whole `Drawing`, chart and all.
                Placed::Drawing {
                    anchor: Some(drawing),
                    ..
                } => rels.extend(drawing.chart.as_deref().map(str::to_string)),
                Placed::Line { line, .. } => {
                    for fragment in &line.fragments {
                        if let wp_layout::inline::Content::Object {
                            chart: Some(rel), ..
                        } = &fragment.content
                        {
                            rels.push(rel.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    rels
}

/// Draws one drawing: a picture, a chart, or the frame that stands in for
/// either when it could not be read.
///
/// A missing picture draws a frame rather than nothing, because a hole in the
/// page is a fact the reader should be able to see — silence would look like a
/// document that never had the picture.
struct Shown<'a> {
    /// The part holding a picture's bytes.
    rel: Option<&'a str>,
    /// The part holding a chart's numbers.
    chart: Option<&'a str>,
    /// The shape's own words, when the shape is words. See [`paint_shape_words`].
    words: Option<&'a wp_layout::block::ShapeWords>,
    /// The shape drawn as itself: the frame round a page of a specification.
    outline: Option<&'a wp_model::doc::ShapeOutline>,
}

fn paint_drawing(
    painter: &egui::Painter,
    pictures: &crate::pictures::Pictures,
    shaper: &mut crate::shaper::Egui,
    shown: Shown<'_>,
    rect: egui::Rect,
    zoom: f32,
) {
    let Shown {
        rel,
        chart,
        words,
        outline,
    } = shown;
    if let Some(outline) = outline {
        paint_shape_outline(painter, outline, rect, zoom);
    }
    if let Some(words) = words {
        paint_shape_words(painter, shaper, words, rect, zoom);
        return;
    }
    if outline.is_some() && rel.is_none() && chart.is_none() {
        return;
    }
    if let Some(plot) = chart.and_then(|rel| pictures.chart(rel)) {
        ui_kit::chart::draw(
            painter,
            rect,
            plot,
            // A document's chart has no cells to read: what the producing
            // application cached is the whole of it.
            &chart::draw::cached_series(plot),
            &ui_kit::chart::Style {
                background: ui_kit::chart::rgb(PAPER),
                outline: ui_kit::chart::rgb(EDGE),
                text: [0, 0, 0],
                grid: ui_kit::chart::rgb(EDGE),
                zoom: f64::from(zoom),
                label: chart::draw::plain_label,
            },
        );
        return;
    }
    if let Some(picture) = rel.and_then(|rel| pictures.played(rel)) {
        paint_metafile(painter, shaper, picture, rect);
        return;
    }
    paint_image(painter, pictures, rel, rect);
}

/// Draws a played metafile: the diagram a document pasted in, as the calls
/// that drew it.
///
/// The recording states its own natural size, so the box decides the scale and
/// nothing here needs to know the zoom — a rectangle twice as wide is a
/// drawing twice as large, type and line widths included. `wp_print::ops`
/// makes the same translation for paper, which is what keeps the two agreeing
/// about where every line and every label goes. They part company on one
/// thing: paper sets a label at the advances the recording states, and the
/// screen lets epaint set it, because a galley is laid as a galley. A label a
/// hair wide on the screen is worth more than one drawn glyph by glyph.
fn paint_metafile(
    painter: &egui::Painter,
    shaper: &mut crate::shaper::Egui,
    picture: &metafile::Picture,
    rect: egui::Rect,
) {
    let scale = (
        f64::from(rect.width()) / picture.size.0,
        f64::from(rect.height()) / picture.size.1,
    );
    let along = (scale.0 + scale.1) / 2.0;
    let place = |point: &(f64, f64)| {
        egui::pos2(
            rect.left() + (point.0 * scale.0) as f32,
            rect.top() + (point.1 * scale.1) as f32,
        )
    };
    let ink = |rgb: [u8; 3]| egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
    for prim in &picture.prims {
        match prim {
            metafile::Prim::Fill { points, rgb } => {
                painter.add(egui::Shape::convex_polygon(
                    points.iter().map(place).collect(),
                    ink(*rgb),
                    egui::Stroke::NONE,
                ));
            }
            metafile::Prim::Stroke { points, rgb, width } => {
                // A hairline still has to be seen: a diagram whose lines are a
                // fifth of a pixel wide would be a blank rectangle.
                let thickness = ((width * along) as f32).max(0.5);
                painter.add(egui::Shape::line(
                    points.iter().map(place).collect(),
                    egui::Stroke::new(thickness, ink(*rgb)),
                ));
            }
            metafile::Prim::Text {
                x,
                baseline,
                text,
                family,
                size,
                bold,
                italic,
                rgb,
                rotation,
                ..
            } => {
                let request = wp_layout::FontRequest {
                    family: family.as_str().into(),
                    size: size * along,
                    bold: *bold,
                    italic: *italic,
                    kern: false,
                };
                let font = shaper.font_id(&request);
                let galley = painter.layout_no_wrap(text.clone(), font, ink(*rgb));
                // The metafile puts its reference point on the baseline and
                // egui puts a galley by its top-left corner, so the face's own
                // ascent is the difference — the same arithmetic the page's
                // own lines are painted by.
                let at = place(&(*x, *baseline));
                let up = shaper.metrics(&request).ascent as f32;
                let mut shape =
                    egui::epaint::TextShape::new(egui::pos2(at.x, at.y - up), galley, ink(*rgb));
                shape.angle = rotation.to_radians() as f32;
                painter.add(shape);
            }
        }
    }
}

/// Draws a shape that *is* its words: a piece of WordArt, and the watermark
/// Word writes with one.
///
/// The face, the size, the angle and the stretch were all decided by the layout
/// — see [`wp_layout::block::ShapeWords`] — so that the page on paper is the
/// page on the screen. What is left here is placing epaint's own galley about
/// the middle of the shape, which is not where epaint turns text about.
///
/// **A stretched piece of WordArt cannot be an ordinary text shape**: epaint
/// will turn a galley but not squash one, and WordArt is type squashed to fill
/// its box. So the galley is tessellated once, unturned, and its vertices are
/// stretched and turned here — which is what a stretch *is*, the glyph drawn at
/// a different proportion rather than at a different size.
///
/// **The pen, not the galley's middle.** What is centred in the shape is the
/// box the layout fitted — the ink's, where the face's outlines could be read
/// — and a galley's own rectangle is the line box, which for an all-capitals
/// watermark is half again as tall. So the placement is asked of
/// [`wp_layout::block::ShapeWords::origin`], the same answer paper works from,
/// and the galley is hung off its first baseline. Anything else and the screen
/// and the printer disagree about where a watermark sits.
fn paint_shape_words(
    painter: &egui::Painter,
    shaper: &mut crate::shaper::Egui,
    words: &wp_layout::block::ShapeWords,
    rect: egui::Rect,
    zoom: f32,
) {
    let colour = egui::Color32::from_rgb(words.rgb[0], words.rgb[1], words.rgb[2]);
    let mut font = shaper.font_id(&words.font);
    font.size *= zoom;
    let galley = painter.layout_no_wrap(words.text.clone(), font, colour);
    let angle = words.rotation.to_radians() as f32;
    let (sin, cos) = angle.sin_cos();
    let stretch = words.stretch as f32;

    // Where the pen starts and where its baseline sits, in the shape's own
    // box, at this zoom.
    let (pen_x, pen_y) = words.origin(
        0.0,
        0.0,
        f64::from(rect.width() / zoom),
        f64::from(rect.height() / zoom),
    );
    let pen = rect.min + egui::vec2(pen_x as f32 * zoom, pen_y as f32 * zoom);
    // The same point inside the galley: a row states its glyphs' baseline, and
    // the first glyph's own x is where the pen stood when it was drawn.
    let (origin_x, baseline) = galley
        .rows
        .first()
        .and_then(|row| {
            let glyph = row.row.glyphs.first()?;
            Some((row.pos.x + glyph.pos.x, row.pos.y + glyph.pos.y))
        })
        .unwrap_or((0.0, galley.rect.height()));

    // Where a point of the unturned, unstretched galley lands on the page: it
    // is measured from the pen, stretched about the baseline — which is the
    // fixed point paper's text matrix stretches about too — turned, and hung
    // off the pen's place in the shape.
    let place = |p: egui::Pos2| {
        let (x, y) = (p.x - origin_x, (p.y - baseline) * stretch);
        egui::pos2(pen.x + x * cos - y * sin, pen.y + x * sin + y * cos)
    };

    if (stretch - 1.0).abs() < 1e-4 {
        let corner = place(egui::Pos2::ZERO);
        let mut shape = egui::epaint::TextShape::new(corner, galley, colour);
        shape.angle = angle;
        painter.add(shape);
        return;
    }

    let ctx = painter.ctx();
    let mut tessellator = egui::epaint::Tessellator::new(
        ctx.pixels_per_point(),
        ctx.tessellation_options(|options| *options),
        ctx.fonts(|fonts| fonts.font_image_size()),
        Vec::new(),
    );
    let mut mesh = egui::epaint::Mesh::default();
    tessellator.tessellate_text(
        &egui::epaint::TextShape::new(egui::Pos2::ZERO, galley, colour),
        &mut mesh,
    );
    for vertex in &mut mesh.vertices {
        vertex.pos = place(vertex.pos);
    }
    painter.add(egui::Shape::Mesh(std::sync::Arc::new(mesh)));
}

/// Draws a shape that is a rectangle: its fill, then its line.
fn paint_shape_outline(
    painter: &egui::Painter,
    outline: &wp_model::doc::ShapeOutline,
    rect: egui::Rect,
    zoom: f32,
) {
    let colour = |colour: wp_model::Color| match colour {
        wp_model::Color::Rgb(rgb) => egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]),
        _ => egui::Color32::BLACK,
    };
    if let Some(fill) = outline.fill {
        painter.rect_filled(rect, 0.0, colour(fill));
    }
    if let Some(line) = outline.line {
        // Never thinner than a device pixel: a hairline that rounds to zero is
        // a frame that is not there.
        let thickness = (outline.line_width.points() * zoom as f64).max(1.0) as f32;
        painter.rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(thickness, colour(line)),
            egui::StrokeKind::Middle,
        );
    }
}

fn paint_image(
    painter: &egui::Painter,
    pictures: &crate::pictures::Pictures,
    rel: Option<&str>,
    rect: egui::Rect,
) {
    match rel.and_then(|rel| pictures.texture(rel)) {
        Some(texture) => {
            painter.image(
                texture.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => {
            painter.rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0, EDGE),
                egui::StrokeKind::Inside,
            );
            painter.line_segment([rect.left_top(), rect.right_bottom()], (1.0, EDGE));
            painter.line_segment([rect.right_top(), rect.left_bottom()], (1.0, EDGE));
        }
    }
}

/// One drawing a click could pick.
struct Pickable {
    picked: Picked,
    rect: (f64, f64, f64, f64),
    /// Whether the drawing was put under the words, which decides whether a
    /// click that lands on a letter is the letter's or its own.
    behind_text: bool,
}

/// Every drawing on a page that a click may pick, and the rectangle it was
/// drawn in, with whether it was drawn *under* the words.
///
/// **Only the body's own drawings are here.** A shape anchored in the header
/// or the footer is drawn on every page — a watermark across the middle of it,
/// the rectangle Word draws a page frame with around the whole of it — and a
/// click on the words would otherwise land on that shape instead of in the
/// text, because the shape covers the page. Word does not let one be picked
/// from the body either: a watermark "is usually part of the header, even
/// though it appears in the middle of the page", and reaching it means opening
/// the header first — which is exactly what `scope` is. A watermark or a page
/// frame is picked by opening the band it lives in, and is untouchable from
/// the body, because [`Picked`] names a paragraph of one flow and a header
/// starts its count again at nought: the page frame of a 440-paragraph
/// document once reported itself as a drawing of body paragraph 0, and
/// resizing it would have resized whatever picture that paragraph really
/// holds.
///
/// In painting order, so the last one that contains a point is the one on top —
/// which is the one a click means.
fn pickable(view: &View, scope: Scope, page: usize) -> Vec<Pickable> {
    let mut out = Vec::new();
    let Some(page) = view.pages.get(page) else {
        return out;
    };
    for placement in page.placements(scope) {
        match &placement.kind {
            Placed::Drawing {
                anchor,
                paragraph,
                nth,
                ..
            } => {
                let (x, y) = match anchor {
                    Some(drawing) => crate::pictures::anchor_position(
                        drawing,
                        &page.geometry,
                        (placement.x, placement.y),
                    ),
                    None => (placement.x, placement.y),
                };
                out.push(Pickable {
                    picked: Picked {
                        paragraph: *paragraph,
                        nth: *nth,
                    },
                    rect: (x, y, placement.width, placement.height),
                    behind_text: anchor.as_ref().is_some_and(|drawing| drawing.behind_text),
                });
            }
            Placed::Line { line, paragraph } => {
                for fragment in &line.fragments {
                    // A picture bullet is an object with no `nth`: it is drawn
                    // like a drawing but it is not one of the paragraph's, so
                    // there is nothing for a click on it to pick.
                    let Content::Object {
                        height,
                        nth: Some(nth),
                        ..
                    } = &fragment.content
                    else {
                        continue;
                    };
                    out.push(Pickable {
                        picked: Picked {
                            paragraph: *paragraph,
                            nth: *nth,
                        },
                        rect: (
                            placement.x + fragment.x,
                            placement.y + line.baseline - height,
                            fragment.width,
                            *height,
                        ),
                        // A drawing in a line is never under the words: it is
                        // one of them.
                        behind_text: false,
                    });
                }
            }
            _ => {}
        }
    }
    out
}

/// Every drawing on a page a click may pick, and the rectangle it was drawn in.
pub fn drawing_rects(
    view: &View,
    scope: Scope,
    page: usize,
) -> Vec<(Picked, (f64, f64, f64, f64))> {
    pickable(view, scope, page)
        .into_iter()
        .map(|found| (found.picked, found.rect))
        .collect()
}

/// The drawing a point is on, and the rectangle it occupies.
///
/// `reach` widens the rectangle by the handles' grab zone, so the outer half
/// of a handle is still the drawing and not the paper behind it.
///
/// **Words win over a drawing that was put behind them.** A shape set to sit
/// under the text is a background, and a click on a letter drawn over it means
/// the letter — which is Word's own rule: picking a graphic through the text
/// covering it needs the Select Objects tool, and a plain click does not do
/// it. The shape is still reachable everywhere it is not covered, so nothing
/// becomes unselectable by being large.
pub fn drawing_at(
    view: &View,
    scope: Scope,
    spot: Spot,
    reach: f64,
) -> Option<(Picked, (f64, f64, f64, f64))> {
    let found = pickable(view, scope, spot.page);
    // Asked once, and only by a page that has something behind its words to
    // ask about: this runs for every frame the pointer moves over the paper.
    let on_a_letter =
        found.iter().any(|found| found.behind_text) && character_over(view, scope, spot).is_some();
    found
        .into_iter()
        .rev()
        .find(|found| {
            let (x, y, w, h) = found.rect;
            !(found.behind_text && on_a_letter)
                && spot.x >= x - reach
                && spot.x <= x + w + reach
                && spot.y >= y - reach
                && spot.y <= y + h + reach
        })
        .map(|found| (found.picked, found.rect))
}

/// The rectangle a picked drawing was drawn in, wherever it is.
pub fn rect_of(view: &View, scope: Scope, picked: Picked) -> Option<(usize, (f64, f64, f64, f64))> {
    for page in 0..view.pages.len() {
        if let Some((_, rect)) = drawing_rects(view, scope, page)
            .into_iter()
            .find(|(found, _)| *found == picked)
        {
            return Some((page, rect));
        }
    }
    None
}

/// Washes out everything on a page that is not the flow being edited.
///
/// **Word greys the text while the header is open**, and the grey is the whole
/// message: it says the words are still there, still printing, and not what
/// the keys are going into. A page that shows a different header than the one
/// open — another section's, or the first page's — has no live band at all,
/// so the whole sheet goes back.
fn veil(painter: &egui::Painter, page: &Page, scope: Scope, rect: egui::Rect, zoom: f32) {
    // Premultiplied, which is the whole of it: full-white channels at
    // two-thirds alpha do not veil a page, they paint over it — the first
    // attempt erased the text it was meant to grey.
    const WASH: egui::Color32 = egui::Color32::from_rgba_premultiplied(0xA8, 0xA8, 0xA8, 0xA8);
    let (from, to) = match (page.header_scope(), page.footer_scope()) {
        (Some(header), _) if header == scope => (page.geometry.top, page.geometry.height),
        (_, Some(footer)) if footer == scope => (0.0, page.geometry.height - page.geometry.bottom),
        _ => (0.0, page.geometry.height),
    };
    if to <= from {
        return;
    }
    let veiled = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.top() + from as f32 * zoom),
        egui::pos2(rect.right(), rect.top() + to as f32 * zoom),
    );
    painter.rect_filled(veiled.intersect(rect), 0.0, WASH);
}

/// The dashed rule and the tab that say where the band ends, as Word draws
/// them while a header is open.
fn paint_band_rule(
    painter: &egui::Painter,
    page: &Page,
    scope: Scope,
    rect: egui::Rect,
    zoom: f32,
) {
    let Scope::Chrome(_) = scope else {
        return;
    };
    let (y, label, above) = if page.header_scope() == Some(scope) {
        (page.geometry.top, "Header", true)
    } else if page.footer_scope() == Some(scope) {
        (page.geometry.height - page.geometry.bottom, "Footer", false)
    } else {
        return;
    };
    let y = rect.top() + y as f32 * zoom;
    let (left, right) = (
        rect.left() + page.geometry.start as f32 * zoom,
        rect.right() - page.geometry.end as f32 * zoom,
    );
    // Dashes rather than a rule, because the line is not on the paper: it is
    // the application saying where the band stops, and a solid one reads as a
    // border the document does not have.
    let dash = 4.0;
    let mut x = left;
    while x < right {
        let end = (x + dash).min(right);
        painter.line_segment(
            [egui::pos2(x, y), egui::pos2(end, y)],
            egui::Stroke::new(1.0, BAND),
        );
        x = end + dash;
    }
    let anchor = match above {
        true => egui::Align2::LEFT_BOTTOM,
        false => egui::Align2::LEFT_TOP,
    };
    painter.text(
        egui::pos2(left, y),
        anchor,
        label,
        egui::FontId::proportional(9.0),
        BAND,
    );
}

/// Draws the selection handles of a picked drawing.
fn paint_handles(painter: &egui::Painter, at: egui::Pos2, size: egui::Vec2, zoom: f32) {
    let rect = egui::Rect::from_min_size(at, size);
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0, HANDLE),
        egui::StrokeKind::Outside,
    );
    let half = 3.0;
    for (x, y) in [
        (rect.left(), rect.top()),
        (rect.center().x, rect.top()),
        (rect.right(), rect.top()),
        (rect.left(), rect.center().y),
        (rect.right(), rect.center().y),
        (rect.left(), rect.bottom()),
        (rect.center().x, rect.bottom()),
        (rect.right(), rect.bottom()),
    ] {
        let grip =
            egui::Rect::from_center_size(egui::pos2(x, y), egui::vec2(half * 2.0, half * 2.0));
        painter.rect_filled(grip, 1.0, egui::Color32::WHITE);
        painter.rect_stroke(
            grip,
            1.0,
            egui::Stroke::new(1.0, HANDLE),
            egui::StrokeKind::Outside,
        );
    }
    let _ = zoom;
}

/// Draws the pages, the selection and the caret.
///
/// `to_screen` maps a point on the stack of pages to a point in the window.
#[allow(clippy::too_many_arguments)]
pub fn paint(
    painter: &egui::Painter,
    view: &View,
    scope: Scope,
    selection: Selection,
    highlights: &[(Scope, Selection)],
    caret: Option<Caret>,
    focused: bool,
    zoom: f32,
    origin: egui::Pos2,
    shaper: &mut Egui,
    pictures: &crate::pictures::Pictures,
    picked: Option<Picked>,
) {
    for (index, page) in view.pages.iter().enumerate() {
        let (page_x, page_y) = view.page_origin(index);
        let top_left = origin + egui::vec2(page_x as f32 * zoom, page_y as f32 * zoom);
        let size = egui::vec2(
            page.geometry.width as f32 * zoom,
            page.geometry.height as f32 * zoom,
        );
        let rect = egui::Rect::from_min_size(top_left, size);
        if !painter.clip_rect().intersects(rect) {
            continue;
        }
        painter.rect_filled(rect, 0.0, PAPER);
        painter.rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(1.0, EDGE),
            egui::StrokeKind::Outside,
        );

        for (flow, placement) in page.painted() {
            paint_placement(
                painter,
                placement,
                top_left,
                zoom,
                shaper,
                // A selection belongs to one flow. Paragraph three of the
                // body and paragraph three of the header wear the same
                // number, and a highlight that went by number alone struck
                // the header of every page a body selection touched.
                match flow == scope {
                    true => selection,
                    false => Selection::default(),
                },
                // The matches carry their flow for the same reason. Find
                // looks through the headers as well as the text, so a match
                // is highlighted where it was found and nowhere else.
                flow,
                highlights,
                pictures,
                &page.geometry,
            );
        }
        if scope != Scope::Body {
            veil(painter, page, scope, rect, zoom);
        }
        paint_band_rule(painter, page, scope, rect, zoom);
    }

    if let Some(picked) = picked {
        if let Some((page, (x, y, width, height))) = rect_of(view, scope, picked) {
            let (page_x, page_y) = view.page_origin(page);
            let top_left = origin + egui::vec2(page_x as f32 * zoom, page_y as f32 * zoom);
            paint_handles(
                painter,
                top_left + egui::vec2(x as f32 * zoom, y as f32 * zoom),
                egui::vec2(width as f32 * zoom, height as f32 * zoom),
                zoom,
            );
        }
    }

    // A drawing is selected *or* the caret is, never both: the caret would sit
    // blinking in text the arrow keys are no longer moving through.
    if focused && picked.is_none() {
        if let Some(caret) = caret {
            if let Some((page, rect)) = caret_rect(view, scope, caret) {
                let (page_x, page_y) = view.page_origin(page);
                let top_left = origin + egui::vec2(page_x as f32 * zoom, page_y as f32 * zoom);
                let stroke = egui::Rect::from_min_size(
                    top_left + rect.min.to_vec2() * zoom,
                    egui::vec2(1.5, rect.height() * zoom),
                );
                painter.rect_filled(stroke, 0.0, egui::Color32::BLACK);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_placement(
    painter: &egui::Painter,
    placement: &Placement,
    page: egui::Pos2,
    zoom: f32,
    shaper: &mut Egui,
    selection: Selection,
    flow: Scope,
    highlights: &[(Scope, Selection)],
    pictures: &crate::pictures::Pictures,
    geometry: &wp_model::PageBox,
) {
    let at = |x: f64, y: f64| page + egui::vec2(x as f32 * zoom, y as f32 * zoom);
    match &placement.kind {
        Placed::Fill(rgb) => {
            let rect = egui::Rect::from_min_size(
                at(placement.x, placement.y),
                egui::vec2(
                    placement.width as f32 * zoom,
                    placement.height as f32 * zoom,
                ),
            );
            painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
        }
        Placed::Edge { border, side } => {
            let color = border
                .color
                .and_then(|c| c.resolve(&wp_model::Theme::default()))
                .map(|rgb| egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
                .unwrap_or(egui::Color32::BLACK);
            let thickness = border.size.map(|s| s.points()).unwrap_or(0.5);
            let width = thickness as f32 * zoom;
            let (x, y, w, h) = (placement.x, placement.y, placement.width, placement.height);
            // A row is laid out in bands so that a page can break inside it, and
            // its side edges arrive one band at a time. Abutting segments leave
            // a hairline of paper between them once they are anti-aliased, which
            // turns a ruled column into a dotted one — so each overlaps its
            // neighbour by half its own thickness.
            let over = thickness / 2.0;
            let line = match side {
                wp_layout::block::Side::Top => (at(x, y), at(x + w, y)),
                wp_layout::block::Side::Bottom => (at(x, y + h), at(x + w, y + h)),
                wp_layout::block::Side::Start => (at(x, y - over), at(x, y + h + over)),
                wp_layout::block::Side::End => (at(x + w, y - over), at(x + w, y + h + over)),
            };
            painter.line_segment([line.0, line.1], egui::Stroke::new(width.max(0.5), color));
        }
        Placed::Line { line, paragraph } => {
            paint_line(
                painter, placement, line, *paragraph, page, zoom, shaper, selection, flow,
                highlights, pictures,
            );
        }
        Placed::Drawing {
            rel, anchor, words, ..
        } => {
            // The placement's y is where the paragraph's first line landed,
            // which is the one thing pagination knows and the anchor needs.
            let (x, y) = match anchor {
                Some(drawing) => {
                    crate::pictures::anchor_position(drawing, geometry, (placement.x, placement.y))
                }
                None => (placement.x, placement.y),
            };
            let rect = egui::Rect::from_min_size(
                at(x, y),
                egui::vec2(
                    placement.width as f32 * zoom,
                    placement.height as f32 * zoom,
                ),
            );
            paint_drawing(
                painter,
                pictures,
                shaper,
                Shown {
                    rel: rel.as_deref(),
                    chart: anchor.as_ref().and_then(|d| d.chart.as_deref()),
                    words: words.as_deref(),
                    outline: anchor.as_ref().and_then(|d| d.outline.as_ref()),
                },
                rect,
                zoom,
            );
        }
        // Resolved into `Edge` or dropped at pagination; never on a page.
        Placed::BreakEdge { .. } => {}
        Placed::FootnoteSeparator => {
            // The same two inches of hairline `wp_print::ops` puts on paper.
            const SEPARATOR_WIDTH: f64 = 144.0;
            let from = at(placement.x, placement.y);
            let to = at(placement.x + SEPARATOR_WIDTH, placement.y);
            painter.line_segment(
                [from, to],
                egui::Stroke::new(zoom.max(0.5), egui::Color32::BLACK),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_line(
    painter: &egui::Painter,
    placement: &Placement,
    line: &wp_layout::inline::Line,
    paragraph: usize,
    page: egui::Pos2,
    zoom: f32,
    shaper: &mut Egui,
    selection: Selection,
    flow: Scope,
    highlights: &[(Scope, Selection)],
    pictures: &crate::pictures::Pictures,
) {
    let baseline = placement.y + line.baseline;
    // How much of the bytes `from..to` a selection covers, in bytes of this
    // paragraph — so a find match can be painted over just the letters it
    // matched rather than over the whole fragment they sit in.
    let covered = |of: &Selection, from: usize, to: usize| -> Option<(usize, usize)> {
        if of.is_empty() {
            return None;
        }
        let (start, end) = of.ordered();
        if paragraph < start.paragraph || paragraph > end.paragraph {
            return None;
        }
        let low = if paragraph == start.paragraph {
            start.offset
        } else {
            0
        };
        let high = if paragraph == end.paragraph {
            end.offset
        } else {
            usize::MAX
        };
        let overlap = from.max(low)..to.min(high);
        (overlap.start < overlap.end).then_some((overlap.start, overlap.end))
    };

    // The selection is one band across the line, not one rectangle per
    // fragment. A line is one fragment per *word* and a fragment's width stops
    // at its last letter — the space that follows hangs past it — so painting
    // fragment by fragment left a gap of paper at every space and the selection
    // came out striped, a separate block under each word.
    if let Some(band) = selection_band(selection, paragraph, placement, line, page, zoom, shaper) {
        painter.rect_filled(band, 0.0, SELECTION);
    }

    // Shading and highlight for the same reason, and joined the same way: two
    // words of one inverse-video run must not show a hairline of paper where
    // their rectangles meet. `wp_print::ops` draws them from the same list.
    for (from, to, rgb) in wp_print::ops::painted_runs(line) {
        let rect = egui::Rect::from_min_size(
            page + egui::vec2(
                (placement.x + from) as f32 * zoom,
                placement.y as f32 * zoom,
            ),
            egui::vec2((to - from) as f32 * zoom, placement.height as f32 * zoom),
        );
        painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
    }

    for fragment in &line.fragments {
        let x = placement.x + fragment.x;
        if let Some(source) = fragment.source {
            let here = highlights
                .iter()
                .filter(|(at, _)| *at == flow)
                .map(|(_, highlight)| highlight);
            for highlight in here {
                if let Some((from, to)) = covered(highlight, source.start, source.end) {
                    if let Some(rect) =
                        span_rect(placement, fragment, source.start, from, to, page, zoom)
                    {
                        painter.rect_filled(rect, 0.0, MATCH);
                    }
                }
            }
        }
        if let Content::Object {
            height, rel, chart, ..
        } = &fragment.content
        {
            // An inline drawing sits on the baseline like a very large letter.
            let top = baseline - height;
            let rect = egui::Rect::from_min_size(
                page + egui::vec2(x as f32 * zoom, top as f32 * zoom),
                egui::vec2(fragment.width as f32 * zoom, *height as f32 * zoom),
            );
            paint_drawing(
                painter,
                pictures,
                shaper,
                Shown {
                    rel: rel.as_deref(),
                    chart: chart.as_deref(),
                    words: None,
                    outline: None,
                },
                rect,
                zoom,
            );
            continue;
        }
        // A list label draws exactly like text — it just is not text the
        // document holds, so it comes with its own.
        let text = match &fragment.content {
            Content::Text { text, .. }
            | Content::Label { text, .. }
            // A leadered tab draws the dots of a table of contents, and is
            // otherwise an empty stretch that draws nothing at all.
            | Content::Tab { fill: text, .. } => text,
            _ => continue,
        };
        if text.is_empty() {
            continue;
        }
        let style = &fragment.style;
        // A run's own `<w:bdr>` box, stroked down the middle of the room
        // `border_pad` reserved for it — the same box `wp_print::ops` draws
        // on paper, drawn here so the screen shows it before printing does.
        if let Some(border) = style.border {
            let thickness = border.size.map(|s| s.points()).unwrap_or(0.5);
            let rgb = border
                .color
                .and_then(|c| c.resolve(&wp_model::Theme::default()))
                .unwrap_or([0, 0, 0]);
            let half = thickness / 2.0;
            let rect = egui::Rect::from_min_size(
                page + egui::vec2((x + half) as f32 * zoom, (placement.y + half) as f32 * zoom),
                egui::vec2(
                    (fragment.width - thickness).max(0.0) as f32 * zoom,
                    (placement.height - thickness).max(0.0) as f32 * zoom,
                ),
            );
            painter.rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(
                    (thickness * zoom as f64).max(0.5) as f32,
                    egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]),
                ),
                egui::StrokeKind::Inside,
            );
        }
        let color = style
            .color
            .map(|rgb| egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
            // `auto` is the page's own foreground, and the page is paper.
            .unwrap_or(egui::Color32::BLACK);
        // The face the glyphs go in, which for small capitals is not the one
        // the style names. Both the size and the ascent below come from it, or
        // the letters are drawn at one size and anchored by another's.
        let drawn = style.drawn_font();
        let mut font = shaper.font_id(&drawn);
        font.size *= zoom;
        // Anchored by the glyph box's *top*, at baseline minus the face's own
        // ascent. Anchoring the bottom at the baseline — the obvious thing —
        // draws everything a descent too high, because a galley's bottom is
        // baseline plus descent. That was the text poking through table rules.
        let ascent = shaper.metrics(&drawn).ascent;
        let top = baseline - style.raise - ascent;
        let pos = page + egui::vec2(x as f32 * zoom, top as f32 * zoom);
        painter.text(pos, egui::Align2::LEFT_TOP, text, font.clone(), color);

        // The baseline point, which the underline and the strike hang off.
        let base = page + egui::vec2(x as f32 * zoom, (baseline - style.raise) as f32 * zoom);
        let width = fragment.width as f32 * zoom;
        if style.underline.draws() {
            let under = base + egui::vec2(0.0, 2.0 * zoom);
            painter.line_segment(
                [under, under + egui::vec2(width, 0.0)],
                egui::Stroke::new(
                    1.0 * zoom,
                    style
                        .underline_color
                        .map(|rgb| egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
                        .unwrap_or(color),
                ),
            );
        }
        if style.strike || style.double_strike {
            let middle = base - egui::vec2(0.0, font.size * 0.3);
            painter.line_segment(
                [middle, middle + egui::vec2(width, 0.0)],
                egui::Stroke::new(1.0 * zoom, color),
            );
        }
    }
}

/// The one band a selection covers on a line: from the first letter it takes to
/// the last, the spaces between words included.
///
/// Word draws the paragraph mark. A selection that carries on past the end of a
/// line shows about a space of blue after the last letter — that is the ¶, and
/// it is how the reader can see that Delete will pull the next paragraph up onto
/// this one. A wrapped line is given the same block, because the selection
/// really does continue there and a line ending abruptly at its last letter
/// reads as though it did not.
#[allow(clippy::too_many_arguments)]
fn selection_band(
    selection: Selection,
    paragraph: usize,
    placement: &Placement,
    line: &wp_layout::inline::Line,
    page: egui::Pos2,
    zoom: f32,
    shaper: &mut Egui,
) -> Option<egui::Rect> {
    if selection.is_empty() {
        return None;
    }
    let (start, end) = selection.ordered();
    if paragraph < start.paragraph || paragraph > end.paragraph {
        return None;
    }
    let low = match paragraph == start.paragraph {
        true => start.offset,
        false => 0,
    };
    let high = match paragraph == end.paragraph {
        true => end.offset,
        false => usize::MAX,
    };
    let (line_start, line_end) = line_range(line);
    let from = line_start.max(low);
    let to = line_end.min(high);
    // Whether the selection reaches past the end of this line — into the next
    // line of a paragraph that wrapped, or into the paragraph after it.
    let beyond = high > line_end;
    if from > to || (from == to && !beyond) {
        return None;
    }

    let left = x_of(line, from).unwrap_or(0.0);
    let mut right = x_of(line, to).unwrap_or(left);
    if beyond {
        // The mark's width, which is a space in whatever face the line ends in.
        // An empty paragraph has no fragment to ask, and its own height is the
        // next best guess: a space is about a quarter of an em and a line is
        // about six.
        right += match line.fragments.last() {
            Some(fragment) => shaper.width(" ", &fragment.style.font),
            None => placement.height / 4.8,
        };
    }
    if right <= left {
        return None;
    }
    Some(egui::Rect::from_min_max(
        page + egui::vec2(
            (placement.x + left) as f32 * zoom,
            placement.y as f32 * zoom,
        ),
        page + egui::vec2(
            (placement.x + right) as f32 * zoom,
            (placement.y + placement.height) as f32 * zoom,
        ),
    ))
}

/// The rectangle the bytes `from..to` occupy within one fragment, walked from
/// the fragment's own advances so a highlight covers exactly the letters it
/// matched.
fn span_rect(
    placement: &Placement,
    fragment: &wp_layout::inline::Fragment,
    source_start: usize,
    from: usize,
    to: usize,
    page: egui::Pos2,
    zoom: f32,
) -> Option<egui::Rect> {
    let Content::Text { text, advances, .. } = &fragment.content else {
        // A tab or an object is one thing: the whole fragment or nothing.
        let left = placement.x + fragment.x;
        return Some(egui::Rect::from_min_size(
            page + egui::vec2(left as f32 * zoom, placement.y as f32 * zoom),
            egui::vec2(fragment.width as f32 * zoom, placement.height as f32 * zoom),
        ));
    };
    let mut left = 0.0f64;
    let mut width = 0.0f64;
    for (index, (byte, c)) in text.char_indices().enumerate() {
        let advance = advances.get(index).copied().unwrap_or(0.0);
        let at = source_start + byte;
        if at + c.len_utf8() <= from {
            left += advance;
        } else if at < to {
            width += advance;
        }
    }
    if width <= 0.0 {
        return None;
    }
    let x = placement.x + fragment.x + left;
    Some(egui::Rect::from_min_size(
        page + egui::vec2(x as f32 * zoom, placement.y as f32 * zoom),
        egui::vec2(width as f32 * zoom, placement.height as f32 * zoom),
    ))
}

/// The colour the desk is painted.
pub fn desk() -> egui::Color32 {
    DESK
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::doc::{Block, Paragraph};

    #[test]
    fn a_hundred_per_cent_is_words_logical_inch() {
        // Word's own answer: `Application.PointsToPixels(72)` is 96 — the
        // device-independent pixels Windows scales per monitor. 100% must
        // put a document inch on exactly that many of egui's points.
        assert_eq!(72.0 * SCALE, 96.0);
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        ui_kit::fonts::register(&ctx, &[]);
        // egui has no fonts until a frame has been run, and a shaper that asks
        // before then panics rather than measuring.
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        // epaint panics if a frame's texture deltas are dropped unapplied —
        // there is no GPU here to apply them to.
        out.textures_delta.clear();
        ctx
    }

    fn document(texts: &[&str]) -> Document {
        let mut document = Document {
            body: texts
                .iter()
                .map(|text| Block::Paragraph(Paragraph::of(text)))
                .collect(),
            ..Document::new()
        };
        let mut normal = wp_model::Style::new("Normal", wp_model::StyleKind::Paragraph);
        normal.default = true;
        normal.run.size = Some(wp_model::HalfPoint(24));
        document.styles.insert(normal);
        document
    }

    fn laid(texts: &[&str]) -> (View, Egui) {
        let ctx = context();
        let mut shaper = Egui::new(&ctx);
        let mut view = View::default();
        view.refresh(
            &document(texts),
            &wp_layout::FieldValues::new(),
            1,
            &mut shaper,
        );
        (view, shaper)
    }

    /// A document with a header of its own, laid out.
    fn laid_with_header(body: &[&str], header: &str) -> (View, Egui, wp_model::HeaderId) {
        use wp_model::doc::HeaderFooter;
        use wp_model::section::{HeaderId, HeaderKind, HeaderRef};
        let mut document = document(body);
        let id = HeaderId(1);
        document.headers.push(HeaderFooter {
            id,
            part: None,
            rel: None,
            footer: false,
            content: vec![Block::Paragraph(Paragraph::of(header))],
        });
        document.section.headers.push(HeaderRef {
            kind: HeaderKind::Default,
            body: id,
            rel: None,
        });
        let ctx = context();
        let mut shaper = Egui::new(&ctx);
        let mut view = View::default();
        view.refresh(&document, &wp_layout::FieldValues::new(), 1, &mut shaper);
        (view, shaper, id)
    }

    #[test]
    fn the_caret_in_a_header_is_shown_on_the_page_it_was_opened_from() {
        // **A header stands on every page that shows it**, so the caret in one
        // has as many places on the screen as there are pages. Taking the
        // first of them scrolled the window back to page one when the running
        // head was opened on page two — showing the reader the very words the
        // pointer was already on, several inches away. Found by driving the
        // application over a two-section document.
        let body: Vec<String> = (0..80).map(|n| format!("paragraph {n}")).collect();
        let refs: Vec<&str> = body.iter().map(String::as_str).collect();
        let (view, _, id) = laid_with_header(&refs, "RUNNING HEAD");
        assert!(view.pages().len() > 1, "more than one page to choose from");
        let scope = wp_model::Scope::Chrome(id);
        let caret = Caret {
            paragraph: 0,
            offset: 0,
        };

        let (first, _) = caret_rect(&view, scope, caret).expect("the band has a caret");
        assert_eq!(first, 0, "with nothing said, the first page that shows it");
        let (asked, _) =
            caret_rect_on(&view, scope, caret, Some(1)).expect("and on the page asked for");
        assert_eq!(asked, 1);
        // A page that does not show the band falls back rather than answering
        // with nothing.
        assert!(caret_rect_on(&view, scope, caret, Some(99)).is_some());
    }

    #[test]
    fn a_click_in_the_open_header_puts_the_caret_in_the_header_and_not_in_the_text() {
        let (view, _, id) = laid_with_header(&["the body"], "RESUME / CV");
        let page = &view.pages()[0];
        assert_eq!(page.header_body, Some(id), "the page says whose band it is");
        let band = page
            .header
            .iter()
            .find(|placement| matches!(placement.kind, Placed::Line { .. }))
            .expect("the header was laid out");
        let spot = Spot {
            page: 0,
            x: band.x + 1.0,
            y: band.y + band.height / 2.0,
        };

        let scope = Scope::Chrome(id);
        let caret = caret_at(&view, scope, spot).expect("a caret in the header");
        assert_eq!(caret.paragraph, 0, "the header's own first paragraph");

        // The same point, asked of the body, is not the body's: a header is a
        // flow of its own and none of its lines are the text's.
        assert!(
            page.placements(Scope::Body)
                .iter()
                .all(|placement| !std::ptr::eq(placement, band)),
            "the band is not among the body's placements"
        );
    }

    #[test]
    fn a_selection_in_one_flow_does_not_paint_the_same_numbered_line_in_another() {
        // Paragraph 0 of the body and paragraph 0 of the header wear the same
        // number, and the painter is told the flow so it cannot confuse them.
        let (view, _, id) = laid_with_header(&["the body"], "RESUME / CV");
        let page = &view.pages()[0];
        let flows: Vec<Scope> = page
            .painted()
            .filter(|(_, placement)| matches!(placement.kind, Placed::Line { .. }))
            .map(|(flow, _)| flow)
            .collect();
        assert!(flows.contains(&Scope::Body));
        assert!(flows.contains(&Scope::Chrome(id)));
    }

    #[test]
    fn a_document_lays_out_to_at_least_one_page() {
        let (view, _) = laid(&["hello world"]);
        assert!(!view.pages().is_empty());
        let (width, height) = view.extent();
        assert!(width > 0.0 && height > 0.0);
    }

    #[test]
    fn a_step_off_the_last_line_of_a_half_empty_page_lands_on_the_next_page() {
        // The demonstration document's first page holds five short paragraphs
        // and half a page of nothing, and the caret could not be arrowed off
        // it: the point one line below the last line was nearer to that line
        // than to the top of page two, so Down chose where it already was.
        let ctx = context();
        let mut shaper = Egui::new(&ctx);
        let mut document = document(&["first", "second"]);
        let Block::Paragraph(second) = &mut document.body[1] else {
            unreachable!("the document is two paragraphs")
        };
        second.props.page_break_before = Some(true);
        let mut view = View::default();
        view.refresh(&document, &wp_layout::FieldValues::new(), 1, &mut shaper);
        assert_eq!(view.pages().len(), 2);

        let caret = Caret {
            paragraph: 0,
            offset: 0,
        };
        let (_, rect) =
            caret_rect(&view, Scope::Body, caret).expect("the caret is drawn somewhere");
        let step = rect.height() as f64;
        let down = step_from(&view, Scope::Body, caret, step).expect("a line below");
        assert_eq!(down.paragraph, 1, "the page below, not the line it left");

        let up = step_from(&view, Scope::Body, down, -step).expect("a line above");
        assert_eq!(up.paragraph, 0, "and back again");
    }

    #[test]
    fn the_layout_is_not_redone_when_nothing_changed() {
        // A hundred pages laid out per frame is what makes an editor feel slow.
        let ctx = context();
        let mut shaper = Egui::new(&ctx);
        let mut view = View::default();
        let document = document(&["one", "two"]);
        let fields = wp_layout::FieldValues::new();
        view.refresh(&document, &fields, 7, &mut shaper);
        assert!(!view.is_stale(7));
        let pages = view.pages().len();
        view.refresh(&document, &fields, 7, &mut shaper);
        assert_eq!(view.pages().len(), pages);
        assert!(view.is_stale(8), "a new revision needs laying out again");
    }

    #[test]
    fn a_click_on_a_line_resolves_to_a_place_in_that_paragraph() {
        let (view, _) = laid(&["first paragraph", "second paragraph"]);
        let page = &view.pages()[0];
        let first = page
            .content
            .iter()
            .find(|p| matches!(&p.kind, Placed::Line { paragraph, .. } if *paragraph == 0))
            .expect("the first paragraph is on the page");
        let caret = caret_at(
            &view,
            Scope::Body,
            Spot {
                page: 0,
                x: first.x + 1.0,
                y: first.y + first.height / 2.0,
            },
        )
        .expect("a caret");
        assert_eq!(caret.paragraph, 0);
        assert_eq!(caret.offset, 0, "a click at the left edge is the start");
    }

    #[test]
    fn a_click_past_the_end_of_a_line_lands_at_its_end() {
        let (view, _) = laid(&["short"]);
        let page = &view.pages()[0];
        let line = page
            .content
            .iter()
            .find(|p| matches!(p.kind, Placed::Line { .. }))
            .expect("a line");
        let caret = caret_at(
            &view,
            Scope::Body,
            Spot {
                page: 0,
                x: line.x + line.width + 500.0,
                y: line.y + 1.0,
            },
        )
        .expect("a caret");
        assert_eq!(caret.offset, "short".len());
    }

    #[test]
    fn the_pointer_is_on_the_last_letter_of_a_line_rather_than_past_it() {
        // The difference that makes a hyperlink followable at its far end: a
        // caret snaps to the nearer side of a letter, and on the right-hand
        // half of the last one that is the offset after the link, where there
        // is no link at all.
        let (view, _) = laid(&["short"]);
        let page = &view.pages()[0];
        let line = page
            .content
            .iter()
            .find(|p| matches!(p.kind, Placed::Line { .. }))
            .expect("a line");
        let spot = Spot {
            page: 0,
            // Just inside the right edge of the last letter.
            x: line.x + line.width - 0.5,
            y: line.y + line.height / 2.0,
        };
        assert_eq!(
            caret_at(&view, Scope::Body, spot).expect("a caret").offset,
            "short".len(),
            "the caret goes after the letter"
        );
        assert_eq!(
            character_over(&view, Scope::Body, spot)
                .expect("a letter")
                .offset,
            "shor".len(),
            "but the pointer is on it"
        );
        // And off the line there is no letter at all.
        assert!(character_over(
            &view,
            Scope::Body,
            Spot {
                x: line.x + line.width + 40.0,
                ..spot
            }
        )
        .is_none());
    }

    #[test]
    fn a_click_below_the_last_line_lands_in_the_last_paragraph() {
        // Nearest rather than containing: a click on the empty half of a page
        // has to go somewhere, and Word puts it at the end.
        let (view, _) = laid(&["one", "two", "three"]);
        let caret = caret_at(
            &view,
            Scope::Body,
            Spot {
                page: 0,
                x: 100.0,
                y: 10_000.0,
            },
        )
        .expect("a caret");
        assert_eq!(caret.paragraph, 2);
    }

    #[test]
    fn the_caret_has_a_place_on_the_page_for_every_offset() {
        let (view, _) = laid(&["hello world"]);
        for offset in 0..="hello world".len() {
            let found = caret_rect(
                &view,
                Scope::Body,
                Caret {
                    paragraph: 0,
                    offset,
                },
            );
            assert!(found.is_some(), "no caret rectangle at offset {offset}");
        }
    }

    #[test]
    fn an_empty_paragraph_still_has_somewhere_to_put_the_caret() {
        let (view, _) = laid(&["", "after"]);
        let found = caret_rect(
            &view,
            Scope::Body,
            Caret {
                paragraph: 0,
                offset: 0,
            },
        );
        assert!(found.is_some());
    }

    /// A narrow document whose one paragraph wraps into several lines, and the
    /// first two of those line placements.
    fn wrapped() -> (View, Vec<(usize, usize, f64)>) {
        let ctx = context();
        let mut shaper = Egui::new(&ctx);
        let mut view = View::default();
        let mut document = document(&["aa bb cc dd ee ff gg hh"]);
        let margins =
            document.section.margins.start.points() + document.section.margins.end.points();
        document.section.page.width = wp_model::Twips::from_points(60.0 + margins);
        view.refresh(&document, &wp_layout::FieldValues::new(), 1, &mut shaper);
        let mut lines = Vec::new();
        for placement in &view.pages()[0].content {
            if let Placed::Line { line, paragraph } = &placement.kind {
                if *paragraph == 0 {
                    let (start, end) = line_range(line);
                    lines.push((start, end, placement.y));
                }
            }
        }
        assert!(lines.len() >= 2, "the paragraph did not wrap");
        (view, lines)
    }

    #[test]
    fn a_caret_on_the_wrap_boundary_draws_on_the_line_below() {
        // Down and Home land exactly on the byte where one line ends and the
        // next begins. Drawing that caret at the end of the line above looks
        // like the caret refused to move.
        let (view, lines) = wrapped();
        let boundary = lines[1].0;
        assert_eq!(lines[0].1, boundary, "wrapped lines share the byte");
        let (_, rect) = caret_rect(
            &view,
            Scope::Body,
            Caret {
                paragraph: 0,
                offset: boundary,
            },
        )
        .expect("a caret");
        assert!(
            (rect.min.y as f64 - lines[1].2).abs() < 0.1,
            "drawn on the second line, not the first"
        );
        assert_eq!(
            line_span(
                &view,
                Scope::Body,
                Caret {
                    paragraph: 0,
                    offset: boundary
                }
            ),
            Some((lines[1].0, lines[1].1)),
            "and Home/End mean the second line too"
        );
    }

    #[test]
    fn a_click_past_a_wrapped_lines_end_stays_on_that_line() {
        let (view, lines) = wrapped();
        let caret = caret_at(
            &view,
            Scope::Body,
            Spot {
                page: 0,
                x: 10_000.0,
                y: lines[0].2 + 1.0,
            },
        )
        .expect("a caret");
        assert!(
            caret.offset < lines[0].1,
            "before the space the wrap ate, not on the boundary"
        );
        let (_, rect) = caret_rect(&view, Scope::Body, caret).expect("a rect");
        assert!(
            (rect.min.y as f64 - lines[0].2).abs() < 0.1,
            "and it draws on the clicked line"
        );
    }

    #[test]
    fn a_click_level_with_two_cells_lands_in_the_cell_under_the_pointer() {
        // Two cells side by side hold lines at the same y. Choosing by
        // vertical distance alone sent every click on the row to the left
        // cell, whichever cell the pointer was really in.
        use wp_model::table::{Cell, CellProps, Row, Table};
        use wp_model::Twips;
        let table = Table {
            grid: vec![Twips(1440), Twips(1440)],
            rows: vec![Row {
                cells: vec![
                    Cell {
                        props: CellProps::new(),
                        content: vec![Block::Paragraph(Paragraph::of("left"))],
                    },
                    Cell {
                        props: CellProps::new(),
                        content: vec![Block::Paragraph(Paragraph::of("right"))],
                    },
                ],
                ..Row::new()
            }],
            ..Table::new()
        };
        let ctx = context();
        let mut shaper = Egui::new(&ctx);
        let mut view = View::default();
        let mut document = document(&[]);
        document.body = vec![Block::Table(table)];
        view.refresh(&document, &wp_layout::FieldValues::new(), 1, &mut shaper);

        for want in [0usize, 1] {
            let placement = view.pages()[0]
                .content
                .iter()
                .find(|p| matches!(&p.kind, Placed::Line { paragraph, .. } if *paragraph == want))
                .expect("both cells have a line");
            let caret = caret_at(
                &view,
                Scope::Body,
                Spot {
                    page: 0,
                    x: placement.x + placement.width / 2.0,
                    y: placement.y + placement.height / 2.0,
                },
            )
            .expect("a caret");
            assert_eq!(caret.paragraph, want, "the click landed in the other cell");
        }
    }

    #[test]
    fn clicking_where_the_caret_is_drawn_gives_back_the_same_place() {
        // The round trip that decides whether clicking in text works at all.
        let (view, _) = laid(&["hello world"]);
        for offset in [0usize, 3, 6, 11] {
            let caret = Caret {
                paragraph: 0,
                offset,
            };
            let (page, rect) = caret_rect(&view, Scope::Body, caret).expect("a rectangle");
            let back = caret_at(
                &view,
                Scope::Body,
                Spot {
                    page,
                    x: rect.min.x as f64 + 0.1,
                    y: rect.center().y as f64,
                },
            )
            .expect("a caret");
            assert_eq!(
                back, caret,
                "offset {offset} did not survive the round trip"
            );
        }
    }

    /// One paragraph built from several runs, the way a document that has ever
    /// been edited is: a bold word, a hyperlink, a spelling correction.
    fn of_runs(pieces: &[&str]) -> (View, String) {
        use wp_model::doc::{Inline, Run};
        let ctx = context();
        let mut shaper = Egui::new(&ctx);
        let mut view = View::default();
        let mut document = document(&[]);
        document.body = vec![Block::Paragraph(Paragraph {
            content: pieces
                .iter()
                .map(|text| Inline::Run(Run::of(text)))
                .collect(),
            ..Paragraph::new()
        })];
        view.refresh(&document, &wp_layout::FieldValues::new(), 1, &mut shaper);
        (view, pieces.concat())
    }

    /// Every offset of a paragraph, clicked where its caret is drawn.
    fn round_trips(view: &View, text: &str) {
        for offset in 0..=text.len() {
            if !text.is_char_boundary(offset) {
                continue;
            }
            let caret = Caret {
                paragraph: 0,
                offset,
            };
            let (page, rect) = caret_rect(view, Scope::Body, caret).expect("a rectangle");
            let back = caret_at(
                view,
                Scope::Body,
                Spot {
                    page,
                    x: rect.min.x as f64 + 0.1,
                    y: rect.center().y as f64,
                },
            );
            assert_eq!(back, Some(caret), "offset {offset} did not survive");
        }
    }

    #[test]
    fn an_offset_in_the_second_run_is_counted_from_the_paragraph() {
        // `Source` named the byte range within the *piece*, and the caret's
        // offset counts from the start of the paragraph. One run cannot tell
        // the two apart — which is why this went unnoticed — and every run
        // after the first put the caret back at whatever byte of the first run
        // shared its number, so a click near the end of a sentence jumped to
        // the beginning of it.
        let (view, text) = of_runs(&["Hello ", "brave ", "new ", "world"]);
        round_trips(&view, &text);
    }

    #[test]
    fn a_click_past_the_end_of_a_paragraph_lands_after_its_last_space() {
        // The space at the end of a *wrapped* line is one the wrap ate, and the
        // caret there belongs before it. The space at the end of a paragraph is
        // text like any other, and Word puts a click out past the margin after
        // it. Stripping both made the last character of a paragraph ending in a
        // space unreachable by clicking.
        let text = "ends in a space ";
        let (view, _) = laid(&[text]);
        let line = view.pages()[0]
            .content
            .iter()
            .find(|p| matches!(p.kind, Placed::Line { .. }))
            .expect("a line");
        let caret = caret_at(
            &view,
            Scope::Body,
            Spot {
                page: 0,
                x: line.x + line.width + 500.0,
                y: line.y + 1.0,
            },
        )
        .expect("a caret");
        assert_eq!(caret.offset, text.len());
    }

    #[test]
    fn a_tab_is_a_byte_the_caret_can_stand_on_either_side_of() {
        use wp_model::doc::{Inline, Piece, Run};
        let ctx = context();
        let mut shaper = Egui::new(&ctx);
        let mut view = View::default();
        let mut document = document(&[]);
        document.body = vec![Block::Paragraph(Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![
                    Piece::Text("before".into()),
                    Piece::Tab,
                    Piece::Text("after".into()),
                ],
                ..Run::default()
            })],
            ..Paragraph::new()
        })];
        view.refresh(&document, &wp_layout::FieldValues::new(), 1, &mut shaper);
        // The tab is one byte of the text, and the caret has to be able to sit
        // on both sides of it: a tab fragment used to carry no source at all,
        // so the walk skipped straight over it.
        round_trips(&view, "before\tafter");
        let (_, before) = caret_rect(
            &view,
            Scope::Body,
            Caret {
                paragraph: 0,
                offset: 6,
            },
        )
        .expect("before the tab");
        let (_, after) = caret_rect(
            &view,
            Scope::Body,
            Caret {
                paragraph: 0,
                offset: 7,
            },
        )
        .expect("after the tab");
        assert!(
            after.min.x > before.min.x + 1.0,
            "the tab has width between them: {} then {}",
            before.min.x,
            after.min.x
        );
    }

    #[test]
    fn an_inline_picture_is_an_object_a_click_can_pick_and_a_grip_can_pull() {
        use wp_model::doc::{Inline, Piece, Run};
        let ctx = context();
        let mut shaper = Egui::new(&ctx);
        let mut view = View::default();
        let mut document = document(&[]);
        let drawing = wp_model::Drawing {
            source: Vec::new().into(),
            anchored: false,
            extent: (
                wp_model::Emu::from_points(60.0),
                wp_model::Emu::from_points(30.0),
            ),
            rel: Some("rId9".into()),
            chart: None,
            name: None,
            description: None,
            wrap: wp_model::Wrap::None,
            distance: Default::default(),
            position: None,
            behind_text: false,
            text: None,
            tone: None,
            outline: None,
        };
        document.body = vec![Block::Paragraph(Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![Piece::Text("ab".into()), Piece::Drawing(Box::new(drawing))],
                ..Run::default()
            })],
            ..Paragraph::new()
        })];
        view.refresh(&document, &wp_layout::FieldValues::new(), 1, &mut shaper);
        let rects = drawing_rects(&view, Scope::Body, 0);
        let (picked, rect) = *rects.first().expect("the picture is on the page");
        let (x, y, w, h) = rect;
        assert_eq!((w, h), (60.0, 30.0), "drawn at the size it says");

        // A click in the middle finds it, and the model can be reached from
        // what the click answered: paragraph, and which drawing of it.
        let (hit, _) = drawing_at(
            &view,
            Scope::Body,
            Spot {
                page: 0,
                x: x + w / 2.0,
                y: y + h / 2.0,
            },
            crate::drawings::GRIP,
        )
        .expect("a click on a picture picks it");
        assert_eq!(hit, picked);
        assert_eq!(
            rect_of(&view, Scope::Body, picked).map(|(page, _)| page),
            Some(0)
        );

        // And its corner is a grip, which is what a drag pulls.
        assert_eq!(
            crate::drawings::grip_at(rect, x + w, y + h, crate::drawings::GRIP),
            Some(crate::drawings::Grip::Corner {
                right: true,
                bottom: true
            })
        );
    }

    /// A shape as wide and as tall as the page, the shape a page frame and a
    /// watermark are both drawn with.
    fn page_sized(behind_text: bool) -> wp_model::Drawing {
        wp_model::Drawing {
            source: Vec::new().into(),
            anchored: true,
            extent: (
                wp_model::Emu::from_points(540.0),
                wp_model::Emu::from_points(720.0),
            ),
            rel: Some("rId4".into()),
            chart: None,
            name: None,
            description: None,
            wrap: wp_model::Wrap::None,
            distance: Default::default(),
            position: None,
            behind_text,
            text: None,
            tone: None,
            outline: None,
        }
    }

    fn holding(drawing: wp_model::Drawing) -> Block {
        use wp_model::doc::{Inline, Piece, Run};
        Block::Paragraph(Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![Piece::Drawing(Box::new(drawing))],
                ..Run::default()
            })],
            ..Paragraph::new()
        })
    }

    #[test]
    fn a_shape_in_the_header_is_not_what_a_click_on_the_body_means() {
        // The page frame of a real `.doc`: a rectangle anchored in the header,
        // covering the whole text area of every page. It is drawn, and a click
        // goes straight through it to the words underneath — as it does in
        // Word, where reaching a header's own shapes means opening the header.
        let ctx = context();
        let mut shaper = Egui::new(&ctx);
        let mut view = View::default();
        let mut document = document(&["the body of the document"]);
        document.section.headers.push(wp_model::HeaderRef {
            kind: wp_model::HeaderKind::Default,
            body: wp_model::HeaderId(0),
            rel: None,
        });
        document.headers.push(wp_model::HeaderFooter {
            id: wp_model::HeaderId(0),
            part: None,
            rel: None,
            footer: false,
            content: vec![holding(page_sized(true))],
        });
        view.refresh(&document, &wp_layout::FieldValues::new(), 1, &mut shaper);

        let page = view.pages().first().expect("a page");
        assert!(
            page.header
                .iter()
                .any(|placement| matches!(placement.kind, Placed::Drawing { .. })),
            "the frame is still drawn"
        );
        let spot = Spot {
            page: 0,
            x: page.geometry.width / 2.0,
            y: page.geometry.height / 2.0,
        };
        assert_eq!(
            drawing_at(&view, Scope::Body, spot, crate::drawings::GRIP),
            None
        );
        assert!(
            caret_at(&view, Scope::Body, spot).is_some(),
            "and the caret can be put"
        );
    }

    #[test]
    fn a_drawing_put_behind_the_words_gives_up_a_click_that_lands_on_one() {
        let ctx = context();
        let mut shaper = Egui::new(&ctx);
        let mut view = View::default();
        let mut document = document(&[]);
        document.body = vec![
            holding(page_sized(true)),
            Block::Paragraph(Paragraph::of("words over the top of it")),
        ];
        view.refresh(&document, &wp_layout::FieldValues::new(), 1, &mut shaper);

        // The line of text is inside the shape, and a click on a letter of it
        // is a click on the letter.
        let line = view.pages()[0]
            .content
            .iter()
            .find(|placement| matches!(&placement.kind, Placed::Line { paragraph, .. } if *paragraph == 1))
            .expect("the words were laid");
        let on_a_letter = Spot {
            page: 0,
            x: line.x + 1.0,
            y: line.y + line.height / 2.0,
        };
        assert_eq!(drawing_at(&view, Scope::Body, on_a_letter, 0.0), None);

        // Below the words the shape is bare, and there it is still the shape.
        let bare = Spot {
            page: 0,
            x: line.x + 1.0,
            y: line.y + line.height + 200.0,
        };
        assert!(drawing_at(&view, Scope::Body, bare, 0.0).is_some());
    }

    #[test]
    fn a_picture_is_a_character_the_caret_can_stand_on_either_side_of() {
        use wp_model::doc::{Inline, Piece, Run};
        let ctx = context();
        let mut shaper = Egui::new(&ctx);
        let mut view = View::default();
        let mut document = document(&[]);
        let drawing = wp_model::Drawing {
            source: Vec::new().into(),
            anchored: false,
            extent: (
                wp_model::Emu::from_points(60.0),
                wp_model::Emu::from_points(30.0),
            ),
            rel: Some("rId9".into()),
            chart: None,
            name: None,
            description: None,
            wrap: wp_model::Wrap::None,
            distance: Default::default(),
            position: None,
            behind_text: false,
            text: None,
            tone: None,
            outline: None,
        };
        document.body = vec![Block::Paragraph(Paragraph {
            content: vec![Inline::Run(Run {
                content: vec![
                    Piece::Text("before".into()),
                    Piece::Drawing(Box::new(drawing)),
                    Piece::Text("after".into()),
                ],
                ..Run::default()
            })],
            ..Paragraph::new()
        })];
        view.refresh(&document, &wp_layout::FieldValues::new(), 1, &mut shaper);
        // Every offset, including the two the picture stands between, has a
        // place on the page and comes back from a click there.
        round_trips(&view, &format!("before{}after", wp_model::doc::OBJECT));
        let (_, before) = caret_rect(
            &view,
            Scope::Body,
            Caret {
                paragraph: 0,
                offset: 6,
            },
        )
        .expect("in front of it");
        let (_, after) = caret_rect(
            &view,
            Scope::Body,
            Caret {
                paragraph: 0,
                offset: 7,
            },
        )
        .expect("behind it");
        assert!(
            (after.min.x - before.min.x - 60.0).abs() < 1.0,
            "one step crosses the whole picture: {} then {}",
            before.min.x,
            after.min.x
        );
    }

    /// A justified paragraph, narrow enough that its first line is a full one.
    ///
    /// Justification is what pulls the words of a line apart: it is done by
    /// moving whole fragments, so the space between two of them is nobody's
    /// fragment and belongs to no run.
    fn justified() -> (View, Egui) {
        let ctx = context();
        let mut shaper = Egui::new(&ctx);
        let mut view = View::default();
        let mut document = document(&["one two three four five six seven eight nine"]);
        let margins =
            document.section.margins.start.points() + document.section.margins.end.points();
        document.section.page.width = wp_model::Twips::from_points(120.0 + margins);
        for paragraph in document.paragraphs_mut() {
            paragraph.props.justify = Some(wp_model::prop::Justify::Both);
        }
        view.refresh(&document, &wp_layout::FieldValues::new(), 1, &mut shaper);
        (view, shaper)
    }

    fn band_of<'a>(
        view: &'a View,
        shaper: &mut Egui,
        paragraph: usize,
        selection: Selection,
    ) -> Option<(egui::Rect, &'a Placement, &'a wp_layout::inline::Line)> {
        let placement = view.pages()[0]
            .content
            .iter()
            .find(|p| matches!(&p.kind, Placed::Line { paragraph: n, .. } if *n == paragraph))?;
        let Placed::Line { line, .. } = &placement.kind else {
            return None;
        };
        let band = selection_band(
            selection,
            paragraph,
            placement,
            line,
            egui::Pos2::ZERO,
            1.0,
            shaper,
        )?;
        Some((band, placement, line))
    }

    #[test]
    fn a_selection_is_one_band_across_the_line_and_not_a_block_under_each_word() {
        // A line is one fragment per word, and justification spreads the
        // fragments apart — so the space between two words is nobody's fragment.
        // Painting the selection fragment by fragment left a stripe of white
        // paper at every one of those spaces, and a selected sentence came out
        // looking like a row of separately selected words.
        let (view, mut shaper) = justified();
        let selection = Selection {
            anchor: Caret {
                paragraph: 0,
                offset: 0,
            },
            head: Caret {
                paragraph: 1,
                offset: 0,
            },
        };
        let (band, placement, line) =
            band_of(&view, &mut shaper, 0, selection).expect("a selected line");
        let first = &line.fragments[0];
        let second = &line.fragments[1];
        let gap = placement.x + (first.x + first.width + second.x) / 2.0;
        assert!(
            second.x > first.x + first.width,
            "the words really are apart: {} then {}",
            first.x + first.width,
            second.x
        );
        assert!(
            band.min.x as f64 <= gap && gap <= band.max.x as f64,
            "the space between two words is selected too: {gap} not in {band:?}"
        );
    }

    #[test]
    fn a_selection_that_carries_on_past_a_line_shows_the_mark_at_its_end() {
        // Word paints the ¶. Without it a selection reaching into the next
        // paragraph stops dead at the last letter, and nothing on the page says
        // that the paragraph mark is going with it.
        let (view, mut shaper) = laid(&["first", "second"]);
        let within = Selection {
            anchor: Caret {
                paragraph: 0,
                offset: 0,
            },
            head: Caret {
                paragraph: 0,
                offset: 5,
            },
        };
        let onward = Selection {
            head: Caret {
                paragraph: 1,
                offset: 0,
            },
            ..within
        };
        let (stops, ..) = band_of(&view, &mut shaper, 0, within).expect("a band");
        let (carries, ..) = band_of(&view, &mut shaper, 0, onward).expect("a band");
        assert!(
            carries.max.x > stops.max.x + 1.0,
            "the mark is drawn past the last letter: {} then {}",
            stops.max.x,
            carries.max.x
        );
    }

    #[test]
    fn an_empty_paragraph_inside_a_selection_is_visibly_in_it() {
        // It has no letters to paint over, so it painted nothing at all — a gap
        // in the middle of a selection, exactly where the user had dragged.
        let (view, mut shaper) = laid(&["before", "", "after"]);
        let selection = Selection {
            anchor: Caret {
                paragraph: 0,
                offset: 0,
            },
            head: Caret {
                paragraph: 2,
                offset: 5,
            },
        };
        let (band, ..) = band_of(&view, &mut shaper, 1, selection).expect("the empty paragraph");
        assert!(
            band.width() > 1.0,
            "a mark's worth of blue, not nothing: {band:?}"
        );
    }

    #[test]
    fn a_selection_paints_the_letters_it_covers_and_not_the_word_around_them() {
        // A line is one fragment per *word*, and the selection filled whichever
        // fragments it touched — so selecting two letters turned the whole word
        // blue. The find bar had this right all along, in the same function,
        // three lines above.
        let (view, _) = laid(&["selectable words here"]);
        let placement = view.pages()[0]
            .content
            .iter()
            .find(|p| matches!(p.kind, Placed::Line { .. }))
            .expect("a line");
        let Placed::Line { line, .. } = &placement.kind else {
            unreachable!()
        };
        let fragment = line
            .fragments
            .iter()
            .find(|f| matches!(&f.content, Content::Text { text, .. } if text.starts_with("selectable")))
            .expect("the first word");
        let source = fragment.source.expect("it came from the text");
        let part = span_rect(
            placement,
            fragment,
            source.start,
            source.start,
            source.start + 3,
            egui::Pos2::ZERO,
            1.0,
        )
        .expect("three letters of it");
        assert!(
            part.width() < fragment.width as f32 * 0.6,
            "three letters of a ten-letter word, not the word: {} of {}",
            part.width(),
            fragment.width
        );
    }
}
