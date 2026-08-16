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
use wp_layout::block::{Placed, Placement};
use wp_layout::inline::Content;
use wp_layout::shape::Shaper;
use wp_model::Document;

use crate::drawings::{Picked, GRIP};
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

/// The laid-out document, and how it is being looked at.
pub struct View {
    /// 1.0 is a point per point. Word's zoom, as a fraction.
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
            default_tab: document.settings.default_tab_stop,
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
            show_revisions: self.show_revisions,
            show_hidden: self.show_marks,
            fields: match self.settled.is_empty() {
                true => fields,
                false => &carried,
            },
            band: None,
        };
        self.pages = wp_layout::block::layout(document, &ctx, shaper);
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
pub fn caret_at(view: &View, spot: Spot) -> Option<Caret> {
    let page = view.pages.get(spot.page)?;
    let mut best: Option<((f64, f64), &Placement, usize)> = None;
    for placement in &page.content {
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
    Some(Caret {
        paragraph,
        offset: offset_in(line, spot.x - placement.x),
    })
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
    let mut last = 0usize;
    let mut first = None;
    for fragment in &line.fragments {
        let Content::Text { advances, .. } = &fragment.content else {
            continue;
        };
        let Some(source) = fragment.source else {
            continue;
        };
        if first.is_none() {
            first = Some(source.start);
        }
        last = source.end;
        if x < fragment.x {
            return source.start;
        }
        if x <= fragment.x + fragment.width {
            // Inside this fragment: walk the advances to the nearest boundary.
            let mut at = fragment.x;
            let mut offset = source.start;
            let text = match &fragment.content {
                Content::Text { text, .. } => text.as_str(),
                _ => "",
            };
            for (index, (byte, c)) in text.char_indices().enumerate() {
                let width = advances.get(index).copied().unwrap_or(0.0);
                if x < at + width / 2.0 {
                    return source.start + byte;
                }
                at += width;
                offset = source.start + byte + c.len_utf8();
            }
            return offset;
        }
    }
    // Past the end of the line: the end of its last piece of text — minus the
    // space a wrap ate, whose byte belongs to this line but whose caret would
    // draw at the start of the next. Word puts a click out here after the last
    // letter, and so does this.
    if line.fragments.is_empty() {
        return 0;
    }
    let mut end = last.max(first.unwrap_or(0));
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

/// The line placement the caret belongs to, and which page it is on.
///
/// A wrapped line's end is byte-identical to the next line's start, and the
/// caret there belongs to the line *below*: Down and Home land on that offset,
/// and drawing it at the end of the line above looks like the caret refused to
/// move. Only the paragraph's true end — which no later line shares — keeps
/// the caret on the line that ends there.
fn line_holding(view: &View, caret: Caret) -> Option<(usize, &Placement)> {
    let mut at_end: Option<(usize, &Placement)> = None;
    for (index, page) in view.pages.iter().enumerate() {
        for placement in &page.content {
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
pub fn caret_rect(view: &View, caret: Caret) -> Option<(usize, egui::Rect)> {
    if let Some((index, placement)) = line_holding(view, caret) {
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
    for (index, page) in view.pages.iter().enumerate() {
        for placement in &page.content {
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
pub fn line_span(view: &View, caret: Caret) -> Option<(usize, usize)> {
    let (_, placement) = line_holding(view, caret)?;
    let Placed::Line { line, .. } = &placement.kind else {
        return None;
    };
    Some(line_range(line))
}

/// The caret nearest the point `dy` points above or below where `caret` is
/// drawn — measured down the whole stack of pages, so a step can cross from the
/// last line of one page onto the first line of the next.
///
/// Arrow keys and Page Up/Down both come here: the only difference is the size
/// of the step.
pub fn step_from(view: &View, caret: Caret, dy: f64) -> Option<Caret> {
    let (page, rect) = caret_rect(view, caret)?;
    let (page_x, page_y) = view.page_origin(page);
    let x = page_x + rect.min.x as f64;
    let y = page_y + rect.center().y as f64 + dy;
    let mut chosen = view.pages.len().checked_sub(1)?;
    for index in 0..view.pages.len() {
        let (_, top) = view.page_origin(index);
        if y < top + view.pages[index].geometry.height {
            chosen = index;
            break;
        }
    }
    let (chosen_x, chosen_y) = view.page_origin(chosen);
    caret_at(
        view,
        Spot {
            page: chosen,
            x: x - chosen_x,
            y: y - chosen_y,
        },
    )
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
            return Some(fragment.x);
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
fn paint_drawing(
    painter: &egui::Painter,
    pictures: &crate::pictures::Pictures,
    rel: Option<&str>,
    chart: Option<&str>,
    rect: egui::Rect,
    zoom: f32,
) {
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
    paint_image(painter, pictures, rel, rect);
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

/// Every drawing on a page, and the rectangle it was drawn in.
///
/// In painting order, so the last one that contains a point is the one on top —
/// which is the one a click means.
pub fn drawing_rects(view: &View, page: usize) -> Vec<(Picked, (f64, f64, f64, f64))> {
    let mut out = Vec::new();
    let Some(page) = view.pages.get(page) else {
        return out;
    };
    for placement in page.everything() {
        match &placement.kind {
            Placed::Drawing {
                anchor,
                paragraph,
                nth,
                ..
            } => {
                let (x, y) = match anchor {
                    Some(drawing) => {
                        crate::pictures::anchor_position(drawing, &page.geometry, placement.y)
                    }
                    None => (placement.x, placement.y),
                };
                out.push((
                    Picked {
                        paragraph: *paragraph,
                        nth: *nth,
                    },
                    (x, y, placement.width, placement.height),
                ));
            }
            Placed::Line { line, paragraph } => {
                for fragment in &line.fragments {
                    let Content::Object { height, nth, .. } = &fragment.content else {
                        continue;
                    };
                    out.push((
                        Picked {
                            paragraph: *paragraph,
                            nth: *nth,
                        },
                        (
                            placement.x + fragment.x,
                            placement.y + line.baseline - height,
                            fragment.width,
                            *height,
                        ),
                    ));
                }
            }
            _ => {}
        }
    }
    out
}

/// The drawing a point is on, and the rectangle it occupies.
pub fn drawing_at(view: &View, spot: Spot) -> Option<(Picked, (f64, f64, f64, f64))> {
    drawing_rects(view, spot.page)
        .into_iter()
        .rev()
        .find(|(_, (x, y, w, h))| {
            spot.x >= *x - GRIP
                && spot.x <= x + w + GRIP
                && spot.y >= *y - GRIP
                && spot.y <= y + h + GRIP
        })
}

/// The rectangle a picked drawing was drawn in, wherever it is.
pub fn rect_of(view: &View, picked: Picked) -> Option<(usize, (f64, f64, f64, f64))> {
    for page in 0..view.pages.len() {
        if let Some((_, rect)) = drawing_rects(view, page)
            .into_iter()
            .find(|(found, _)| *found == picked)
        {
            return Some((page, rect));
        }
    }
    None
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
    selection: Selection,
    highlights: &[Selection],
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

        for placement in page.everything() {
            paint_placement(
                painter,
                placement,
                top_left,
                zoom,
                shaper,
                selection,
                highlights,
                pictures,
                &page.geometry,
            );
        }
    }

    if let Some(picked) = picked {
        if let Some((page, (x, y, width, height))) = rect_of(view, picked) {
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
            if let Some((page, rect)) = caret_rect(view, caret) {
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
    highlights: &[Selection],
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
                painter, placement, line, *paragraph, page, zoom, shaper, selection, highlights,
                pictures,
            );
        }
        Placed::Drawing { rel, anchor, .. } => {
            // The placement's y is where the paragraph's first line landed,
            // which is the one thing pagination knows and the anchor needs.
            let (x, y) = match anchor {
                Some(drawing) => crate::pictures::anchor_position(drawing, geometry, placement.y),
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
                rel.as_deref(),
                anchor.as_ref().and_then(|d| d.chart.as_deref()),
                rect,
                zoom,
            );
        }
        // Resolved into `Edge` or dropped at pagination; never on a page.
        Placed::BreakEdge { .. } => {}
        Placed::FootnoteSeparator => {}
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
    highlights: &[Selection],
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

    for fragment in &line.fragments {
        let x = placement.x + fragment.x;
        if let Some(source) = fragment.source {
            for highlight in highlights {
                if let Some((from, to)) = covered(highlight, source.start, source.end) {
                    if let Some(rect) =
                        span_rect(placement, fragment, source.start, from, to, page, zoom)
                    {
                        painter.rect_filled(rect, 0.0, MATCH);
                    }
                }
            }
            if covered(&selection, source.start, source.end).is_some() {
                let rect = egui::Rect::from_min_size(
                    page + egui::vec2(x as f32 * zoom, placement.y as f32 * zoom),
                    egui::vec2(fragment.width as f32 * zoom, placement.height as f32 * zoom),
                );
                painter.rect_filled(rect, 0.0, SELECTION);
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
                rel.as_deref(),
                chart.as_deref(),
                rect,
                zoom,
            );
            continue;
        }
        // A list label draws exactly like text — it just is not text the
        // document holds, so it comes with its own.
        let text = match &fragment.content {
            Content::Text { text, .. } | Content::Label { text, .. } => text,
            _ => continue,
        };
        if text.is_empty() {
            continue;
        }
        let style = &fragment.style;
        if let Some(rgb) = style.highlight {
            let rect = egui::Rect::from_min_size(
                page + egui::vec2(x as f32 * zoom, placement.y as f32 * zoom),
                egui::vec2(fragment.width as f32 * zoom, placement.height as f32 * zoom),
            );
            painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
        }
        let color = style
            .color
            .map(|rgb| egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
            // `auto` is the page's own foreground, and the page is paper.
            .unwrap_or(egui::Color32::BLACK);
        let mut font = shaper.font_id(&style.font);
        font.size *= zoom;
        // Anchored by the glyph box's *top*, at baseline minus the face's own
        // ascent. Anchoring the bottom at the baseline — the obvious thing —
        // draws everything a descent too high, because a galley's bottom is
        // baseline plus descent. That was the text poking through table rules.
        let ascent = shaper.metrics(&style.font).ascent;
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

    #[test]
    fn a_document_lays_out_to_at_least_one_page() {
        let (view, _) = laid(&["hello world"]);
        assert!(!view.pages().is_empty());
        let (width, height) = view.extent();
        assert!(width > 0.0 && height > 0.0);
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
    fn a_click_below_the_last_line_lands_in_the_last_paragraph() {
        // Nearest rather than containing: a click on the empty half of a page
        // has to go somewhere, and Word puts it at the end.
        let (view, _) = laid(&["one", "two", "three"]);
        let caret = caret_at(
            &view,
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
        let (_, rect) = caret_rect(&view, caret).expect("a rect");
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
            let (page, rect) = caret_rect(&view, caret).expect("a rectangle");
            let back = caret_at(
                &view,
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
}
