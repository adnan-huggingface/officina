//! Drawing the grid, and the mouse and keyboard that drive it.
//!
//! Frozen panes are why this is not one loop. A sheet with a frozen top row and
//! frozen first column is drawn as *four* independent views of the same sheet:
//! the corner scrolls with neither axis, the top strip scrolls horizontally
//! only, the left strip vertically only, and the body with both. Each is the
//! same [`plan`](super::plan) call with a different scroll offset and clip
//! rectangle, which is the whole reason `plan` takes those as parameters rather
//! than reading them from the view.

use ss_formula::edit::Geometry;
use ss_model::{column_name, Axis, CellRange, CellRef, Sheet, Workbook};
use ui_kit::egui;

use super::editor::{self, Editor, Mode};
use super::{
    plan, rect_of, rect_of_range, Action, Direction, Drag, Format, GridView, Layout, PaintEdge,
    Scroll, BOTTOM, LEFT, RESIZE_GRAB, RIGHT, TOP,
};
use ss_formula::cond::{Formatting, Overlay};
use ss_model::style::{BorderStyle, HAlign, VAlign};

/// Side of the little square at the corner of the selection.
const FILL_HANDLE: f32 = 6.0;

/// Line and fill colours, resolved from the surrounding egui theme so the grid
/// follows a light or dark visual style without a second palette.
struct Palette {
    grid: egui::Color32,
    text: egui::Color32,
    header_bg: egui::Color32,
    header_text: egui::Color32,
    header_active: egui::Color32,
    selection_fill: egui::Color32,
    selection_edge: egui::Color32,
    background: egui::Color32,
}

impl Palette {
    fn of(ui: &egui::Ui) -> Self {
        let visuals = ui.visuals();
        let dark = visuals.dark_mode;
        Palette {
            grid: if dark {
                egui::Color32::from_gray(60)
            } else {
                egui::Color32::from_gray(214)
            },
            text: visuals.text_color(),
            header_bg: visuals.faint_bg_color,
            header_text: visuals.weak_text_color(),
            header_active: visuals.selection.bg_fill.gamma_multiply(0.35),
            selection_fill: visuals.selection.bg_fill.gamma_multiply(0.18),
            selection_edge: visuals.selection.stroke.color,
            background: visuals.extreme_bg_color,
        }
    }
}

/// Where a cell sits on screen, across whichever pane holds it.
fn cell_rect(layout: &Layout, panes: &[Pane], at: CellRef) -> Option<egui::Rect> {
    panes.iter().find_map(|pane| {
        let rect = rect_of(layout, at, pane.rect, pane.scroll);
        pane.rect.contains(rect.center()).then_some(rect)
    })
}

/// The rectangle a fill drag covers: the source, grown along one axis only.
///
/// Excel fills in a single direction. Dragging diagonally still fills whichever
/// way you pulled further, because a fill extrapolates one series, not two.
fn fill_target(from: CellRange, to: CellRef) -> CellRange {
    let down = to.row.max(from.end.row) - to.row.min(from.start.row) - (from.rows() - 1);
    let across = to.col.max(from.end.col) - to.col.min(from.start.col) - (from.cols() - 1);
    if down >= across {
        CellRange::new(
            CellRef::new(from.start.row.min(to.row), from.start.col),
            CellRef::new(from.end.row.max(to.row), from.end.col),
        )
    } else {
        CellRange::new(
            CellRef::new(from.start.row, from.start.col.min(to.col)),
            CellRef::new(from.end.row, from.end.col.max(to.col)),
        )
    }
}

/// One of the up-to-four views a frozen sheet is drawn as.
struct Pane {
    rect: egui::Rect,
    scroll: Scroll,
}

/// Everything about where the grid sits this frame, gathered once.
struct Frame<'a> {
    layout: &'a Layout,
    palette: &'a Palette,
    /// The whole widget, headers included.
    full: egui::Rect,
    /// Just the cells, with the headers taken off the top and left.
    body: egui::Rect,
    panes: &'a [Pane],
    conditional: &'a Formatting,
}

impl GridView {
    /// Draws the grid and handles input for one frame.
    pub fn show(&mut self, ui: &mut egui::Ui, book: &mut Workbook) -> egui::Response {
        let full = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(full, egui::Sense::click_and_drag());
        if book.sheet(self.sheet_index).is_none() {
            self.sheet_index = 0;
        }
        let Some(sheet) = book.sheet(self.sheet_index) else {
            return response;
        };

        // Taken rather than borrowed: the layout lives in `self` and so does
        // everything the input handlers need to change.
        self.ensure_layout(sheet);
        self.ensure_conditional(book, sheet);
        let cached = self.layout.take().expect("just ensured");
        let layout = &cached.2;
        let conditional = self.conditional.take().expect("just ensured");

        let palette = Palette::of(ui);
        ui.painter().rect_filled(full, 0.0, palette.background);

        let body = egui::Rect::from_min_max(
            full.min + egui::vec2(layout.header_width as f32, layout.header_height as f32),
            full.max,
        );
        let frozen = sheet.frozen.unwrap_or(CellRef::new(0, 0));
        let split = egui::vec2(
            layout.cols.offset(frozen.col) as f32,
            layout.rows.offset(frozen.row) as f32,
        );

        self.clamp_scroll(layout, body.size() - split);
        let panes = split_panes(body, split, self.scroll, layout, frozen);

        let frame = Frame {
            layout,
            palette: &palette,
            full,
            body,
            panes: &panes,
            conditional: &conditional.2,
        };
        for pane in &panes {
            if pane.rect.width() <= 0.0 || pane.rect.height() <= 0.0 {
                continue;
            }
            self.paint_pane(ui, book, sheet, &frame, pane);
        }
        self.paint_headers(ui, &frame);

        // The frozen split is the one line that is not a gridline.
        if frozen.row > 0 {
            let y = body.top() + split.y;
            ui.painter().hline(
                full.x_range(),
                y,
                egui::Stroke::new(1.0, palette.selection_edge),
            );
        }
        if frozen.col > 0 {
            let x = body.left() + split.x;
            ui.painter().vline(
                x,
                full.y_range(),
                egui::Stroke::new(1.0, palette.selection_edge),
            );
        }

        // Resolved while the layout is still borrowed, painted after it goes
        // back into `self`.
        let editor_rect = self
            .editor
            .as_ref()
            .and_then(|open| cell_rect(layout, &panes, open.at));

        // A chart floats above the cells, so a click on one is not a click on
        // the cell underneath. Resolved here, where the panes are still known.
        if response.clicked() {
            self.selected_chart = ui.ctx().pointer_interact_pos().and_then(|pos| {
                panes.iter().find_map(|pane| {
                    if !pane.rect.contains(pos) {
                        return None;
                    }
                    sheet.charts.iter().position(|chart| {
                        super::chart::rect_of(layout, &chart.anchor, pane.rect, pane.scroll)
                            .contains(pos)
                    })
                })
            });
        }

        let cursor_rect = cell_rect(layout, &panes, self.selection.cursor());

        self.layout = Some(cached);
        self.conditional = Some(conditional);
        self.paint_editor(ui, editor_rect);
        self.paint_dropdown(ui, book, cursor_rect);
        self.handle_input(ui, book, &response, full, body);
        response
    }

