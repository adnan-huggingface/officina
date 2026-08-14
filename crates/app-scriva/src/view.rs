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
            fallback_font: "Calibri",
            show_revisions: self.show_revisions,
            show_hidden: self.show_marks,
            fields: match self.settled.is_empty() {
                true => fields,
                false => &carried,
            },
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
    fn page_origin(&self, index: usize) -> (f64, f64) {
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
    let mut best: Option<(f64, &Placement, usize)> = None;
    for placement in &page.content {
        let Placed::Line { paragraph, .. } = &placement.kind else {
            continue;
        };
        let middle = placement.y + placement.height / 2.0;
        let distance = (middle - spot.y).abs();
        if best.as_ref().is_none_or(|(best, _, _)| distance < *best) {
            best = Some((distance, placement, *paragraph));
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
    // Past the end of the line: the end of its last piece of text.
    if line.fragments.is_empty() {
        return 0;
    }
    last.max(first.unwrap_or(0))
}

/// Where a caret sits on the page, as a vertical stroke.
pub fn caret_rect(view: &View, caret: Caret) -> Option<(usize, egui::Rect)> {
    for (index, page) in view.pages.iter().enumerate() {
        for placement in &page.content {
            let Placed::Line { line, paragraph } = &placement.kind else {
                continue;
            };
            if *paragraph != caret.paragraph {
                continue;
            }
            let (start, end) = line_range(line);
            if caret.offset < start || caret.offset > end {
                continue;
            }
            let x = x_of(line, caret.offset).unwrap_or(0.0) + placement.x;
            let top = placement.y;
            return Some((
                index,
                egui::Rect::from_min_size(
                    egui::pos2(x as f32, top as f32),
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

/// Draws one picture, or the frame that stands in for a picture that could not
/// be decoded.
///
/// A missing picture draws a frame rather than nothing, because a hole in the
/// page is a fact the reader should be able to see — silence would look like a
/// document that never had the picture.
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
    caret: Option<Caret>,
    focused: bool,
    zoom: f32,
    origin: egui::Pos2,
    shaper: &Egui,
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
    shaper: &Egui,
    selection: Selection,
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
            let width = border.size.map(|s| s.points()).unwrap_or(0.5) as f32 * zoom;
            let (x, y, w, h) = (placement.x, placement.y, placement.width, placement.height);
            let line = match side {
                wp_layout::block::Side::Top => (at(x, y), at(x + w, y)),
                wp_layout::block::Side::Bottom => (at(x, y + h), at(x + w, y + h)),
                wp_layout::block::Side::Start => (at(x, y), at(x, y + h)),
                wp_layout::block::Side::End => (at(x + w, y), at(x + w, y + h)),
            };
            painter.line_segment([line.0, line.1], egui::Stroke::new(width.max(0.5), color));
        }
        Placed::Line { line, paragraph } => {
            paint_line(
                painter, placement, line, *paragraph, page, zoom, shaper, selection, pictures,
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
            paint_image(painter, pictures, rel.as_deref(), rect);
        }
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
    shaper: &Egui,
    selection: Selection,
    pictures: &crate::pictures::Pictures,
) {
    let baseline = placement.y + line.baseline;
    let (start, end) = selection.ordered();
    let selected = |from: usize, to: usize| -> bool {
        if selection.is_empty() {
            return false;
        }
        if paragraph < start.paragraph || paragraph > end.paragraph {
            return false;
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
        from < high && to > low
    };

    for fragment in &line.fragments {
        let x = placement.x + fragment.x;
        if let Some(source) = fragment.source {
            if selected(source.start, source.end) {
                let rect = egui::Rect::from_min_size(
                    page + egui::vec2(x as f32 * zoom, placement.y as f32 * zoom),
                    egui::vec2(fragment.width as f32 * zoom, placement.height as f32 * zoom),
                );
                painter.rect_filled(rect, 0.0, SELECTION);
            }
        }
        if let Content::Object { height, rel, .. } = &fragment.content {
            // An inline drawing sits on the baseline like a very large letter.
            let top = baseline - height;
            let rect = egui::Rect::from_min_size(
                page + egui::vec2(x as f32 * zoom, top as f32 * zoom),
                egui::vec2(fragment.width as f32 * zoom, *height as f32 * zoom),
            );
            paint_image(painter, pictures, rel.as_deref(), rect);
            continue;
        }
        let Content::Text { text, .. } = &fragment.content else {
            continue;
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
        let y = baseline - style.raise;
        let pos = page + egui::vec2(x as f32 * zoom, y as f32 * zoom);
        painter.text(pos, egui::Align2::LEFT_BOTTOM, text, font.clone(), color);

        let width = fragment.width as f32 * zoom;
        if style.underline.draws() {
            let under = pos + egui::vec2(0.0, 2.0 * zoom);
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
            let middle = pos - egui::vec2(0.0, font.size * 0.3);
            painter.line_segment(
                [middle, middle + egui::vec2(width, 0.0)],
                egui::Stroke::new(1.0 * zoom, color),
            );
        }
    }
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
