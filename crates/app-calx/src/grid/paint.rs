//! Drawing the grid, and the mouse and keyboard that drive it.
//!
//! Frozen panes are why this is not one loop. A sheet with a frozen top row and
//! frozen first column is drawn as *four* independent views of the same sheet:
//! the corner scrolls with neither axis, the top strip scrolls horizontally
//! only, the left strip vertically only, and the body with both. Each is the
//! same [`plan`](super::plan) call with a different scroll offset and clip
//! rectangle, which is the whole reason `plan` takes those as parameters rather
//! than reading them from the view.

use ss_model::{column_name, CellRef, Sheet, Workbook};
use ui_kit::egui;

use super::{plan, rect_of, Direction, Drag, GridView, Layout, Scroll, RESIZE_GRAB};

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
        let cached = self.layout.take().expect("just ensured");
        let layout = &cached.2;

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

        self.layout = Some(cached);
        self.handle_input(ui, book, &response, full, body);
        response
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
        let drawn = plan(book, sheet, layout, pane.rect, pane.scroll);
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

        let font = egui::FontId::proportional((13.0 * self.zoom) as f32);
        for cell in &drawn.cells {
            let color = cell
                .color
                .map_or(palette.text, |[r, g, b]| egui::Color32::from_rgb(r, g, b));
            let galley = painter.layout_no_wrap(cell.text.clone(), font.clone(), color);
            let padding = 3.0;
            let room = cell.rect.width() - padding * 2.0;

            // A number too wide for its column is not truncated — Excel shows a
            // row of hashes, because a clipped number reads as a smaller one.
            if cell.numeric && galley.size().x > room {
                let hashes = "#".repeat(((room / 7.0).max(1.0)) as usize);
                let galley = painter.layout_no_wrap(hashes, font.clone(), color);
                let pos = egui::pos2(
                    cell.rect.right() - padding - galley.size().x,
                    cell.rect.center().y - galley.size().y / 2.0,
                );
                painter.galley(pos, galley, color);
                continue;
            }

            let x = if cell.numeric {
                cell.rect.right() - padding - galley.size().x
            } else {
                cell.rect.left() + padding
            };
            let pos = egui::pos2(x, cell.rect.center().y - galley.size().y / 2.0);
            // Text may run into empty neighbours; clip to what it borrowed.
            let room = egui::Rect::from_min_max(
                cell.rect.min,
                egui::pos2(cell.rect.right() + cell.overflow, cell.rect.bottom()),
            );
            painter
                .with_clip_rect(room.intersect(pane.rect))
                .galley(pos, galley, color);
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

        // A drag in progress owns the pointer until it is released.
        if let Some(drag) = self.drag {
            if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                self.continue_drag(drag, book, &frame, pos);
            }
            if ui.input(|i| i.pointer.any_released()) {
                self.drag = None;
                self.invalidate();
            }
            self.layout = Some(cached);
            return;
        }

        if let Some(pos) = response.interact_pointer_pos() {
            if response.drag_started() || response.clicked() {
                self.begin_drag(book, &frame, pos, modifiers);
            }
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

    fn handle_keys(&mut self, ui: &egui::Ui, book: &Workbook, body: egui::Rect, layout: &Layout) {
        let Some(sheet) = book.sheet(self.sheet_index) else {
            return;
        };
        let events = ui.input(|i| i.events.clone());
        let mut moved = false;
        for event in events {
            let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
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
                moved = true;
                continue;
            }
            match key {
                egui::Key::Tab => {
                    let dir = if modifiers.shift {
                        Direction::Left
                    } else {
                        Direction::Right
                    };
                    self.selection.advance(dir, sheet);
                    moved = true;
                }
                egui::Key::Enter => {
                    let dir = if modifiers.shift {
                        Direction::Up
                    } else {
                        Direction::Down
                    };
                    self.selection.advance(dir, sheet);
                    moved = true;
                }
                egui::Key::A if modifiers.ctrl => self.selection.select_all(),
                egui::Key::Home => {
                    let at = if modifiers.ctrl {
                        CellRef::new(0, 0)
                    } else {
                        CellRef::new(self.selection.cursor().row, 0)
                    };
                    self.selection.move_to(at, sheet);
                    moved = true;
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
                    moved = true;
                }
                _ => {}
            }
        }
        if moved {
            let cursor = self.selection.cursor();
            self.scroll_into_view(cursor, body.size(), sheet);
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