    /// The small square at the bottom-right corner of the selection.
    fn fill_handle(&self, layout: &Layout, pane: &Pane) -> egui::Rect {
        let corner = rect_of_range(
            layout,
            self.selection.active_range(),
            pane.rect,
            pane.scroll,
        )
        .right_bottom();
        egui::Rect::from_center_size(corner, egui::vec2(FILL_HANDLE, FILL_HANDLE))
    }

    /// Draws the open editor over its cell.
    fn paint_editor(&mut self, ui: &mut egui::Ui, rect: Option<egui::Rect>) {
        let zoom = self.zoom;
        let Some(open) = &mut self.editor else {
            return;
        };
        // Scrolled out of view: the formula bar is still showing the same text,
        // so there is nothing to draw and nothing lost.
        let Some(rect) = rect else {
            return;
        };

        let id = egui::Id::new("calx-cell-editor");
        let font = egui::FontId::proportional((13.0 * zoom) as f32);
        let plain = ui.visuals().text_color();
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
            let mut job = editor::highlight(text.as_str(), font.clone(), plain);
            job.wrap.max_width = wrap;
            ui.fonts_mut(|f| f.layout_job(job))
        };

        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.expand(1.0)));
        let output = egui::TextEdit::singleline(&mut open.text)
            .id(id)
            .margin(egui::Margin::symmetric(2, 0))
            .layouter(&mut layouter)
            .desired_width(rect.width().max(60.0))
            .show(&mut child);

        if open.fresh {
            output.response.request_focus();
            // The caret belongs after what was seeded, not before it.
            let mut state = output.state.clone();
            let end = egui::text::CCursor::new(open.text.chars().count());
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(end)));
            state.store(ui.ctx(), id);
            open.fresh = false;
        }
    }

    /// The list-validation dropdown on the selected cell.
    ///
    /// Only on the cursor: Excel draws the arrow for the active cell alone, and
    /// resolving a range-sourced list costs an evaluation, which is not
    /// something to do for every visible cell of a validated column.
    fn paint_dropdown(&mut self, ui: &mut egui::Ui, book: &Workbook, rect: Option<egui::Rect>) {
        let Some(rect) = rect else { return };
        if self.editor.is_some() {
            return;
        }
        let at = self.selection.cursor();
        let shown = book
            .sheet(self.sheet_index)
            .and_then(|s| s.validation_at(at))
            .is_some_and(|dv| dv.show_dropdown);
        if !shown {
            return;
        }
        let Some(choices) = ss_formula::cond::choices(book, self.sheet_index, at) else {
            return;
        };
        if choices.is_empty() {
            return;
        }

        let button = egui::Rect::from_min_size(
            egui::pos2(rect.right(), rect.top()),
            egui::vec2(16.0, rect.height().min(20.0)),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(button));
        let mut picked = None;
        egui::ComboBox::from_id_salt("calx-validation-list")
            .selected_text("▾")
            .width(16.0)
            .show_ui(&mut child, |ui| {
                for choice in &choices {
                    if ui.selectable_label(false, choice).clicked() {
                        picked = Some(choice.clone());
                    }
                }
            });
        if let Some(text) = picked {
            self.actions.push(Action::Commit {
                at,
                text,
                advance: None,
            });
        }
    }

    fn ensure_conditional(&mut self, book: &Workbook, sheet: &Sheet) {
        let key = (self.sheet_index, self.generation);
        let stale = match &self.conditional {
            Some((index, generation, _)) => (*index, *generation) != key,
            None => true,
        };
        if stale {
            let index = book
                .sheets
                .iter()
                .position(|s| std::ptr::eq(s, sheet))
                .unwrap_or(self.sheet_index);
            self.conditional = Some((key.0, key.1, Formatting::prepare(book, index)));
        }
    }

    fn ensure_layout(&mut self, sheet: &Sheet) {
        let key = (self.sheet_index, self.generation);
        let stale = match &self.layout {
            Some((index, generation, _)) => (*index, *generation) != key,
            None => true,
        };
        if stale {
            self.layout = Some((key.0, key.1, Layout::for_sheet(sheet, self.zoom)));
        }
    }

    /// Keeps the scroll offset inside the sheet, allowing one screen of
    /// overscroll at the end so the last row is reachable.
    fn clamp_scroll(&mut self, layout: &Layout, body: egui::Vec2) {
        let max_x = (layout.cols.total() - f64::from(body.x)).max(0.0);
        let max_y = (layout.rows.total() - f64::from(body.y)).max(0.0);
        self.scroll.x = self.scroll.x.clamp(0.0, max_x);
        self.scroll.y = self.scroll.y.clamp(0.0, max_y);
    }

    fn paint_pane(
        &self,
        ui: &egui::Ui,
        book: &Workbook,
        sheet: &Sheet,
        frame: &Frame,
        pane: &Pane,
    ) {
        let (layout, palette) = (frame.layout, frame.palette);
        let painter = ui.painter().with_clip_rect(pane.rect);
        let drawn = plan(
            book,
            sheet,
            layout,
            pane.rect,
            pane.scroll,
            frame.conditional,
        );
        let stroke = egui::Stroke::new(1.0, palette.grid);

        // Gridlines first, so cell text sits on top of them.
        for row in drawn.rows.clone() {
            let y = pane.rect.top()
                + (layout.rows.offset(row) + layout.rows.size(row) - pane.scroll.y) as f32;
            painter.hline(pane.rect.x_range(), y.round(), stroke);
        }
        for col in drawn.cols.clone() {
            let x = pane.rect.left()
                + (layout.cols.offset(col) + layout.cols.size(col) - pane.scroll.x) as f32;
            painter.vline(x.round(), pane.rect.y_range(), stroke);
        }

        // Fills over the gridlines: a shaded cell in Excel has no gridlines
        // showing through it, which is what makes a filled table read as solid.
        for cell in &drawn.cells {
            if let Some([r, g, b]) = cell.look.fill {
                painter.rect_filled(cell.rect, 0.0, egui::Color32::from_rgb(r, g, b));
            }
            match cell.overlay {
                Some(Overlay::Shade([r, g, b])) => {
                    painter.rect_filled(cell.rect, 0.0, egui::Color32::from_rgb(r, g, b));
                }
                Some(Overlay::Bar {
                    fraction,
                    color: [r, g, b],
                }) => {
                    let inset = cell.rect.shrink(1.0);
                    let bar = egui::Rect::from_min_size(
                        inset.min,
                        egui::vec2(inset.width() * fraction as f32, inset.height()),
                    );
                    painter.rect_filled(
                        bar,
                        1.0,
                        egui::Color32::from_rgb(r, g, b).gamma_multiply(0.85),
                    );
                }
                None => {}
            }
        }

        // Selection under the text.
        for range in self.selection.ranges() {
            let rect = super::rect_of_range(layout, *range, pane.rect, pane.scroll);
            let visible = rect.intersect(pane.rect);
            if !visible.is_positive() {
                continue;
            }
            painter.rect_filled(visible, 0.0, palette.selection_fill);
            painter.rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0, palette.selection_edge),
                egui::StrokeKind::Inside,
            );
        }

        // While a formula is being typed its references are outlined in the
        // same colours the text is coloured with, so `=SUM(B2:B9)` and the box
        // round B2:B9 are visibly the same thing.
        if let Some(open) = self.editor.as_ref().filter(|e| e.is_formula()) {
            for (index, range) in open.references().iter().enumerate() {
                let rect = rect_of_range(layout, *range, pane.rect, pane.scroll);
                if !rect.intersect(pane.rect).is_positive() {
                    continue;
                }
                let color = editor::REFERENCE_COLORS[index % editor::REFERENCE_COLORS.len()];
                painter.rect_stroke(
                    rect,
                    0.0,
                    egui::Stroke::new(1.5, color),
                    egui::StrokeKind::Inside,
                );
            }
        }

        // The fill preview, and the handle that starts one.
        if let (Some(Drag::Fill { from }), Some(to)) = (self.drag, self.fill_target) {
            let _ = from;
            let rect = rect_of_range(layout, to, pane.rect, pane.scroll);
            painter.rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0, palette.selection_edge),
                egui::StrokeKind::Inside,
            );
        } else if self.editor.is_none() {
            let handle = self.fill_handle(layout, pane);
            if handle.intersect(pane.rect).is_positive() {
                painter.rect_filled(handle, 1.0, palette.selection_edge);
            }
        }

        // Cell borders over the fills and under the text.
        for cell in &drawn.cells {
            let r = cell.rect;
            for (slot, from, to) in [
                (LEFT, r.left_top(), r.left_bottom()),
                (RIGHT, r.right_top(), r.right_bottom()),
                (TOP, r.left_top(), r.right_top()),
                (BOTTOM, r.left_bottom(), r.right_bottom()),
            ] {
                if let Some(edge) = cell.look.edges[slot] {
                    draw_edge(&painter, from, to, edge, self.zoom as f32);
                }
            }
        }

        for cell in &drawn.cells {
            if cell.text.is_empty() {
                continue;
            }
            let color = cell
                .color
                .map_or(palette.text, |[r, g, b]| egui::Color32::from_rgb(r, g, b));
            let padding = 3.0 + cell.look.indent as f32 * 9.0 * self.zoom as f32;
            let room = cell.rect.width() - padding - 3.0;
            // Points to logical pixels. The grid's default is 13 px for an 11 pt
            // font and every other size follows from that, so a 22 pt heading
            // comes out twice the height of 11 pt body text.
            let font = egui::FontId::proportional(
                (cell.look.size * 13.0 / 11.0 * self.zoom as f32).max(1.0),
            );
            let job = text_job(cell, font.clone(), color, room);
            let galley = ui.fonts_mut(|f| f.layout_job(job));

            // A number too wide for its column is not truncated — Excel shows a
            // row of hashes, because a clipped number reads as a smaller one.
            if cell.numeric && !cell.look.wrap && galley.size().x > room && cell.look.rotation == 0
            {
                let hashes = "#".repeat(((room / 7.0).max(1.0)) as usize);
                let job = text_job_for(&hashes, cell, font.clone(), color, f32::INFINITY);
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                let pos = egui::pos2(
                    cell.rect.right() - padding - galley.size().x,
                    cell.rect.center().y - galley.size().y / 2.0,
                );
                paint_text(&painter, pos, galley, color, cell.look.bold);
                continue;
            }

            // Text may run into empty neighbours; clip to what it borrowed.
            let clip = egui::Rect::from_min_max(
                cell.rect.min,
                egui::pos2(cell.rect.right() + cell.overflow, cell.rect.bottom()),
            )
            .intersect(pane.rect);
            let painter = painter.with_clip_rect(clip);

            let angle = match cell.look.rotation {
                // Stacked text is drawn upright rather than one glyph per line:
                // that layout is a paragraph problem, not a cell one.
                255 => 0.0,
                r if r <= 90 => -(r as f32).to_radians(),
                r if r <= 180 => ((r - 90) as f32).to_radians(),
                _ => 0.0,
            };
            if angle != 0.0 {
                // Rotation pivots on the text origin, so the anchor is the
                // bottom-left corner and the glyphs sweep up out of it.
                let pos = egui::pos2(
                    cell.rect.left() + padding,
                    cell.rect.bottom() - 2.0 - galley.size().y,
                );
                painter.add(egui::epaint::TextShape::new(pos, galley, color).with_angle(angle));
                continue;
            }

            let x = match resolved_align(cell) {
                HAlign::Right => cell.rect.right() - 3.0 - galley.size().x,
                HAlign::Center | HAlign::CenterContinuous | HAlign::Distributed => {
                    cell.rect.center().x - galley.size().x / 2.0
                }
                _ => cell.rect.left() + padding,
            };
            let y = match cell.look.vertical {
                VAlign::Top => cell.rect.top() + 1.0,
                VAlign::Bottom => cell.rect.bottom() - 1.0 - galley.size().y,
                _ => cell.rect.center().y - galley.size().y / 2.0,
            };
            paint_text(&painter, egui::pos2(x, y), galley, color, cell.look.bold);
        }

        // Charts over everything the cells drew, and under the cursor outline.
        // They float above the grid in Excel too — a chart is not clipped by
        // the cells it happens to sit on.
        for chart in &sheet.charts {
            let rect = super::chart::rect_of(layout, &chart.anchor, pane.rect, pane.scroll);
            if !rect.intersect(pane.rect).is_positive() {
                continue;
            }
            let series = super::chart::resolve(book, chart);
            super::chart::draw(
                &painter,
                rect,
                chart,
                &series,
                &super::chart::Style {
                    background: palette.background,
                    outline: palette.grid,
                    text: palette.text,
                    grid: palette.header_text,
                    zoom: self.zoom as f32,
                },
            );
        }

        // The active cell's heavier outline, drawn last so nothing covers it.
        let cursor = self.selection.cursor();
        let anchor = sheet.merge_at(cursor).map_or(cursor, |m| m.start);
        let rect = match sheet.merge_at(anchor) {
            Some(m) => super::rect_of_range(layout, *m, pane.rect, pane.scroll),
            None => rect_of(layout, anchor, pane.rect, pane.scroll),
        };
        if rect.intersects(pane.rect) {
            painter.rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(2.0, palette.selection_edge),
                egui::StrokeKind::Inside,
            );
        }
    }

    fn paint_headers(&self, ui: &egui::Ui, frame: &Frame) {
        let Frame {
            layout,
            palette,
            full,
            body,
            panes,
            ..
        } = *frame;
        let painter = ui.painter();
        let header_font = egui::FontId::proportional((11.0 * self.zoom) as f32);
        let corner = egui::Rect::from_min_size(full.min, body.min - full.min);
        painter.rect_filled(corner, 0.0, palette.header_bg);

        let selected_cols: Vec<_> = self.selection.ranges().to_vec();
        let stroke = egui::Stroke::new(1.0, palette.grid);

        for pane in panes {
            // Column letters, above each pane that shows columns.
            let strip = egui::Rect::from_min_max(
                egui::pos2(pane.rect.left(), full.top()),
                egui::pos2(pane.rect.right(), body.top()),
            );
            if strip.width() > 0.0 && pane.rect.top() <= body.top() + 0.5 {
                let p = painter.with_clip_rect(strip);
                p.rect_filled(strip, 0.0, palette.header_bg);
                let cols = layout
                    .cols
                    .visible(pane.scroll.x, pane.scroll.x + f64::from(pane.rect.width()));
                for col in cols {
                    let x = pane.rect.left() + (layout.cols.offset(col) - pane.scroll.x) as f32;
                    let w = layout.cols.size(col) as f32;
                    let cell = egui::Rect::from_min_size(
                        egui::pos2(x, strip.top()),
                        egui::vec2(w, strip.height()),
                    );
                    if selected_cols
                        .iter()
                        .any(|r| r.start.col <= col && col <= r.end.col)
                    {
                        p.rect_filled(cell, 0.0, palette.header_active);
                    }
                    p.vline(cell.right().round(), strip.y_range(), stroke);
                    p.text(
                        cell.center(),
                        egui::Align2::CENTER_CENTER,
                        column_name(col),
                        header_font.clone(),
                        palette.header_text,
                    );
                }
            }

            // Row numbers, left of each pane that shows rows.
            let gutter = egui::Rect::from_min_max(
                egui::pos2(full.left(), pane.rect.top()),
                egui::pos2(body.left(), pane.rect.bottom()),
            );
            if gutter.height() > 0.0 && pane.rect.left() <= body.left() + 0.5 {
                let p = painter.with_clip_rect(gutter);
                p.rect_filled(gutter, 0.0, palette.header_bg);
                let rows = layout
                    .rows
                    .visible(pane.scroll.y, pane.scroll.y + f64::from(pane.rect.height()));
                for row in rows {
                    let y = pane.rect.top() + (layout.rows.offset(row) - pane.scroll.y) as f32;
                    let h = layout.rows.size(row) as f32;
                    let cell = egui::Rect::from_min_size(
                        egui::pos2(gutter.left(), y),
                        egui::vec2(gutter.width(), h),
                    );
                    if selected_cols
                        .iter()
                        .any(|r| r.start.row <= row && row <= r.end.row)
                    {
                        p.rect_filled(cell, 0.0, palette.header_active);
                    }
                    p.hline(gutter.x_range(), cell.bottom().round(), stroke);
                    p.text(
                        cell.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("{}", row + 1),
                        header_font.clone(),
                        palette.header_text,
                    );
                }
            }
        }
        painter.rect_stroke(
            corner,
            0.0,
            egui::Stroke::new(1.0, palette.grid),
            egui::StrokeKind::Inside,
        );
    }

    /// The cell under a screen position, and which pane it belongs to.
    fn cell_at(
        &self,
        layout: &Layout,
        body: egui::Rect,
        panes: &[Pane],
        pos: egui::Pos2,
    ) -> Option<CellRef> {
        let pane = panes
            .iter()
            .find(|p| p.rect.contains(pos))
            .or_else(|| panes.last())?;
        let _ = body;
        let x = pane.scroll.x + f64::from(pos.x - pane.rect.left());
        let y = pane.scroll.y + f64::from(pos.y - pane.rect.top());
        Some(CellRef::new(
            layout.rows.index_at(y),
            layout.cols.index_at(x),
        ))
    }

    fn handle_input(
        &mut self,
        ui: &egui::Ui,
        book: &mut Workbook,
        response: &egui::Response,
        full: egui::Rect,
        body: egui::Rect,
    ) {
        let Some(sheet) = book.sheet(self.sheet_index) else {
            return;
        };
        let cached = self.layout.take().expect("set by show");
        let layout = &cached.2;
        let frozen = sheet.frozen.unwrap_or(CellRef::new(0, 0));
        let split = egui::vec2(
            layout.cols.offset(frozen.col) as f32,
            layout.rows.offset(frozen.row) as f32,
        );
        let panes = split_panes(body, split, self.scroll, layout, frozen);
        let palette = Palette::of(ui);
        let frame = Frame {
            layout,
            palette: &palette,
            full,
            body,
            panes: &panes,
            // Input only ever asks this frame about geometry.
            conditional: &Formatting::empty(),
        };

        let (scroll_delta, zoom_delta, modifiers) =
            ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers));

        if response.hovered() {
            if modifiers.ctrl && zoom_delta != 1.0 {
                let zoom = self.zoom * f64::from(zoom_delta);
                self.set_zoom(zoom);
            } else if scroll_delta != egui::Vec2::ZERO {
                self.scroll.x -= f64::from(scroll_delta.x);
                self.scroll.y -= f64::from(scroll_delta.y);
                self.scroll = self.scroll.clamped();
            }
        }

        // A drag in progress owns the pointer until the button comes back up.
        // The end condition is "no button is down", not "a release arrived this
        // frame": a release delivered while the pointer is outside the window,
        // or swallowed by another widget, would otherwise leave the grid
        // selecting cells under a pointer that is merely hovering.
        if let Some(drag) = self.drag {
            let (down, at) = ui.input(|i| (i.pointer.any_down(), i.pointer.interact_pos()));
            match (down, at) {
                (true, Some(pos)) => self.continue_drag(drag, book, &frame, pos),
                (false, _) => {
                    self.finish_drag(drag);
                    self.invalidate();
                }
                _ => {}
            }
            self.layout = Some(cached);
            return;
        }

        // On press, not on click: `clicked()` fires on *release*, so starting a
        // drag there begins one whose release has already happened.
        if let Some(pos) = response.interact_pointer_pos() {
            // A click inside the open editor is the user placing the caret, not
            // leaving the cell.
            let inside_editor = self
                .editor
                .as_ref()
                .and_then(|open| cell_rect(layout, &panes, open.at))
                .is_some_and(|rect| rect.expand(2.0).contains(pos));
            if ui.input(|i| i.pointer.any_pressed()) && !inside_editor {
                // Clicking away from an open editor keeps what was typed, the
                // way every spreadsheet does.
                self.commit(None);
                self.begin_drag(book, &frame, pos, modifiers);
            }
        }
        if response.double_clicked() {
            self.open_editor(book, Mode::Edit);
        }

        self.handle_keys(ui, book, body, layout);
        self.layout = Some(cached);
    }

    fn begin_drag(
        &mut self,
        book: &Workbook,
        frame: &Frame,
        pos: egui::Pos2,
        modifiers: egui::Modifiers,
    ) {
        let Some(sheet) = book.sheet(self.sheet_index) else {
            return;
        };
        let Frame {
            layout,
            full,
            body,
            panes,
            ..
        } = *frame;
        let in_column_header = pos.y < body.top() && pos.x >= body.left();
        let in_row_header = pos.x < body.left() && pos.y >= body.top();

        if in_column_header {
            let pane = panes
                .iter()
                .find(|p| p.rect.x_range().contains(pos.x))
                .unwrap_or(&panes[0]);
            let x = pane.scroll.x + f64::from(pos.x - pane.rect.left());
            let col = layout.cols.index_at(x);
            let edge = layout.cols.offset(col) + layout.cols.size(col) - pane.scroll.x;
            // Within a few pixels of the edge, the drag resizes instead.
            if (f64::from(pos.x - pane.rect.left()) - edge).abs() < f64::from(RESIZE_GRAB) {
                self.before_resize = Some(Geometry::of(sheet));
                self.drag = Some(Drag::ResizeColumn {
                    index: col,
                    origin: pos.x,
                    start: layout.cols.size(col) as f32,
                });
            } else {
                self.selection.select_columns(col, col, modifiers.ctrl);
                self.drag = Some(Drag::Select);
            }
            return;
        }
        if in_row_header {
            let pane = panes
                .iter()
                .find(|p| p.rect.y_range().contains(pos.y))
                .unwrap_or(&panes[0]);
            let y = pane.scroll.y + f64::from(pos.y - pane.rect.top());
            let row = layout.rows.index_at(y);
            let edge = layout.rows.offset(row) + layout.rows.size(row) - pane.scroll.y;
            if (f64::from(pos.y - pane.rect.top()) - edge).abs() < f64::from(RESIZE_GRAB) {
                self.before_resize = Some(Geometry::of(sheet));
                self.drag = Some(Drag::ResizeRow {
                    index: row,
                    origin: pos.y,
                    start: layout.rows.size(row) as f32,
                });
            } else {
                self.selection.select_rows(row, row, modifiers.ctrl);
                self.drag = Some(Drag::Select);
            }
            return;
        }
        // The corner box selects everything.
        if pos.x < body.left() && pos.y < body.top() && full.contains(pos) {
            self.selection.select_all();
            return;
        }

        // The fill handle sits on top of the cell under it, so it is tested first.
        for pane in panes {
            if pane.rect.contains(pos) && self.fill_handle(layout, pane).contains(pos) {
                self.drag = Some(Drag::Fill {
                    from: self.selection.active_range(),
                });
                self.fill_target = Some(self.selection.active_range());
                return;
            }
        }

        let Some(at) = self.cell_at(layout, body, panes, pos) else {
            return;
        };
        if modifiers.shift {
            self.selection.extend_to(at, sheet);
        } else if modifiers.ctrl {
            self.selection.add(at, sheet);
        } else {
            self.selection.move_to(at, sheet);
        }
        self.drag = Some(Drag::Select);
    }

    fn continue_drag(&mut self, drag: Drag, book: &mut Workbook, frame: &Frame, pos: egui::Pos2) {
        let (layout, body, panes) = (frame.layout, frame.body, frame.panes);
        match drag {
            Drag::Fill { from } => {
                let Some(at) = self.cell_at(layout, body, panes, pos) else {
                    return;
                };
                self.fill_target = Some(fill_target(from, at));
            }
            Drag::Select => {
                let Some(at) = self.cell_at(layout, body, panes, pos) else {
                    return;
                };
                if let Some(sheet) = book.sheet(self.sheet_index) {
                    self.selection.extend_to(at, sheet);
                }
            }
            Drag::ResizeColumn {
                index,
                origin,
                start,
            } => {
                let width = (start + (pos.x - origin)).max(0.0);
                if let Some(sheet) = book.sheet_mut(self.sheet_index) {
                    let chars = super::axis::pixels_to_column(f64::from(width), self.zoom);
                    sheet.column_widths.insert(index, chars);
                }
                self.invalidate();
            }
            Drag::ResizeRow {
                index,
                origin,
                start,
            } => {
                let height = (start + (pos.y - origin)).max(0.0);
                if let Some(sheet) = book.sheet_mut(self.sheet_index) {
                    let points = super::axis::pixels_to_row(f64::from(height), self.zoom);
                    sheet.row_heights.insert(index, points);
                }
                self.invalidate();
            }
        }
    }

    /// The pointer came up. A resize or a fill only becomes an undoable change
    /// here, at the end, rather than once per frame of the drag.
    fn finish_drag(&mut self, drag: Drag) {
        self.drag = None;
        match drag {
            Drag::Fill { from } => {
                if let Some(to) = self.fill_target.take() {
                    if to != from {
                        self.actions.push(Action::Fill { from, to });
                    }
                }
            }
            Drag::ResizeColumn { .. } | Drag::ResizeRow { .. } => {
                if let Some(before) = self.before_resize.take() {
                    self.actions.push(Action::Resized(before));
                }
            }
            Drag::Select => {}
        }
    }

    /// Opens the editor on the cursor cell.
    fn open_editor(&mut self, book: &Workbook, mode: Mode) {
        if self.editor.is_some() {
            return;
        }
        let at = self.selection.cursor();
        self.editor = Some(match mode {
            Mode::Edit => Editor::editing(at, super::source_text(book, self.sheet_index, at)),
            Mode::Enter => Editor::typing(at, String::new()),
        });
    }

    fn handle_keys(&mut self, ui: &egui::Ui, book: &Workbook, body: egui::Rect, layout: &Layout) {
        let Some(sheet) = book.sheet(self.sheet_index) else {
            return;
        };
        let events = ui.input(|i| i.events.clone());
        let mut moved = false;

        for event in events {
            match event {
                // Copy and cut arrive as their own events rather than as key
                // presses, because the platform may raise them from a menu.
                egui::Event::Copy if self.editor.is_none() => {
                    self.actions.push(Action::Copy { cut: false })
                }
                egui::Event::Cut if self.editor.is_none() => {
                    self.actions.push(Action::Copy { cut: true })
                }
                egui::Event::Paste(text) if self.editor.is_none() => {
                    self.actions.push(Action::Paste(text))
                }
                // Typing over a selected cell starts an edit, seeded with what
                // was typed. Control characters are not typing.
                egui::Event::Text(text)
                    if self.editor.is_none() && !text.chars().any(char::is_control) =>
                {
                    self.editor = Some(Editor::typing(self.selection.cursor(), text));
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if self.editor.is_some() {
                        self.editing_key(key, modifiers);
                    } else {
                        moved |= self.grid_key(key, modifiers, sheet, book, body, layout);
                    }
                }
                _ => {}
            }
        }

        if moved {
            let cursor = self.selection.cursor();
            self.scroll_into_view(cursor, body.size(), sheet);
        }
    }

    /// Keys while the editor is open.
    ///
    /// The mode is the whole point: an arrow key commits and moves when the
    /// editor was opened by typing, and moves the caret when it was opened with
    /// F2. Getting this backwards makes a half-typed formula unusable in one
    /// mode and cell navigation unusable in the other.
    fn editing_key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        let enter_mode = self.editor.as_ref().is_some_and(|e| e.mode == Mode::Enter);
        let direction = match key {
            egui::Key::ArrowUp => Some(Direction::Up),
            egui::Key::ArrowDown => Some(Direction::Down),
            egui::Key::ArrowLeft => Some(Direction::Left),
            egui::Key::ArrowRight => Some(Direction::Right),
            _ => None,
        };
        if let Some(direction) = direction {
            if enter_mode {
                self.commit(Some(direction));
            }
            return;
        }
        match key {
            egui::Key::Enter => self.commit(Some(if modifiers.shift {
                Direction::Up
            } else {
                Direction::Down
            })),
            egui::Key::Tab => self.commit(Some(if modifiers.shift {
                Direction::Left
            } else {
                Direction::Right
            })),
            // Escape throws the edit away. Nothing is reported, so nothing is
            // written and there is nothing to undo.
            egui::Key::Escape => self.editor = None,
            // F2 promotes a typing edit to a caret edit, as Excel does.
            egui::Key::F2 => {
                if let Some(open) = &mut self.editor {
                    open.mode = Mode::Edit;
                }
            }
            _ => {}
        }
    }

    /// Keys while the grid itself has the keyboard. Returns whether the cursor
    /// moved, so the caller can scroll it back into view.
    fn grid_key(
        &mut self,
        key: egui::Key,
        modifiers: egui::Modifiers,
        sheet: &Sheet,
        book: &Workbook,
        body: egui::Rect,
        layout: &Layout,
    ) -> bool {
        let direction = match key {
            egui::Key::ArrowUp => Some(Direction::Up),
            egui::Key::ArrowDown => Some(Direction::Down),
            egui::Key::ArrowLeft => Some(Direction::Left),
            egui::Key::ArrowRight => Some(Direction::Right),
            _ => None,
        };
        if let Some(direction) = direction {
            if modifiers.ctrl {
                self.selection.jump(direction, sheet, modifiers.shift);
            } else if modifiers.shift {
                self.selection.extend(direction, sheet);
            } else {
                self.selection.step(direction, sheet);
            }
            return true;
        }

        match key {
            egui::Key::Tab => {
                let dir = if modifiers.shift {
                    Direction::Left
                } else {
                    Direction::Right
                };
                self.selection.advance(dir, sheet);
                return true;
            }
            egui::Key::Enter => {
                let dir = if modifiers.shift {
                    Direction::Up
                } else {
                    Direction::Down
                };
                self.selection.advance(dir, sheet);
                return true;
            }
            egui::Key::F2 => self.open_editor(book, Mode::Edit),
            // Backspace opens an editor on an emptied cell; Delete just empties.
            egui::Key::Backspace => self.open_editor(book, Mode::Enter),
            egui::Key::Delete => self.actions.push(Action::Clear),
            egui::Key::Z if modifiers.ctrl => self.actions.push(if modifiers.shift {
                Action::Redo
            } else {
                Action::Undo
            }),
            egui::Key::Y if modifiers.ctrl => self.actions.push(Action::Redo),
            egui::Key::D if modifiers.ctrl => self.fill_within(Direction::Down),
            egui::Key::R if modifiers.ctrl => self.fill_within(Direction::Right),
            egui::Key::A if modifiers.ctrl => self.selection.select_all(),
            egui::Key::B if modifiers.ctrl => self.actions.push(Action::Format(Format::Bold)),
            egui::Key::I if modifiers.ctrl => self.actions.push(Action::Format(Format::Italic)),
            egui::Key::U if modifiers.ctrl => self.actions.push(Action::Format(Format::Underline)),
            egui::Key::Num5 if modifiers.ctrl => self.actions.push(Action::Format(Format::Strike)),
            egui::Key::Space if modifiers.ctrl => {
                let range = self.selection.active_range();
                self.selection
                    .select_columns(range.start.col, range.end.col, modifiers.shift);
            }
            egui::Key::Space if modifiers.shift => {
                let range = self.selection.active_range();
                self.selection
                    .select_rows(range.start.row, range.end.row, false);
            }
            // Ctrl-plus and Ctrl-minus. Excel asks which way with a dialog; the
            // selection already answers it whenever it covers whole rows or
            // whole columns, which is how people use these keys anyway.
            egui::Key::Plus | egui::Key::Equals if modifiers.ctrl => {
                self.actions.push(Action::Insert(self.structural_axis()));
            }
            egui::Key::Minus if modifiers.ctrl => {
                self.actions.push(Action::Delete(self.structural_axis()));
            }
            egui::Key::Home => {
                let at = if modifiers.ctrl {
                    CellRef::new(0, 0)
                } else {
                    CellRef::new(self.selection.cursor().row, 0)
                };
                self.selection.move_to(at, sheet);
                return true;
            }
            // A page is however many rows currently fit, which is why it
            // depends on the viewport rather than on a constant.
            egui::Key::PageDown | egui::Key::PageUp => {
                let rows = layout
                    .rows
                    .visible(self.scroll.y, self.scroll.y + f64::from(body.height()));
                let page = rows.clone().count().max(1) as u32;
                let cursor = self.selection.cursor();
                let row = if key == egui::Key::PageDown {
                    cursor.row.saturating_add(page)
                } else {
                    cursor.row.saturating_sub(page)
                };
                let at = CellRef::new(row.min(ss_model::cell::MAX_ROWS - 1), cursor.col);
                if modifiers.shift {
                    self.selection.extend_to(at, sheet);
                } else {
                    self.selection.move_to(at, sheet);
                }
                return true;
            }
            _ => {}
        }
        false
    }

    /// Ctrl-D and Ctrl-R: fill the selection from its own first row or column.
    fn fill_within(&mut self, direction: Direction) {
        let to = self.selection.active_range();
        let from = match direction {
            Direction::Down => CellRange::new(to.start, CellRef::new(to.start.row, to.end.col)),
            Direction::Right => CellRange::new(to.start, CellRef::new(to.end.row, to.start.col)),
            _ => return,
        };
        if from != to {
            self.actions.push(Action::Fill { from, to });
        }
    }

    /// Whether an insert or delete should move rows or columns.
    ///
    /// A selection of whole columns says columns; anything else says rows,
    /// which is the answer for a selection of whole rows and the safer default
    /// for a selection of neither.
    fn structural_axis(&self) -> Axis {
        let range = self.selection.active_range();
        if range.rows() == ss_model::cell::MAX_ROWS && range.cols() < ss_model::cell::MAX_COLS {
            Axis::Columns
        } else {
            Axis::Rows
        }
    }
}

/// The up-to-four views a frozen sheet is drawn as.
fn split_panes(
    body: egui::Rect,
    split: egui::Vec2,
    scroll: Scroll,
    layout: &Layout,
    frozen: CellRef,
) -> Vec<Pane> {
    let frozen_x = layout.cols.offset(frozen.col);
    let frozen_y = layout.rows.offset(frozen.row);
    let mut panes = Vec::with_capacity(4);

    let left = egui::Rect::from_min_max(
        body.min,
        egui::pos2(body.left() + split.x, body.top() + split.y),
    );
    let top = egui::Rect::from_min_max(
        egui::pos2(body.left() + split.x, body.top()),
        egui::pos2(body.right(), body.top() + split.y),
    );
    let side = egui::Rect::from_min_max(
        egui::pos2(body.left(), body.top() + split.y),
        egui::pos2(body.left() + split.x, body.bottom()),
    );
    let main = egui::Rect::from_min_max(
        egui::pos2(body.left() + split.x, body.top() + split.y),
        body.max,
    );

    // The frozen bands are only panes when they exist.
    if frozen.col > 0 && frozen.row > 0 {
        panes.push(Pane {
            rect: left,
            scroll: Scroll { x: 0.0, y: 0.0 },
        });
    }
    if frozen.row > 0 {
        panes.push(Pane {
            rect: top,
            scroll: Scroll {
                x: frozen_x + scroll.x,
                y: 0.0,
            },
        });
    }
    if frozen.col > 0 {
        panes.push(Pane {
            rect: side,
            scroll: Scroll {
                x: 0.0,
                y: frozen_y + scroll.y,
            },
        });
    }
    panes.push(Pane {
        rect: main,
        scroll: Scroll {
            x: frozen_x + scroll.x,
            y: frozen_y + scroll.y,
        },
    });
    panes
}

/// Builds the laid-out text for one cell.
///
/// A [`egui::text::LayoutJob`] rather than a plain string because underline and
/// strikethrough are properties of the layout rather than of the drawing, and
/// because wrapping is one field on the same object.
fn text_job(
    cell: &super::PaintCell,
    font: egui::FontId,
    color: egui::Color32,
    room: f32,
) -> egui::text::LayoutJob {
    text_job_for(&cell.text, cell, font, color, room)
}

/// The same, for text that is not the cell's own — the row of hashes a number
/// too wide for its column is replaced by.
fn text_job_for(
    text: &str,
    cell: &super::PaintCell,
    font: egui::FontId,
    color: egui::Color32,
    room: f32,
) -> egui::text::LayoutJob {
    let hairline = (font.size / 14.0).max(1.0);
    let rule = |on: bool| {
        if on {
            egui::Stroke::new(hairline, color)
        } else {
            egui::Stroke::NONE
        }
    };
    let mut job = egui::text::LayoutJob::single_section(
        text.to_string(),
        egui::TextFormat {
            font_id: font,
            color,
            italics: cell.look.italic,
            underline: rule(cell.look.underline),
            strikethrough: rule(cell.look.strike),
            ..Default::default()
        },
    );
    job.wrap.max_width = if cell.look.wrap && room.is_finite() {
        room.max(1.0)
    } else {
        f32::INFINITY
    };
    job
}

/// Draws a galley, faking bold by drawing it twice a fraction of a pixel apart.
///
/// egui ships one weight of its default font and no way to ask for another, so
/// bold here is synthetic. At cell sizes it is convincing; what it is *not* is a
/// real bold face, so a heavy heading reads a little lighter than in Excel.
/// Italic is genuine — epaint slants the glyphs itself.
fn paint_text(
    painter: &egui::Painter,
    pos: egui::Pos2,
    galley: std::sync::Arc<egui::Galley>,
    color: egui::Color32,
    bold: bool,
) {
    if bold {
        painter.galley(pos + egui::vec2(0.55, 0.0), galley.clone(), color);
    }
    painter.galley(pos, galley, color);
}

/// General alignment resolved against the value: numbers right, text left.
///
/// It has to happen here rather than in the style, because one style on a
/// number and on a label aligns them differently — which is the whole reason
/// `General` is a distinct value and not just "left".
fn resolved_align(cell: &super::PaintCell) -> HAlign {
    match cell.look.horizontal {
        HAlign::General if cell.numeric => HAlign::Right,
        HAlign::General => HAlign::Left,
        other => other,
    }
}

/// One border edge, dashed if its style says so.
fn draw_edge(
    painter: &egui::Painter,
    from: egui::Pos2,
    to: egui::Pos2,
    edge: PaintEdge,
    zoom: f32,
) {
    let color = egui::Color32::from_rgb(edge.color[0], edge.color[1], edge.color[2]);
    let width = (edge.style.width() * zoom).max(1.0);
    let stroke = egui::Stroke::new(width, color);
    match edge.style.dash() {
        Some((on, off)) => {
            painter.add(egui::Shape::dashed_line(
                &[from, to],
                stroke,
                on * zoom,
                off * zoom,
            ));
        }
        None if edge.style == BorderStyle::Double => {
            // Two hairlines with a gap.
            // Two hairlines with a gap. Drawn as one thick line it is
            // indistinguishable from `medium`, which is a different border.
            let normal = if (to.x - from.x).abs() > (to.y - from.y).abs() {
                egui::vec2(0.0, 2.0 * zoom)
            } else {
                egui::vec2(2.0 * zoom, 0.0)
            };
            let thin = egui::Stroke::new(1.0, color);
            painter.line_segment([from, to], thin);
            painter.line_segment([from + normal, to + normal], thin);
        }
        None => {
            painter.line_segment([from, to], stroke);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Selection;

    /// Drives one egui frame with the given events and returns the view.
    fn frame(
        view: &mut GridView,
        book: &mut Workbook,
        events: Vec<egui::Event>,
        ctx: &egui::Context,
    ) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            view.show(ui, book);
        });
        // Nobody is going to upload the font atlas to a GPU here.
        out.textures_delta.clear();
    }

    fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }
    }

    #[test]
    fn a_click_does_not_leave_a_drag_running() {
        let ctx = egui::Context::default();
        let mut book = Workbook::blank();
        let mut view = GridView::default();
        let start = egui::pos2(200.0, 200.0);

        // Press and release in place: an ordinary click.
        frame(
            &mut view,
            &mut book,
            vec![egui::Event::PointerMoved(start)],
            &ctx,
        );
        frame(&mut view, &mut book, vec![press(start, true)], &ctx);
        frame(&mut view, &mut book, vec![press(start, false)], &ctx);
        let clicked = view.selection.cursor();
        assert!(
            view.drag.is_none(),
            "the button is up, so nothing is being dragged"
        );

        // Now move the pointer across the sheet with no button held. Nothing
        // about the selection may change.
        for x in [300.0, 400.0, 500.0_f32] {
            frame(
                &mut view,
                &mut book,
                vec![egui::Event::PointerMoved(egui::pos2(x, 400.0))],
                &ctx,
            );
        }
        assert_eq!(view.selection.cursor(), clicked);
        assert_eq!(view.selection, Selection::at(clicked));
    }

    fn key(k: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn plain(k: egui::Key) -> egui::Event {
        key(k, egui::Modifiers::default())
    }

    fn ctrl(k: egui::Key) -> egui::Event {
        key(k, egui::Modifiers::CTRL)
    }

    /// A grid with the keyboard, ready to be typed at.
    fn typing() -> (egui::Context, Workbook, GridView) {
        (
            egui::Context::default(),
            Workbook::blank(),
            GridView::default(),
        )
    }

    #[test]
    fn typing_a_character_opens_the_editor_with_it() {
        let (ctx, mut book, mut view) = typing();
        frame(
            &mut view,
            &mut book,
            vec![egui::Event::Text("5".into())],
            &ctx,
        );
        let open = view.editor.as_ref().expect("editor opened");
        assert_eq!(open.text, "5");
        assert_eq!(open.mode, Mode::Enter, "typed edits commit on arrow keys");
    }

    #[test]
    fn enter_commits_and_moves_down() {
        let (ctx, mut book, mut view) = typing();
        frame(
            &mut view,
            &mut book,
            vec![egui::Event::Text("hi".into())],
            &ctx,
        );
        frame(&mut view, &mut book, vec![plain(egui::Key::Enter)], &ctx);

        assert!(view.editor.is_none());
        assert_eq!(
            view.take_actions(),
            vec![Action::Commit {
                at: CellRef::new(0, 0),
                text: "hi".into(),
                advance: Some(Direction::Down),
            }]
        );
    }

    #[test]
    fn escape_throws_the_edit_away_without_reporting_it() {
        let (ctx, mut book, mut view) = typing();
        frame(
            &mut view,
            &mut book,
            vec![egui::Event::Text("oops".into())],
            &ctx,
        );
        frame(&mut view, &mut book, vec![plain(egui::Key::Escape)], &ctx);

        assert!(view.editor.is_none());
        assert!(
            view.take_actions().is_empty(),
            "nothing was written, so there is nothing to undo"
        );
    }

    #[test]
    fn an_arrow_key_commits_a_typed_edit_but_not_an_f2_edit() {
        // This is the difference between Excel's enter and edit modes, and it is
        // what makes `=A1+` followed by an arrow key point at a cell.
        let (ctx, mut book, mut view) = typing();
        frame(
            &mut view,
            &mut book,
            vec![egui::Event::Text("1".into())],
            &ctx,
        );
        frame(
            &mut view,
            &mut book,
            vec![plain(egui::Key::ArrowRight)],
            &ctx,
        );
        assert!(view.editor.is_none(), "typed edit committed");
        assert_eq!(view.take_actions().len(), 1);

        frame(&mut view, &mut book, vec![plain(egui::Key::F2)], &ctx);
        frame(
            &mut view,
            &mut book,
            vec![plain(egui::Key::ArrowRight)],
            &ctx,
        );
        assert!(view.editor.is_some(), "F2 edit keeps the caret in the text");
        assert!(view.take_actions().is_empty());
    }

    #[test]
    fn the_keyboard_reports_what_it_cannot_do_itself() {
        let (ctx, mut book, mut view) = typing();
        frame(&mut view, &mut book, vec![plain(egui::Key::Delete)], &ctx);
        frame(&mut view, &mut book, vec![ctrl(egui::Key::Z)], &ctx);
        frame(&mut view, &mut book, vec![ctrl(egui::Key::Y)], &ctx);
        frame(&mut view, &mut book, vec![egui::Event::Copy], &ctx);
        frame(
            &mut view,
            &mut book,
            vec![egui::Event::Paste("a\tb".into())],
            &ctx,
        );

        assert_eq!(
            view.take_actions(),
            vec![
                Action::Clear,
                Action::Undo,
                Action::Redo,
                Action::Copy { cut: false },
                Action::Paste("a\tb".into()),
            ]
        );
    }

    #[test]
    fn navigation_keys_do_nothing_while_a_cell_is_being_edited() {
        let (ctx, mut book, mut view) = typing();
        frame(&mut view, &mut book, vec![plain(egui::Key::F2)], &ctx);
        frame(&mut view, &mut book, vec![plain(egui::Key::Delete)], &ctx);
        frame(&mut view, &mut book, vec![ctrl(egui::Key::Z)], &ctx);

        assert!(view.editor.is_some());
        assert!(
            view.take_actions().is_empty(),
            "Delete inside an editor deletes a character, not the sheet"
        );
    }

    #[test]
    fn ctrl_d_fills_the_selection_from_its_own_first_row() {
        let (ctx, mut book, mut view) = typing();
        view.selection = Selection::at(CellRef::new(0, 0));
        view.selection
            .extend_to(CellRef::new(3, 1), book.sheet(0).expect("sheet"));

        frame(&mut view, &mut book, vec![ctrl(egui::Key::D)], &ctx);
        assert_eq!(
            view.take_actions(),
            vec![Action::Fill {
                from: CellRange::new(CellRef::new(0, 0), CellRef::new(0, 1)),
                to: CellRange::new(CellRef::new(0, 0), CellRef::new(3, 1)),
            }]
        );
    }

    #[test]
    fn a_fill_drag_grows_along_whichever_axis_was_pulled_further() {
        let from = CellRange::new(CellRef::new(0, 0), CellRef::new(0, 0));
        assert_eq!(
            fill_target(from, CellRef::new(5, 1)),
            CellRange::new(CellRef::new(0, 0), CellRef::new(5, 0)),
            "mostly downward"
        );
        assert_eq!(
            fill_target(from, CellRef::new(1, 5)),
            CellRange::new(CellRef::new(0, 0), CellRef::new(0, 5)),
            "mostly rightward"
        );
    }

    #[test]
    fn holding_the_button_down_does_extend_the_selection() {
        let ctx = egui::Context::default();
        let mut book = Workbook::blank();
        let mut view = GridView::default();
        let start = egui::pos2(200.0, 200.0);

        frame(
            &mut view,
            &mut book,
            vec![egui::Event::PointerMoved(start)],
            &ctx,
        );
        frame(&mut view, &mut book, vec![press(start, true)], &ctx);
        frame(
            &mut view,
            &mut book,
            vec![egui::Event::PointerMoved(egui::pos2(500.0, 400.0))],
            &ctx,
        );
        assert!(
            !view.selection.is_single_cell(),
            "a held drag selects a block"
        );
    }
}
