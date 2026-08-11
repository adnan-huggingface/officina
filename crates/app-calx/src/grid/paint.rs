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
    Scroll, Selection, BOTTOM, LEFT, RESIZE_GRAB, RIGHT, TOP,
};
use ss_formula::cond::{Formatting, Overlay};
use ss_model::style::{BorderStyle, HAlign, VAlign};

/// Side of the little square at the corner of the selection.
const FILL_HANDLE: f32 = 6.0;

/// Thickness of the scrollbars along the right and bottom edges.
const SCROLLBAR: f32 = 14.0;

/// The shortest a thumb may get. Proportional all the way down means a thumb
/// one pixel tall on a long sheet, which is not something anyone can grab.
const MIN_THUMB: f32 = 28.0;

/// Where the scrollbars are and how far they reach.
///
/// Recomputed every frame from the layout and kept on the view, because the
/// input pass runs after the paint pass and needs the same rectangles rather
/// than a second derivation of them.
#[derive(Debug, Clone, Copy)]
pub struct Bars {
    pub vertical: egui::Rect,
    pub horizontal: egui::Rect,
    pub vertical_thumb: egui::Rect,
    pub horizontal_thumb: egui::Rect,
    /// The largest scroll offset there is anything to see at.
    pub extent: egui::Vec2,
}

impl Default for Bars {
    fn default() -> Self {
        // `NOTHING` is the empty rectangle, which every hit test rejects: a
        // frame that has not drawn the bars yet has no bars to click on.
        Bars {
            vertical: egui::Rect::NOTHING,
            horizontal: egui::Rect::NOTHING,
            vertical_thumb: egui::Rect::NOTHING,
            horizontal_thumb: egui::Rect::NOTHING,
            extent: egui::Vec2::ZERO,
        }
    }
}

/// The grid's own colours.
///
/// Fixed rather than taken from the surrounding theme, and that is deliberate:
/// a cell with no fill is *paper*. The workbook chose black text on white and
/// nothing else, so tinting the canvas to match a dark application chrome shows
/// the user a document they did not make — and a themed heading resolved
/// against Excel's light scheme, drawn on a dark canvas, disappears entirely.
/// The chrome around the grid follows the theme; the page does not.
struct Palette {
    grid: egui::Color32,
    text: egui::Color32,
    header_bg: egui::Color32,
    header_line: egui::Color32,
    header_text: egui::Color32,
    header_active: egui::Color32,
    header_active_text: egui::Color32,
    selection_fill: egui::Color32,
    selection_edge: egui::Color32,
    background: egui::Color32,
    scrollbar: egui::Color32,
    scrollbar_track: egui::Color32,
}

impl Palette {
    fn of(_ui: &egui::Ui) -> Self {
        let accent = egui::Color32::from_rgb(0x21, 0x73, 0x46);
        Palette {
            grid: egui::Color32::from_rgb(0xD4, 0xD4, 0xD4),
            text: egui::Color32::BLACK,
            header_bg: egui::Color32::from_rgb(0xF5, 0xF5, 0xF5),
            header_line: egui::Color32::from_rgb(0xC6, 0xC6, 0xC6),
            header_text: egui::Color32::from_rgb(0x44, 0x44, 0x44),
            header_active: egui::Color32::from_rgb(0xCB, 0xE1, 0xD4),
            header_active_text: accent,
            selection_fill: accent.gamma_multiply(0.10),
            selection_edge: accent,
            background: egui::Color32::WHITE,
            scrollbar: egui::Color32::from_rgb(0xB4, 0xB4, 0xB4),
            scrollbar_track: egui::Color32::from_rgb(0xF0, 0xF0, 0xF0),
        }
    }
}

/// One filter arrow: where it is, which column it speaks for, and whether that
/// column is currently hiding anything.
struct Arrow {
    rect: egui::Rect,
    /// An offset into the filter's range, which is what `colId` means.
    col: u32,
    filtering: bool,
}

/// The arrows for whichever of the filter's columns are on screen.
///
/// Capped: a filter over a whole row is legal, and a million arrows would be a
/// million rect tests per frame for a header that is at most a few dozen
/// columns wide on any screen.
fn filter_arrows(sheet: &Sheet, layout: &Layout, panes: &[Pane]) -> Vec<Arrow> {
    const MAX_COLUMNS: u32 = 4096;
    let Some(filter) = &sheet.filter else {
        return Vec::new();
    };
    let row = filter.header_row();
    let last = filter
        .range
        .end
        .col
        .min(filter.range.start.col + MAX_COLUMNS);
    (filter.range.start.col..=last)
        .filter_map(|col| {
            let cell = cell_rect(layout, panes, CellRef::new(row, col))?;
            // Inside the cell at its right edge, the way Excel draws it, and
            // never wider than the cell — a narrow column gets a small arrow
            // rather than one that overhangs its neighbour.
            let size = 15.0_f32.min(cell.width() - 1.0).min(cell.height() - 1.0);
            if size < 6.0 {
                return None;
            }
            Some(Arrow {
                rect: egui::Rect::from_min_size(
                    egui::pos2(cell.right() - size - 1.0, cell.center().y - size / 2.0),
                    egui::vec2(size, size),
                ),
                col: col - filter.range.start.col,
                filtering: filter.column(col - filter.range.start.col).is_some(),
            })
        })
        .collect()
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

/// The row or column boundary the pointer is close enough to grab, if any.
///
/// Both sides of the line count. Excel's grab zone straddles the boundary, and
/// one that existed only to the left of it would be four pixels wide in a
/// header sixty-four across — findable by somebody who already knew it was
/// there, which is the same as not being there at all.
///
/// The boundary returned is named by the row or column *before* it, because
/// that is the one a drag resizes. Where that one is hidden, dragging pulls it
/// back open, which is how it was shut in the first place.
fn header_edge(
    layout: &Layout,
    content: egui::Rect,
    body: egui::Rect,
    panes: &[Pane],
    pos: egui::Pos2,
) -> Option<(Axis, u32)> {
    if panes.is_empty() || !content.contains(pos) {
        return None;
    }
    let grab = f64::from(RESIZE_GRAB);
    if pos.y < body.top() && pos.x >= body.left() {
        let pane = panes
            .iter()
            .find(|p| p.rect.x_range().contains(pos.x))
            .unwrap_or(&panes[0]);
        let local = f64::from(pos.x - pane.rect.left());
        let col = layout.cols.index_at(pane.scroll.x + local);
        let after = layout.cols.offset(col) + layout.cols.size(col) - pane.scroll.x;
        if (local - after).abs() < grab {
            return Some((Axis::Columns, col));
        }
        let before = layout.cols.offset(col) - pane.scroll.x;
        if col > 0 && (local - before).abs() < grab {
            return Some((Axis::Columns, col - 1));
        }
        return None;
    }
    if pos.x < body.left() && pos.y >= body.top() {
        let pane = panes
            .iter()
            .find(|p| p.rect.y_range().contains(pos.y))
            .unwrap_or(&panes[0]);
        let local = f64::from(pos.y - pane.rect.top());
        let row = layout.rows.index_at(pane.scroll.y + local);
        let after = layout.rows.offset(row) + layout.rows.size(row) - pane.scroll.y;
        if (local - after).abs() < grab {
            return Some((Axis::Rows, row));
        }
        let before = layout.rows.offset(row) - pane.scroll.y;
        if row > 0 && (local - before).abs() < grab {
            return Some((Axis::Rows, row - 1));
        }
        return None;
    }
    None
}

/// The row or column a position along a header names.
///
/// Clamped rather than optional: sweeping a band of columns runs the pointer
/// off the end of the strip all the time, and the band should follow it to the
/// last column rather than stop dead the moment the pointer leaves.
fn header_index(layout: &Layout, panes: &[Pane], axis: Axis, pos: egui::Pos2) -> u32 {
    let Some(first) = panes.first() else {
        return 0;
    };
    match axis {
        Axis::Columns => {
            let pane = panes
                .iter()
                .find(|p| p.rect.x_range().contains(pos.x))
                .unwrap_or_else(|| panes.last().unwrap_or(first));
            layout
                .cols
                .index_at(pane.scroll.x + f64::from(pos.x - pane.rect.left()))
        }
        Axis::Rows => {
            let pane = panes
                .iter()
                .find(|p| p.rect.y_range().contains(pos.y))
                .unwrap_or_else(|| panes.last().unwrap_or(first));
            layout
                .rows
                .index_at(pane.scroll.y + f64::from(pos.y - pane.rect.top()))
        }
    }
}

/// The whole-row or whole-column band the selection holds `index` in, if any.
///
/// Only a band of *entire* rows or columns counts. A rectangle that happens to
/// span three columns is a selection of cells, and dragging the header above
/// one of them means "select this column", not "move these three".
fn selected_band(selection: &Selection, axis: Axis, index: u32) -> Option<(u32, u32)> {
    selection.ranges().iter().find_map(|r| {
        let (whole, first, last) = match axis {
            Axis::Columns => (
                r.start.row == 0 && r.end.row == ss_model::cell::MAX_ROWS - 1,
                r.start.col,
                r.end.col,
            ),
            Axis::Rows => (
                r.start.col == 0 && r.end.col == ss_model::cell::MAX_COLS - 1,
                r.start.row,
                r.end.row,
            ),
        };
        (whole && (first..=last).contains(&index)).then_some((first, last))
    })
}

/// The boundary a drop at `pos` names: the index the band would sit before.
///
/// The nearer edge of whatever is under the pointer, because a drop is
/// *between* two rows and the answer has to be able to name the far side of
/// the last one.
fn drop_boundary(layout: &Layout, panes: &[Pane], axis: Axis, pos: egui::Pos2) -> u32 {
    let index = header_index(layout, panes, axis, pos);
    let Some(pane) = panes.first() else {
        return index;
    };
    let pane = match axis {
        Axis::Columns => panes
            .iter()
            .find(|p| p.rect.x_range().contains(pos.x))
            .unwrap_or(pane),
        Axis::Rows => panes
            .iter()
            .find(|p| p.rect.y_range().contains(pos.y))
            .unwrap_or(pane),
    };
    let (along, start, size) = match axis {
        Axis::Columns => (
            pane.scroll.x + f64::from(pos.x - pane.rect.left()),
            layout.cols.offset(index),
            layout.cols.size(index),
        ),
        Axis::Rows => (
            pane.scroll.y + f64::from(pos.y - pane.rect.top()),
            layout.rows.offset(index),
            layout.rows.size(index),
        ),
    };
    if along > start + size / 2.0 {
        index.saturating_add(1)
    } else {
        index
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
        self.ensure_layout(book, sheet);
        self.ensure_conditional(book, sheet);
        self.ensure_summary(book);
        self.textures.ensure(ui.ctx(), &sheet.pictures);
        let cached = self.layout.take().expect("just ensured");
        let layout = &cached.2;
        let conditional = self.conditional.take().expect("just ensured");

        let palette = Palette::of(ui);
        ui.painter().rect_filled(full, 0.0, palette.background);

        // The scrollbars are part of the widget, along its right and bottom
        // edges, so everything else is laid out inside what they leave.
        let content =
            egui::Rect::from_min_max(full.min, full.max - egui::vec2(SCROLLBAR, SCROLLBAR));
        let body = egui::Rect::from_min_max(
            content.min + egui::vec2(layout.header_width as f32, layout.header_height as f32),
            content.max,
        );
        let frozen = sheet.frozen.unwrap_or(CellRef::new(0, 0));
        let split = egui::vec2(
            layout.cols.offset(frozen.col) as f32,
            layout.rows.offset(frozen.row) as f32,
        );

        let viewport = (body.size() - split).max(egui::Vec2::ZERO);
        self.bars = bars(full, layout, sheet, viewport, self.scroll);
        self.clamp_scroll(self.bars.extent);
        self.bars = bars(full, layout, sheet, viewport, self.scroll);
        let panes = split_panes(body, split, self.scroll, layout, frozen);

        let frame = Frame {
            layout,
            palette: &palette,
            full: content,
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
        if sheet.view.headings {
            self.paint_headers(ui, &frame);
        }
        paint_bars(ui, &self.bars, &palette);

        // The frozen split is the one line that is not a gridline. Grey and
        // solid rather than the selection colour: it is a property of the
        // sheet, and it is still there when nothing is selected.
        let seam = egui::Stroke::new(1.0, egui::Color32::from_gray(0x88));
        if frozen.row > 0 {
            ui.painter()
                .hline(content.x_range(), body.top() + split.y, seam);
        }
        if frozen.col > 0 {
            ui.painter()
                .vline(body.left() + split.x, content.y_range(), seam);
        }

        // Where a band being dragged would land. A line between two rows says
        // "between", which a highlighted row would not: dropping *on* a row and
        // dropping *before* it are different answers.
        if let (Some(Drag::MoveBand { axis, .. }), Some(before)) = (self.drag, self.move_target) {
            let drop = egui::Stroke::new(3.0, palette.selection_edge);
            for pane in &panes {
                match axis {
                    Axis::Columns => {
                        let x =
                            pane.rect.left() + (layout.cols.offset(before) - pane.scroll.x) as f32;
                        if pane.rect.x_range().contains(x) {
                            ui.painter().vline(x, content.y_range(), drop);
                        }
                    }
                    Axis::Rows => {
                        let y =
                            pane.rect.top() + (layout.rows.offset(before) - pane.scroll.y) as f32;
                        if pane.rect.y_range().contains(y) {
                            ui.painter().hline(content.x_range(), y, drop);
                        }
                    }
                }
            }
        }

        // Resolved while the layout is still borrowed, painted after it goes
        // back into `self`.
        let editor_rect = self
            .editor
            .as_ref()
            .and_then(|open| cell_rect(layout, &panes, open.at));

        // A chart floats above the cells, so a click on one is not a click on
        // the cell underneath. Resolved here, where the panes are still known.
        if response.clicked() && self.selected_picture.is_none() {
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

        // What the pointer would do if pressed, or — while a drag is in flight
        // — what it is doing. A resize arrow over a handle is most of what
        // tells the user the handles are draggable at all, and a header
        // boundary is a handle nothing else marks out, since the line between
        // two columns is drawn whether or not it can be dragged.
        //
        // A drag names its own icon rather than asking where the pointer is,
        // and it has to: the pointer leaves the boundary the moment it starts
        // dragging it, so hovering would answer "nothing here" and the arrow
        // would drop back to a plain one halfway through the gesture — which
        // reads as the drag having been let go.
        let icon = match self.drag {
            Some(Drag::ResizeColumn { .. }) => Some(egui::CursorIcon::ResizeHorizontal),
            Some(Drag::ResizeRow { .. }) => Some(egui::CursorIcon::ResizeVertical),
            Some(Drag::MoveBand { .. }) => Some(egui::CursorIcon::Grabbing),
            Some(Drag::MovePicture { .. }) => Some(egui::CursorIcon::Move),
            Some(Drag::ResizePicture { handle, .. }) => Some(handle.cursor()),
            Some(Drag::Fill { .. }) => Some(egui::CursorIcon::Crosshair),
            Some(_) => None,
            None => ui.ctx().pointer_hover_pos().and_then(|pos| {
                header_edge(layout, content, body, &panes, pos)
                    .map(|(axis, _)| match axis {
                        Axis::Columns => egui::CursorIcon::ResizeHorizontal,
                        Axis::Rows => egui::CursorIcon::ResizeVertical,
                    })
                    .or_else(|| self.movable_band(layout, content, body, &panes, pos))
                    .or_else(|| self.pointer_over(sheet, layout, &panes, pos))
                    .or_else(|| self.fill_hover(layout, &panes, pos))
            }),
        };
        if let Some(icon) = icon {
            ui.ctx().set_cursor_icon(icon);
        }

        let cursor_rect = cell_rect(layout, &panes, self.selection.cursor());

        // The filter arrows, resolved while the panes are still known. Drawn
        // before the layout goes back into `self` because they need it, and
        // after the cells because they sit on top of the heading text.
        let arrows = filter_arrows(sheet, layout, &panes);

        self.layout = Some(cached);
        self.conditional = Some(conditional);
        self.paint_arrows(ui, &arrows, &response);
        self.paint_editor(ui, editor_rect);
        self.paint_dropdown(ui, book, cursor_rect);
        self.handle_input(ui, book, &response, content, body);
        response
    }

    /// Puts the named thumb's leading edge at a screen position.
    ///
    /// The inverse of [`thumb`]: the thumb travels `track - thumb` pixels while
    /// the sheet travels `extent`, so the ratio between them is the scale.
    fn scroll_to_thumb(&mut self, axis: Axis, at: f32) {
        let (track, thumb, extent) = match axis {
            Axis::Rows => (
                self.bars.vertical,
                self.bars.vertical_thumb,
                self.bars.extent.y,
            ),
            Axis::Columns => (
                self.bars.horizontal,
                self.bars.horizontal_thumb,
                self.bars.extent.x,
            ),
        };
        if extent <= 0.0 {
            return;
        }
        let (start, length, size) = match axis {
            Axis::Rows => (track.top(), track.height(), thumb.height()),
            Axis::Columns => (track.left(), track.width(), thumb.width()),
        };
        let travel = length - size;
        if travel <= 0.0 {
            return;
        }
        let fraction = ((at - start) / travel).clamp(0.0, 1.0);
        let moved = f64::from(fraction * extent);
        match axis {
            Axis::Rows => self.scroll.y = moved,
            Axis::Columns => self.scroll.x = moved,
        }
    }

    /// The thin cross over the fill handle. The square is six pixels wide, so
    /// the cursor changing is most of what says it exists at all.
    fn fill_hover(&self, layout: &Layout, panes: &[Pane], pos: egui::Pos2) -> Option<egui::CursorIcon> {
        if self.editor.is_some() || self.selected_picture.is_some() {
            return None;
        }
        panes
            .iter()
            .any(|pane| pane.rect.contains(pos) && self.fill_handle(layout, pane).contains(pos))
            .then_some(egui::CursorIcon::Crosshair)
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

        // Multiline, but Enter still commits: the editor's own return key is
        // Alt+Enter, which is how Excel breaks a line inside a cell. The
        // child gets room below the cell so added lines grow downward over
        // the grid instead of being clipped at the first row boundary.
        let room = egui::Rect::from_min_max(
            rect.min - egui::vec2(1.0, 1.0),
            egui::pos2(rect.max.x + 1.0, ui.max_rect().bottom()),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(room));
        let output = egui::TextEdit::multiline(&mut open.text)
            .id(id)
            .margin(egui::Margin::symmetric(2, 0))
            .layouter(&mut layouter)
            .desired_width(rect.width().max(60.0))
            .desired_rows(1)
            .lock_focus(false)
            .return_key(Some(egui::KeyboardShortcut::new(
                egui::Modifiers::ALT,
                egui::Key::Enter,
            )))
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

    /// Draws the filter arrows and turns a click on one into an action.
    ///
    /// Drawn rather than made of widgets: a widget per column would take the
    /// click before the grid's own handler saw it, and clicking a heading has
    /// to keep selecting the cell as well as opening the menu when it lands on
    /// the arrow itself.
    fn paint_arrows(&mut self, ui: &mut egui::Ui, arrows: &[Arrow], response: &egui::Response) {
        let painter = ui.painter();
        let hover = ui.ctx().pointer_hover_pos();
        for arrow in arrows {
            let hovered = hover.is_some_and(|p| arrow.rect.contains(p));
            let (fill, ink) = if arrow.filtering {
                (
                    egui::Color32::from_rgb(0x21, 0x73, 0x46),
                    egui::Color32::WHITE,
                )
            } else if hovered {
                (
                    egui::Color32::from_gray(0xC8),
                    egui::Color32::from_gray(0x22),
                )
            } else {
                (
                    egui::Color32::from_gray(0xEE),
                    egui::Color32::from_gray(0x44),
                )
            };
            painter.rect(
                arrow.rect,
                egui::CornerRadius::same(2),
                fill,
                egui::Stroke::new(1.0, egui::Color32::from_gray(0x99)),
                egui::StrokeKind::Inside,
            );
            let c = arrow.rect.center();
            // A funnel when the column is filtering, a plain chevron when it is
            // not — the same distinction Excel draws, and the only thing that
            // says *which* column is hiding the missing rows.
            if arrow.filtering {
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        egui::pos2(c.x - 4.0, c.y - 3.5),
                        egui::pos2(c.x + 4.0, c.y - 3.5),
                        egui::pos2(c.x + 1.0, c.y + 0.5),
                        egui::pos2(c.x + 1.0, c.y + 4.0),
                        egui::pos2(c.x - 1.0, c.y + 2.5),
                        egui::pos2(c.x - 1.0, c.y + 0.5),
                    ],
                    ink,
                    egui::Stroke::NONE,
                ));
            } else {
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        egui::pos2(c.x - 4.0, c.y - 2.0),
                        egui::pos2(c.x + 4.0, c.y - 2.0),
                        egui::pos2(c.x, c.y + 3.0),
                    ],
                    ink,
                    egui::Stroke::NONE,
                ));
            }
        }

        if !response.clicked() {
            return;
        }
        let Some(pos) = ui.ctx().pointer_interact_pos() else {
            return;
        };
        if let Some(arrow) = arrows.iter().find(|a| a.rect.contains(pos)) {
            self.actions.push(Action::FilterMenu(arrow.col));
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

    fn ensure_layout(&mut self, book: &Workbook, sheet: &Sheet) {
        let key = (self.sheet_index, self.generation);
        let stale = match &self.layout {
            Some((index, generation, _)) => (*index, *generation) != key,
            None => true,
        };
        if stale {
            self.layout = Some((key.0, key.1, Layout::for_sheet(book, sheet, self.zoom)));
        }
    }

    /// Keeps the scroll offset inside what there is to look at.
    ///
    /// Against the *used* range rather than against the sheet: a sheet is
    /// twenty million pixels tall and clamping to that lets the wheel take the
    /// user to row 900,000 of an empty grid with no way back but Ctrl+Home.
    fn clamp_scroll(&mut self, extent: egui::Vec2) {
        self.scroll.x = self.scroll.x.clamp(0.0, f64::from(extent.x));
        self.scroll.y = self.scroll.y.clamp(0.0, f64::from(extent.y));
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

        // Gridlines first, so cell text sits on top of them — and only when the
        // sheet asks for them. A sheet laid out as an invoice or a dashboard
        // turns them off and fills the cells instead; drawing them anyway puts
        // a grid through the middle of somebody's letterhead.
        if sheet.view.gridlines {
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

        // Selection under the text — unless a picture owns the selection, in
        // which case there is nothing selected in the cells at all and saying
        // otherwise would leave the user guessing what Delete is about to do.
        //
        // The active cell is left unfilled inside the highlight, as Excel
        // leaves it white: it is the one cell typing would land in, and the
        // gap in the wash is how the eye finds it in a screenful of blue.
        let unfilled = {
            let cursor = self.selection.cursor();
            let anchor = sheet.merge_at(cursor).map_or(cursor, |m| m.start);
            match sheet.merge_at(anchor) {
                Some(m) => super::rect_of_range(layout, *m, pane.rect, pane.scroll),
                None => rect_of(layout, anchor, pane.rect, pane.scroll),
            }
        };
        for range in self
            .selection
            .ranges()
            .iter()
            .filter(|_| self.selected_picture.is_none())
        {
            let rect = super::rect_of_range(layout, *range, pane.rect, pane.scroll);
            let visible = rect.intersect(pane.rect);
            if !visible.is_positive() {
                continue;
            }
            fill_except(&painter, visible, unfilled, palette.selection_fill);
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
        } else if self.editor.is_none() && self.selected_picture.is_none() {
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
            let font = cell_font(&cell.look, self.zoom as f32);
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
                paint_text(&painter, pos, galley, color);
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
                // "Centre across selection": centred over the cell *and the
                // empty ones it runs into*, which is the whole point of it. In
                // a narrow column, centring inside the cell alone puts most of
                // a long label to the left of its own left edge, where it is
                // clipped away and the cell reads as blank.
                HAlign::CenterContinuous => {
                    (cell.rect.left() + cell.rect.right() + cell.overflow) / 2.0
                        - galley.size().x / 2.0
                }
                HAlign::Center | HAlign::Distributed => {
                    cell.rect.center().x - galley.size().x / 2.0
                }
                _ => cell.rect.left() + padding,
            };
            let y = match cell.look.vertical {
                VAlign::Top => cell.rect.top() + 1.0,
                VAlign::Bottom => cell.rect.bottom() - 1.0 - galley.size().y,
                _ => cell.rect.center().y - galley.size().y / 2.0,
            };
            paint_text(&painter, egui::pos2(x, y), galley, color);
        }

        // Pictures over the cells and under the charts, which is the order
        // Excel draws them in when a chart is dropped on top of a logo. A
        // masthead is not decoration to be skipped: on a form sheet it is the
        // heading, and leaving it out shows a document nobody wrote.
        for picture in &sheet.pictures {
            let rect = super::chart::rect_of(layout, &picture.anchor, pane.rect, pane.scroll);
            if !rect.intersect(pane.rect).is_positive() {
                continue;
            }
            super::picture::draw(&painter, rect, picture, &self.textures, palette.grid);
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

        // A selected picture wears Excel's chrome: an outline and eight
        // handles. Drawn after the charts so nothing covers it.
        if let Some(picture) = self.selected_picture.and_then(|i| sheet.pictures.get(i)) {
            let rect = super::chart::rect_of(layout, &picture.anchor, pane.rect, pane.scroll);
            if rect.intersect(pane.rect).is_positive() {
                super::picture::draw_selection(&painter, rect);
            }
        }

        // The cell cursor is not drawn while a picture is selected: two
        // selections on screen at once would leave the user guessing which one
        // Delete is about to act on.
        if self.selected_picture.is_some() {
            return;
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

    /// The row numbers and column letters.
    ///
    /// Three states, not two, and the third is the one that matters: a header
    /// is plain, *within the selection*, or the cursor's own. Excel draws the
    /// cursor's row and column darker than the rest of the selected band, which
    /// is how you find where you are in a screen of forty selected rows without
    /// hunting for the outline.
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
        let size = (11.0 * self.zoom).clamp(7.0, 18.0) as f32;
        let plain = egui::FontId::new(
            size,
            ui_kit::fonts::face(ui_kit::Family::Sans, false, false),
        );
        let heavy = egui::FontId::new(size, ui_kit::fonts::face(ui_kit::Family::Sans, true, false));

        let corner = egui::Rect::from_min_max(full.min, body.min);
        painter.rect_filled(corner, 0.0, palette.header_bg);
        // The little triangle in the corner box, which is what says it is a
        // button rather than a gap between the two headers.
        let side = (corner.width().min(corner.height()) * 0.42).min(9.0);
        if side > 3.0 {
            let tip = corner.right_bottom() - egui::vec2(3.0, 3.0);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    tip,
                    tip - egui::vec2(side, 0.0),
                    tip - egui::vec2(0.0, side),
                ],
                palette.header_text.gamma_multiply(0.5),
                egui::Stroke::NONE,
            ));
        }

        // Empty while a picture is selected: the headers are how you find
        // where the cell cursor is, and it is not anywhere just now.
        let ranges = match self.selected_picture {
            Some(_) => Vec::new(),
            None => self.selection.ranges().to_vec(),
        };
        let cursor = self.selection.cursor();
        let show_cursor = self.selected_picture.is_none();
        let line = egui::Stroke::new(1.0, palette.header_line);

        for pane in panes {
            // Column letters, above each pane that shows columns.
            let strip = egui::Rect::from_min_max(
                egui::pos2(pane.rect.left(), full.top()),
                egui::pos2(pane.rect.right(), body.top()),
            );
            if strip.is_positive() && pane.rect.top() <= body.top() + 0.5 {
                let p = painter.with_clip_rect(strip);
                p.rect_filled(strip, 0.0, palette.header_bg);
                for col in layout
                    .cols
                    .visible(pane.scroll.x, pane.scroll.x + f64::from(pane.rect.width()))
                {
                    let x = pane.rect.left() + (layout.cols.offset(col) - pane.scroll.x) as f32;
                    let cell = egui::Rect::from_min_size(
                        egui::pos2(x, strip.top()),
                        egui::vec2(layout.cols.size(col) as f32, strip.height()),
                    );
                    let selected = ranges
                        .iter()
                        .any(|r| r.start.col <= col && col <= r.end.col);
                    self.header_cell(
                        &p,
                        cell,
                        &column_name(col),
                        selected,
                        show_cursor && col == cursor.col,
                        palette,
                        (&plain, &heavy),
                        Axis::Columns,
                        line,
                    );
                }
            }

            // Row numbers, left of each pane that shows rows.
            let gutter = egui::Rect::from_min_max(
                egui::pos2(full.left(), pane.rect.top()),
                egui::pos2(body.left(), pane.rect.bottom()),
            );
            if gutter.is_positive() && pane.rect.left() <= body.left() + 0.5 {
                let p = painter.with_clip_rect(gutter);
                p.rect_filled(gutter, 0.0, palette.header_bg);
                for row in layout
                    .rows
                    .visible(pane.scroll.y, pane.scroll.y + f64::from(pane.rect.height()))
                {
                    let y = pane.rect.top() + (layout.rows.offset(row) - pane.scroll.y) as f32;
                    let cell = egui::Rect::from_min_size(
                        egui::pos2(gutter.left(), y),
                        egui::vec2(gutter.width(), layout.rows.size(row) as f32),
                    );
                    let selected = ranges
                        .iter()
                        .any(|r| r.start.row <= row && row <= r.end.row);
                    self.header_cell(
                        &p,
                        cell,
                        &format!("{}", row + 1),
                        selected,
                        show_cursor && row == cursor.row,
                        palette,
                        (&plain, &heavy),
                        Axis::Rows,
                        line,
                    );
                }
            }
        }
        painter.rect_stroke(corner, 0.0, line, egui::StrokeKind::Inside);
    }

    /// One row number or column letter.
    #[allow(clippy::too_many_arguments)]
    fn header_cell(
        &self,
        painter: &egui::Painter,
        cell: egui::Rect,
        label: &str,
        selected: bool,
        is_cursor: bool,
        palette: &Palette,
        fonts: (&egui::FontId, &egui::FontId),
        axis: Axis,
        line: egui::Stroke,
    ) {
        if selected {
            painter.rect_filled(cell, 0.0, palette.header_active);
        }
        // The separator between headers, and the edge against the grid.
        match axis {
            Axis::Columns => {
                painter.vline(cell.right().round(), cell.y_range(), line);
            }
            Axis::Rows => {
                painter.hline(cell.x_range(), cell.bottom().round(), line);
            }
        }
        // A zero-height row is a hidden one: it has no room for its number, and
        // drawing it anyway stacks digits on the row below.
        if cell.height() < 6.0 || cell.width() < 6.0 {
            return;
        }
        let (font, color) = if is_cursor {
            (fonts.1, palette.header_active_text)
        } else if selected {
            (fonts.0, palette.header_active_text)
        } else {
            (fonts.0, palette.header_text)
        };
        painter.text(
            cell.center(),
            egui::Align2::CENTER_CENTER,
            label,
            font.clone(),
            color,
        );
        // The heavier line under the cursor's own header, which is the marker
        // Excel uses to point at the active cell from outside the grid.
        if is_cursor {
            let mark = egui::Stroke::new(2.0, palette.selection_edge);
            match axis {
                Axis::Columns => {
                    painter.hline(cell.x_range(), cell.bottom() - 1.0, mark);
                }
                Axis::Rows => {
                    painter.vline(cell.right() - 1.0, cell.y_range(), mark);
                }
            }
        }
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

        // Double-clicking a boundary fits what is on the other side of it,
        // which is the gesture everybody reaches for before they find the
        // menu. It fits that one row or column and not the selection: the
        // pointer named it, and nothing else was asked about.
        //
        // Resolved *before* the drag below, and it has to be: the first press
        // of the pair started a resize, a drag owns the pointer until it ends,
        // and the second click would never be looked at.
        if response.double_clicked() {
            let pos = ui.ctx().pointer_interact_pos();
            let edge = pos.and_then(|pos| header_edge(layout, full, body, &panes, pos));
            if let Some((axis, index)) = edge {
                // The drag it started never moved anything, so it is dropped
                // rather than reported as a resize of nothing.
                self.drag = None;
                self.before_resize = None;
                self.actions.push(Action::AutoFitAt { axis, index });
                self.layout = Some(cached);
                return;
            }
            // And on a cell, the same gesture opens it for editing — for the
            // same reason it has to be resolved up here, and only when the
            // drag underway is the plain selection sweep the first click
            // started. A picture being moved or a thumb being dragged is
            // somebody else's gesture and keeps the pointer.
            if matches!(self.drag, None | Some(Drag::Select))
                && pos.is_some_and(|pos| body.contains(pos))
            {
                self.drag = None;
                self.open_editor(book, Mode::Edit);
                self.layout = Some(cached);
                return;
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
                (true, Some(pos)) => {
                    self.continue_drag(drag, book, &frame, pos);
                    // Held past the edge with the mouse still: no new events
                    // arrive, so the next frame has to be asked for or the
                    // auto-scroll stalls until the pointer moves again.
                    if !body.contains(pos)
                        && matches!(
                            drag,
                            Drag::Select
                                | Drag::SelectHeaders { .. }
                                | Drag::Fill { .. }
                                | Drag::Point
                        )
                    {
                        ui.ctx().request_repaint();
                    }
                }
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
            let (primary, secondary) = ui.input(|i| {
                (
                    i.pointer.button_pressed(egui::PointerButton::Primary),
                    i.pointer.button_pressed(egui::PointerButton::Secondary),
                )
            });
            if primary && !inside_editor {
                // Mid-formula, a click is Excel's point mode: the cell's name
                // goes into the formula, and the click never reaches the grid.
                let points = self
                    .editor
                    .as_ref()
                    .is_some_and(|open| open.is_formula() && open.can_point())
                    && body.contains(pos);
                if points {
                    if let Some(at) = self.cell_at(layout, body, &panes, pos) {
                        if let Some(open) = &mut self.editor {
                            open.point_at(at);
                        }
                        self.drag = Some(Drag::Point);
                        self.layout = Some(cached);
                        return;
                    }
                }
                // Clicking away from an open editor keeps what was typed, the
                // way every spreadsheet does.
                self.commit(None);
                self.begin_drag(book, &frame, pos, modifiers);
            } else if secondary && !inside_editor {
                // A right-click is a request for the menu, not a selection
                // gesture: inside the selection it changes nothing, so the
                // menu can act on what is selected; outside it, it moves
                // there first, as Excel's does. It never starts a drag.
                self.commit(None);
                self.secondary_press(book, &frame, pos);
            }
        }
        self.handle_keys(ui, book, body, layout);
        self.layout = Some(cached);
    }

    /// What a right-button press does to the selection before the context
    /// menu opens over it.
    fn secondary_press(&mut self, book: &Workbook, frame: &Frame, pos: egui::Pos2) {
        let Some(sheet) = book.sheet(self.sheet_index) else {
            return;
        };
        let Frame {
            layout, body, panes, ..
        } = *frame;
        let in_column_header = pos.y < body.top() && pos.x >= body.left();
        let in_row_header = pos.x < body.left() && pos.y >= body.top();
        if in_column_header || in_row_header {
            let axis = if in_column_header {
                Axis::Columns
            } else {
                Axis::Rows
            };
            let index = header_index(layout, panes, axis, pos);
            if selected_band(&self.selection, axis, index).is_none() {
                match axis {
                    Axis::Columns => self.selection.select_columns(index, index, false),
                    Axis::Rows => self.selection.select_rows(index, index, false),
                }
            }
            return;
        }
        if body.contains(pos) {
            if let Some(at) = self.cell_at(layout, body, panes, pos) {
                if !self.selection.contains(at) {
                    self.selection.move_to(at, sheet);
                }
            }
        }
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
        // The scrollbars are outside `full`, so they are tested before
        // anything that assumes the pointer is over a cell.
        for (axis, track, thumb) in [
            (Axis::Rows, self.bars.vertical, self.bars.vertical_thumb),
            (
                Axis::Columns,
                self.bars.horizontal,
                self.bars.horizontal_thumb,
            ),
        ] {
            if !track.contains(pos) {
                continue;
            }
            let (along, thumb_start, thumb_size) = match axis {
                Axis::Rows => (pos.y, thumb.top(), thumb.height()),
                Axis::Columns => (pos.x, thumb.left(), thumb.width()),
            };
            // A click on the track jumps the thumb to the pointer and then
            // drags from its middle, which is what every scrollbar does.
            let grab = if thumb.contains(pos) {
                along - thumb_start
            } else {
                thumb_size / 2.0
            };
            self.drag = Some(Drag::ScrollThumb { axis, grab });
            self.scroll_to_thumb(axis, along - grab);
            return;
        }

        // A picture floats above the cells, so a press on one is not a press on
        // the cell underneath — and it is the start of a move, not of a
        // selection sweep. Anything else deselects it, including a press on a
        // header, which is why this is tested before them.
        if self.begin_picture_drag(sheet, layout, panes, pos) {
            return;
        }

        let in_column_header = pos.y < body.top() && pos.x >= body.left();
        let in_row_header = pos.x < body.left() && pos.y >= body.top();

        // Within a few pixels of a boundary the drag resizes instead of
        // selecting, on whichever header the pointer is over.
        match header_edge(layout, full, body, panes, pos) {
            Some((Axis::Columns, index)) => {
                self.before_resize = Some(Geometry::of(sheet));
                self.drag = Some(Drag::ResizeColumn {
                    index,
                    origin: pos.x,
                    start: layout.cols.size(index) as f32,
                });
                return;
            }
            Some((Axis::Rows, index)) => {
                self.before_resize = Some(Geometry::of(sheet));
                self.drag = Some(Drag::ResizeRow {
                    index,
                    origin: pos.y,
                    start: layout.rows.size(index) as f32,
                });
                return;
            }
            None => {}
        }

        for (in_header, axis) in [
            (in_column_header, Axis::Columns),
            (in_row_header, Axis::Rows),
        ] {
            if !in_header {
                continue;
            }
            let index = header_index(layout, panes, axis, pos);
            // A press inside a band that is already selected picks it up. A
            // press anywhere else on the header starts a new selection, which
            // is what makes the two gestures tell themselves apart without a
            // modifier: you cannot move what you have not selected.
            if !modifiers.ctrl && !modifiers.shift {
                if let Some((first, last)) = selected_band(&self.selection, axis, index) {
                    self.drag = Some(Drag::MoveBand { axis, first, last });
                    return;
                }
            }
            // Shift extends the band already there, as it does everywhere else.
            let anchor = match (modifiers.shift, axis) {
                (true, Axis::Columns) => self.selection.anchor().col,
                (true, Axis::Rows) => self.selection.anchor().row,
                (false, _) => index,
            };
            let additive = modifiers.ctrl && !modifiers.shift;
            match axis {
                Axis::Columns => self.selection.select_columns(anchor, index, additive),
                Axis::Rows => self.selection.select_rows(anchor, index, additive),
            }
            self.drag = Some(Drag::SelectHeaders { axis, anchor });
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

    /// An open hand over the header of a band that could be dragged elsewhere.
    ///
    /// The only thing that says the gesture exists. A header looks exactly the
    /// same whether or not the column under it is selected, so without this
    /// nothing distinguishes "press here to move it" from "press here to
    /// select it" until after the press.
    fn movable_band(
        &self,
        layout: &Layout,
        content: egui::Rect,
        body: egui::Rect,
        panes: &[Pane],
        pos: egui::Pos2,
    ) -> Option<egui::CursorIcon> {
        if !content.contains(pos) {
            return None;
        }
        let axis = if pos.y < body.top() && pos.x >= body.left() {
            Axis::Columns
        } else if pos.x < body.left() && pos.y >= body.top() {
            Axis::Rows
        } else {
            return None;
        };
        let index = header_index(layout, panes, axis, pos);
        selected_band(&self.selection, axis, index).map(|_| egui::CursorIcon::Grab)
    }

    /// The cursor a picture would put under the pointer, if any.
    fn pointer_over(
        &self,
        sheet: &Sheet,
        layout: &Layout,
        panes: &[Pane],
        pos: egui::Pos2,
    ) -> Option<egui::CursorIcon> {
        let pane = panes.iter().find(|p| p.rect.contains(pos))?;
        if let Some(picture) = self.selected_picture.and_then(|i| sheet.pictures.get(i)) {
            let rect = super::chart::rect_of(layout, &picture.anchor, pane.rect, pane.scroll);
            if let Some(handle) = super::picture::handle_at(rect, pos) {
                return Some(handle.cursor());
            }
        }
        sheet
            .pictures
            .iter()
            .any(|p| super::chart::rect_of(layout, &p.anchor, pane.rect, pane.scroll).contains(pos))
            .then_some(egui::CursorIcon::Move)
    }

    /// Selects, moves or resizes a picture. True when the press was claimed.
    fn begin_picture_drag(
        &mut self,
        sheet: &Sheet,
        layout: &Layout,
        panes: &[Pane],
        pos: egui::Pos2,
    ) -> bool {
        // Handles first, and only on the selected picture: they sit *outside*
        // its edges, so a corner handle overlaps whatever is behind it.
        if let Some((index, picture)) = self
            .selected_picture
            .and_then(|i| sheet.pictures.get(i).map(|p| (i, p)))
        {
            for pane in panes {
                if !pane.rect.contains(pos) {
                    continue;
                }
                let rect = super::chart::rect_of(layout, &picture.anchor, pane.rect, pane.scroll);
                if let Some(handle) = super::picture::handle_at(rect, pos) {
                    self.before_pictures = Some(sheet.pictures.clone());
                    self.drag = Some(Drag::ResizePicture {
                        index,
                        handle,
                        start: super::picture::sheet_rect(layout, &picture.anchor),
                    });
                    return true;
                }
            }
        }

        // Then the body of the topmost picture under the pointer. Last in the
        // list is topmost, because that is the order they are drawn in.
        for pane in panes {
            if !pane.rect.contains(pos) {
                continue;
            }
            let hit = sheet.pictures.iter().enumerate().rev().find(|(_, p)| {
                super::chart::rect_of(layout, &p.anchor, pane.rect, pane.scroll).contains(pos)
            });
            if let Some((index, picture)) = hit {
                self.selected_picture = Some(index);
                self.selected_chart = None;
                let rect = super::picture::sheet_rect(layout, &picture.anchor);
                self.before_pictures = Some(sheet.pictures.clone());
                self.drag = Some(Drag::MovePicture {
                    index,
                    grab: sheet_space(pane, pos) - rect.min,
                });
                return true;
            }
        }

        self.selected_picture = None;
        false
    }

    /// Writes a picture's new rectangle back as an anchor of its own kind.
    fn set_picture_rect(
        &mut self,
        book: &mut Workbook,
        layout: &Layout,
        index: usize,
        rect: egui::Rect,
    ) {
        let Some(sheet) = book.sheet_mut(self.sheet_index) else {
            return;
        };
        let Some(current) = sheet.pictures.get(index).map(|p| p.anchor.clone()) else {
            return;
        };
        sheet.pictures[index].anchor = super::picture::anchor_of(layout, rect, &current);
    }

    /// Nudges the view when a drag holds the pointer past the edge of the
    /// body, so a sweep can keep selecting beyond what is on screen. The step
    /// grows with the overshoot, the way Excel accelerates the further out
    /// the pointer is held.
    fn auto_scroll(&mut self, body: egui::Rect, pos: egui::Pos2, horizontal: bool, vertical: bool) {
        let step = |over: f32| f64::from((over * 0.3).clamp(2.0, 48.0));
        if horizontal {
            if pos.x > body.right() {
                self.scroll.x += step(pos.x - body.right());
            } else if pos.x < body.left() {
                self.scroll.x -= step(body.left() - pos.x);
            }
        }
        if vertical {
            if pos.y > body.bottom() {
                self.scroll.y += step(pos.y - body.bottom());
            } else if pos.y < body.top() {
                self.scroll.y -= step(body.top() - pos.y);
            }
        }
        self.scroll = self.scroll.clamped();
    }

    fn continue_drag(&mut self, drag: Drag, book: &mut Workbook, frame: &Frame, pos: egui::Pos2) {
        let (layout, body, panes) = (frame.layout, frame.body, frame.panes);
        match drag {
            Drag::ScrollThumb { axis, grab } => {
                let along = match axis {
                    Axis::Rows => pos.y,
                    Axis::Columns => pos.x,
                };
                self.scroll_to_thumb(axis, along - grab);
            }
            Drag::MovePicture { index, grab } => {
                let Some(pane) = pane_at(panes, pos) else {
                    return;
                };
                let Some(size) = book
                    .sheet(self.sheet_index)
                    .and_then(|s| s.pictures.get(index))
                    .map(|p| super::picture::sheet_rect(layout, &p.anchor).size())
                else {
                    return;
                };
                // Clamped at the top-left corner: an anchor cannot name a cell
                // before A1, so a picture dragged off that edge would otherwise
                // come back somewhere it was never put.
                let at = sheet_space(pane, pos) - grab;
                let rect =
                    egui::Rect::from_min_size(egui::pos2(at.x.max(0.0), at.y.max(0.0)), size);
                self.set_picture_rect(book, layout, index, rect);
            }
            Drag::ResizePicture {
                index,
                handle,
                start,
            } => {
                let Some(pane) = pane_at(panes, pos) else {
                    return;
                };
                let to = sheet_space(pane, pos);
                let rect = handle.resize(start, egui::pos2(to.x.max(0.0), to.y.max(0.0)));
                self.set_picture_rect(book, layout, index, rect);
            }
            Drag::Fill { from } => {
                self.auto_scroll(body, pos, true, true);
                let Some(at) = self.cell_at(layout, body, panes, pos) else {
                    return;
                };
                self.fill_target = Some(fill_target(from, at));
            }
            Drag::Point => {
                self.auto_scroll(body, pos, true, true);
                let Some(at) = self.cell_at(layout, body, panes, pos) else {
                    return;
                };
                if let Some(open) = &mut self.editor {
                    open.point_to(at);
                }
            }
            Drag::Select => {
                self.auto_scroll(body, pos, true, true);
                let Some(at) = self.cell_at(layout, body, panes, pos) else {
                    return;
                };
                if let Some(sheet) = book.sheet(self.sheet_index) {
                    self.selection.extend_to(at, sheet);
                }
            }
            Drag::SelectHeaders { axis, anchor } => {
                // Only along the axis being swept: a column sweep has the
                // pointer above the body the whole time, and scrolling the
                // rows away because of it would be nonsense.
                match axis {
                    Axis::Columns => self.auto_scroll(body, pos, true, false),
                    Axis::Rows => self.auto_scroll(body, pos, false, true),
                }
                let index = header_index(layout, panes, axis, pos);
                self.selection.extend_headers(axis, anchor, index);
            }
            Drag::MoveBand { axis, first, last } => {
                let before = drop_boundary(layout, panes, axis, pos);
                // A drop anywhere inside the band, or against either of its own
                // edges, is a drop that changes nothing. Showing no line there
                // says so before the button comes up.
                self.move_target =
                    (!(first..=last.saturating_add(1)).contains(&before)).then_some(before);
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
            Drag::MoveBand { axis, first, last } => {
                if let Some(before) = self.move_target.take() {
                    self.actions.push(Action::MoveBand {
                        axis,
                        first,
                        last,
                        before,
                    });
                }
            }
            // The model already holds the new geometry. What goes out is how it
            // looked before, and the application drops it if nothing moved —
            // clicking a picture to select it must not fill the undo stack.
            Drag::MovePicture { .. } | Drag::ResizePicture { .. } => {
                if let Some(before) = self.before_pictures.take() {
                    self.actions.push(Action::PicturesMoved(before));
                }
            }
            // Scrolling has already happened frame by frame, and it is not an
            // edit, so there is nothing to report and nothing to undo. A
            // pointed reference is already in the formula's text.
            Drag::Select | Drag::SelectHeaders { .. } | Drag::ScrollThumb { .. } | Drag::Point => {}
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
        // The cell the view should chase, when a key moved one: the cursor
        // for plain movement, the growing corner for an extension.
        let mut follow: Option<CellRef> = None;

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
                    physical_key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if self.editor.is_some() {
                        follow = self.editing_key(key, physical_key, modifiers).or(follow);
                    } else {
                        follow = self
                            .grid_key(key, physical_key, modifiers, sheet, book, body, layout)
                            .or(follow);
                    }
                }
                _ => {}
            }
        }

        if let Some(target) = follow {
            self.scroll_into_view(target, body.size(), book, sheet);
        }
    }

    /// Keys while the editor is open.
    ///
    /// The mode is the whole point: an arrow key commits and moves when the
    /// editor was opened by typing, and moves the caret when it was opened with
    /// F2. Getting this backwards makes a half-typed formula unusable in one
    /// mode and cell navigation unusable in the other.
    fn editing_key(
        &mut self,
        key: egui::Key,
        physical: Option<egui::Key>,
        modifiers: egui::Modifiers,
    ) -> Option<CellRef> {
        // Shift changes what the platform calls the key — Ctrl+Shift+; can
        // arrive as `:` — so keys that pair with shift are matched physically.
        let hit = |want: egui::Key| key == want || physical == Some(want);
        if modifiers.ctrl && hit(egui::Key::Semicolon) {
            let stamp = if modifiers.shift {
                crate::clock::time_text()
            } else {
                crate::clock::date_text()
            };
            if let Some(open) = &mut self.editor {
                open.text.push_str(&stamp);
            }
            return None;
        }
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
                // Mid-formula the arrows point at cells, as Excel's do; only
                // when the text cannot take a reference do they commit.
                let points = self
                    .editor
                    .as_ref()
                    .is_some_and(|e| e.is_formula() && e.can_point());
                if points {
                    return self.point_step(direction, modifiers.shift);
                }
                self.commit(Some(direction));
            }
            return None;
        }
        match key {
            // Alt+Enter is the editor's own return key: it inserts the
            // newline itself, and the commit below must not fire.
            egui::Key::Enter if modifiers.alt => {}
            // Ctrl+Enter writes the entry into every selected cell at once.
            egui::Key::Enter if modifiers.ctrl => {
                if let Some(open) = self.editor.take() {
                    self.actions.push(Action::CommitAll {
                        at: open.at,
                        text: open.text,
                    });
                }
            }
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
            // F4 cycles the dollars on the reference just written or typed.
            egui::Key::F4 => {
                if let Some(open) = &mut self.editor {
                    open.cycle_reference();
                }
            }
            _ => {}
        }
        None
    }

    /// An arrow in point mode: moves the pointed reference one step, or
    /// starts one beside the cell being edited. Returns the pointed cell so
    /// the view can follow it.
    fn point_step(&mut self, direction: Direction, extend: bool) -> Option<CellRef> {
        let open = self.editor.as_mut()?;
        let from = match open.pointed() {
            Some((_, lead)) => lead,
            None => open.at,
        };
        let (dr, dc) = match direction {
            Direction::Up => (-1i64, 0i64),
            Direction::Down => (1, 0),
            Direction::Left => (0, -1),
            Direction::Right => (0, 1),
        };
        let to = CellRef::new(
            (i64::from(from.row) + dr).clamp(0, i64::from(ss_model::cell::MAX_ROWS) - 1) as u32,
            (i64::from(from.col) + dc).clamp(0, i64::from(ss_model::cell::MAX_COLS) - 1) as u32,
        );
        if extend && open.pointed().is_some() {
            open.point_to(to);
        } else {
            open.point_at(to);
        }
        Some(to)
    }

    /// Keys while the grid itself has the keyboard. Returns the cell the view
    /// should scroll to when the key moved one, or None when nothing moved.
    #[allow(clippy::too_many_arguments)]
    fn grid_key(
        &mut self,
        key: egui::Key,
        physical: Option<egui::Key>,
        modifiers: egui::Modifiers,
        sheet: &Sheet,
        book: &Workbook,
        body: egui::Rect,
        layout: &Layout,
    ) -> Option<CellRef> {
        // A selected picture takes the two keys that are about *it* rather than
        // about the cells. Everything else — arrows, typing — deselects it,
        // because the cell cursor is what those keys move.
        if let Some(index) = self.selected_picture {
            match key {
                egui::Key::Delete | egui::Key::Backspace => {
                    self.actions.push(Action::DeletePicture(index));
                    self.selected_picture = None;
                    return None;
                }
                egui::Key::Escape => {
                    self.selected_picture = None;
                    return None;
                }
                _ => self.selected_picture = None,
            }
        }

        // Keys whose logical name changes under shift — Ctrl+Shift+5 arrives
        // as `%` on some platforms — are matched against the physical key too.
        let hit = |want: egui::Key| key == want || physical == Some(want);
        if modifiers.ctrl && !modifiers.alt {
            // Ctrl+9 and Ctrl+0 hide the selected rows and columns; with
            // shift they unhide, as in Excel.
            if hit(egui::Key::Num9) {
                self.actions.push(Action::Visibility {
                    axis: Axis::Rows,
                    hide: !modifiers.shift,
                });
                return None;
            }
            if hit(egui::Key::Num0) {
                self.actions.push(Action::Visibility {
                    axis: Axis::Columns,
                    hide: !modifiers.shift,
                });
                return None;
            }
            // Ctrl+; types today's date and Ctrl+Shift+; the time — into a
            // fresh editor, exactly as if the user had typed the digits.
            if hit(egui::Key::Semicolon) {
                let stamp = if modifiers.shift {
                    crate::clock::time_text()
                } else {
                    crate::clock::date_text()
                };
                self.editor = Some(Editor::typing(self.selection.cursor(), stamp));
                return None;
            }
            // Ctrl+Shift+~ ! @ # $ % ^: Excel's number-format row, using the
            // same codes the toolbar combo names, so the combo agrees.
            if modifiers.shift {
                let code = if hit(egui::Key::Backtick) {
                    Some("General")
                } else if hit(egui::Key::Num1) {
                    Some("#,##0.00")
                } else if hit(egui::Key::Num2) {
                    Some("h:mm:ss AM/PM")
                } else if hit(egui::Key::Num3) {
                    Some("d-mmm-yy")
                } else if hit(egui::Key::Num4) {
                    Some("\"$\"#,##0.00")
                } else if hit(egui::Key::Num5) {
                    Some("0%")
                } else if hit(egui::Key::Num6) {
                    Some("0.00E+00")
                } else {
                    None
                };
                if let Some(code) = code {
                    self.actions
                        .push(Action::Format(Format::NumberFormat(code.into())));
                    return None;
                }
            }
        }

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
            // An extension chases its growing corner; plain movement chases
            // the cursor. The two only differ while shift is held.
            return Some(if modifiers.shift {
                self.selection.lead()
            } else {
                self.selection.cursor()
            });
        }

        match key {
            egui::Key::Tab => {
                let dir = if modifiers.shift {
                    Direction::Left
                } else {
                    Direction::Right
                };
                self.selection.advance(dir, sheet);
                return Some(self.selection.cursor());
            }
            egui::Key::Enter => {
                let dir = if modifiers.shift {
                    Direction::Up
                } else {
                    Direction::Down
                };
                self.selection.advance(dir, sheet);
                return Some(self.selection.cursor());
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
            egui::Key::A if modifiers.ctrl => {
                // First press selects the island of data around the cursor;
                // pressed again — or on an empty patch — the whole sheet.
                let region =
                    super::selection::current_region(sheet, self.selection.cursor());
                let lone = region.rows() == 1 && region.cols() == 1;
                if lone || self.selection.ranges() == [region] {
                    self.selection.select_all();
                } else {
                    self.selection.select_range(region);
                }
            }
            egui::Key::B if modifiers.ctrl => self.actions.push(Action::Format(Format::Bold)),
            egui::Key::I if modifiers.ctrl => self.actions.push(Action::Format(Format::Italic)),
            egui::Key::U if modifiers.ctrl => self.actions.push(Action::Format(Format::Underline)),
            egui::Key::Num5 if modifiers.ctrl => self.actions.push(Action::Format(Format::Strike)),
            // Ctrl+Shift+Space is select-all, as in Excel — not an additive
            // column selection, which is what it used to do here.
            egui::Key::Space if modifiers.ctrl && modifiers.shift => {
                self.selection.select_all();
            }
            egui::Key::Space if modifiers.ctrl => {
                let range = self.selection.active_range();
                self.selection
                    .select_columns(range.start.col, range.end.col, false);
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
            egui::Key::PageDown | egui::Key::PageUp if modifiers.ctrl => {
                let step = if key == egui::Key::PageDown { 1 } else { -1 };
                self.actions.push(Action::StepSheet(step));
            }
            egui::Key::Home => {
                let at = if modifiers.ctrl {
                    CellRef::new(0, 0)
                } else {
                    let row = if modifiers.shift {
                        self.selection.lead().row
                    } else {
                        self.selection.cursor().row
                    };
                    CellRef::new(row, 0)
                };
                if modifiers.shift {
                    self.selection.extend_to(at, sheet);
                    return Some(self.selection.lead());
                }
                self.selection.move_to(at, sheet);
                return Some(self.selection.cursor());
            }
            // End mirrors Home: the last filled cell of the row, and with
            // Ctrl the bottom-right corner of everything the sheet uses.
            egui::Key::End => {
                if modifiers.ctrl {
                    let target = sheet
                        .cells
                        .used_range()
                        .map_or(CellRef::new(0, 0), |(_, end)| end);
                    if modifiers.shift {
                        self.selection.extend_to(target, sheet);
                        return Some(self.selection.lead());
                    }
                    self.selection.move_to(target, sheet);
                    return Some(self.selection.cursor());
                }
                self.selection.end_of_row(sheet, modifiers.shift);
                return Some(if modifiers.shift {
                    self.selection.lead()
                } else {
                    self.selection.cursor()
                });
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
                    return Some(self.selection.lead());
                }
                self.selection.move_to(at, sheet);
                return Some(self.selection.cursor());
            }
            _ => {}
        }
        None
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

/// Where the scrollbars go, and how far they may travel.
///
/// The travel is measured against the *used* range and not against the sheet,
/// because a sheet is 1,048,576 rows tall and a thumb sized to that is a
/// two-pixel sliver representing a document that ends on row 354. Excel does
/// the same thing: its scrollbar covers what you have, and grows as you add.
fn bars(
    full: egui::Rect,
    layout: &Layout,
    sheet: &Sheet,
    viewport: egui::Vec2,
    scroll: Scroll,
) -> Bars {
    let (_, last) = sheet
        .cells
        .used_range()
        .unwrap_or((CellRef::new(0, 0), CellRef::new(0, 0)));
    // A styled-but-empty column counts as content: it is visible, so it must
    // be reachable.
    let last_col = last
        .col
        .max(sheet.column_styles.keys().copied().max().unwrap_or(0));
    let last_row = last
        .row
        .max(sheet.row_styles.keys().copied().max().unwrap_or(0));

    let content = egui::vec2(
        layout.cols.offset(last_col.saturating_add(1)) as f32,
        layout.rows.offset(last_row.saturating_add(1)) as f32,
    );
    // `max` against the current scroll so that a cursor driven past the used
    // range with Ctrl+Down does not leave the scrollbar unable to follow it.
    let extent = egui::vec2(
        (content.x - viewport.x).max(scroll.x as f32).max(0.0),
        (content.y - viewport.y).max(scroll.y as f32).max(0.0),
    );

    let vertical = egui::Rect::from_min_max(
        egui::pos2(
            full.right() - SCROLLBAR,
            full.top() + layout.header_height as f32,
        ),
        egui::pos2(full.right(), full.bottom() - SCROLLBAR),
    );
    let horizontal = egui::Rect::from_min_max(
        egui::pos2(
            full.left() + layout.header_width as f32,
            full.bottom() - SCROLLBAR,
        ),
        egui::pos2(full.right() - SCROLLBAR, full.bottom()),
    );

    Bars {
        vertical,
        horizontal,
        vertical_thumb: thumb(vertical, Axis::Rows, viewport.y, extent.y, scroll.y as f32),
        horizontal_thumb: thumb(
            horizontal,
            Axis::Columns,
            viewport.x,
            extent.x,
            scroll.x as f32,
        ),
        extent,
    }
}

/// The thumb inside one track.
fn thumb(track: egui::Rect, axis: Axis, viewport: f32, extent: f32, scroll: f32) -> egui::Rect {
    let length = match axis {
        Axis::Rows => track.height(),
        Axis::Columns => track.width(),
    };
    if length <= 0.0 {
        return egui::Rect::NOTHING;
    }
    // Nothing to scroll: the thumb fills the track, which is how a scrollbar
    // says "this is all of it" without disappearing and reflowing the layout.
    let fraction = if extent <= 0.0 {
        1.0
    } else {
        (viewport / (viewport + extent)).clamp(0.0, 1.0)
    };
    let size = (length * fraction).max(MIN_THUMB.min(length));
    let travel = (length - size).max(0.0);
    let at = if extent <= 0.0 {
        0.0
    } else {
        travel * (scroll / extent).clamp(0.0, 1.0)
    };
    match axis {
        Axis::Rows => egui::Rect::from_min_size(
            egui::pos2(track.left() + 2.0, track.top() + at),
            egui::vec2(track.width() - 4.0, size),
        ),
        Axis::Columns => egui::Rect::from_min_size(
            egui::pos2(track.left() + at, track.top() + 2.0),
            egui::vec2(size, track.height() - 4.0),
        ),
    }
}

fn paint_bars(ui: &egui::Ui, bars: &Bars, palette: &Palette) {
    let painter = ui.painter();
    let hovered = ui.ctx().pointer_latest_pos();
    for (track, thumb) in [
        (bars.vertical, bars.vertical_thumb),
        (bars.horizontal, bars.horizontal_thumb),
    ] {
        if !track.is_positive() {
            continue;
        }
        painter.rect_filled(track, 0.0, palette.scrollbar_track);
        if !thumb.is_positive() {
            continue;
        }
        let lit = hovered.is_some_and(|p| track.contains(p));
        let color = if lit {
            palette.scrollbar.gamma_multiply(0.75)
        } else {
            palette.scrollbar
        };
        painter.rect_filled(thumb, egui::CornerRadius::same(5), color);
    }
}

/// A screen position in sheet space: pixels from the top-left of A1.
///
/// The frame of reference a drag has to be measured in. Screen coordinates move
/// when the sheet scrolls under the pointer; these do not.
fn sheet_space(pane: &Pane, pos: egui::Pos2) -> egui::Pos2 {
    egui::pos2(
        pane.scroll.x as f32 + (pos.x - pane.rect.left()),
        pane.scroll.y as f32 + (pos.y - pane.rect.top()),
    )
}

/// The pane a position is over, or the last one — which is the scrolling pane,
/// and the right answer for a drag that has wandered off the edge.
fn pane_at(panes: &[Pane], pos: egui::Pos2) -> Option<&Pane> {
    panes
        .iter()
        .find(|p| p.rect.contains(pos))
        .or_else(|| panes.last())
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
            // `italics` is epaint's shear, and it is not asked for: the italic
            // face is a different set of letterforms, not the roman leaning
            // over. The family already carries the slant.
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

/// Points to logical pixels, at a zoom.
///
/// The grid's default is 13 px for an 11 pt font and every other size follows
/// from that, so a 22 pt heading comes out twice the height of 11 pt body text.
///
/// The weight and the slant are part of the *family*, not of the drawing. Bold
/// used to be faked by stamping the glyphs twice half a pixel apart, which
/// smears at any size worth calling a heading; the four faces are loaded from
/// the system at startup and asked for by name instead.
fn cell_font(look: &super::CellLook, zoom: f32) -> egui::FontId {
    egui::FontId::new(
        (look.size * 13.0 / 11.0 * zoom).max(1.0),
        ui_kit::fonts::face(look.family, look.bold, look.italic),
    )
}

fn paint_text(
    painter: &egui::Painter,
    pos: egui::Pos2,
    galley: std::sync::Arc<egui::Galley>,
    color: egui::Color32,
) {
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

/// Fills `area` except where `hole` covers it.
///
/// Four strips around the hole rather than a fill-then-erase: the wash is
/// translucent, so painting over it with an "eraser" colour would also erase
/// the gridlines and any cell fill underneath.
fn fill_except(painter: &egui::Painter, area: egui::Rect, hole: egui::Rect, color: egui::Color32) {
    let hole = hole.intersect(area);
    if !hole.is_positive() {
        painter.rect_filled(area, 0.0, color);
        return;
    }
    let strips = [
        egui::Rect::from_min_max(area.min, egui::pos2(hole.left(), area.bottom())),
        egui::Rect::from_min_max(egui::pos2(hole.right(), area.top()), area.max),
        egui::Rect::from_min_max(
            egui::pos2(hole.left(), area.top()),
            egui::pos2(hole.right(), hole.top()),
        ),
        egui::Rect::from_min_max(
            egui::pos2(hole.left(), hole.bottom()),
            egui::pos2(hole.right(), area.bottom()),
        ),
    ];
    for strip in strips {
        if strip.is_positive() {
            painter.rect_filled(strip, 0.0, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Selection;
    use ss_model::cell::{MAX_COLS, MAX_ROWS};

    /// A context with the type faces installed.
    ///
    /// Not optional: the grid asks for `sans-bold` by name, and epaint panics
    /// rather than substituting when a family has never been registered. The
    /// shell installs them at startup; a test that drives the grid directly
    /// has to do the same thing.
    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        // Registered with no directories: the *names* are what the grid asks
        // for, and epaint panics rather than substituting for a family it has
        // never heard of. Reading a hundred megabytes of type off the disk is
        // not something a unit test needs to do.
        ui_kit::fonts::register(&ctx, &[]);
        ctx
    }

    /// Drives one egui frame with the given events, giving back the cursor the
    /// grid asked for while it ran.
    fn frame(
        view: &mut GridView,
        book: &mut Workbook,
        events: Vec<egui::Event>,
        ctx: &egui::Context,
    ) -> egui::CursorIcon {
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
        out.platform_output.cursor_icon
    }

    /// A ruled sheet: 64-pixel columns, 20-pixel rows, headers to match.
    ///
    /// Built by hand rather than from a workbook so the numbers in the
    /// assertions are the numbers in the fixture, and a boundary is where
    /// arithmetic says it is.
    fn ruled() -> (Layout, Vec<Pane>, egui::Rect, egui::Rect) {
        let layout = Layout {
            rows: super::super::axis::Axis::uniform(20.0, 1000),
            cols: super::super::axis::Axis::uniform(64.0, 100),
            header_width: 46.0,
            header_height: 20.0,
            zoom: 1.0,
        };
        let content = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0));
        let body = egui::Rect::from_min_max(content.min + egui::vec2(46.0, 20.0), content.max);
        let panes = vec![Pane {
            rect: body,
            scroll: Scroll::default(),
        }];
        (layout, panes, content, body)
    }

    #[test]
    fn a_boundary_can_be_grabbed_from_either_side_of_it() {
        // The bug this exists for: the grab zone used to be four pixels wide
        // and all of them to the left of the line, so half of every attempt to
        // drag a column landed on "select this column" instead.
        let (layout, panes, content, body) = ruled();
        let edge = |x: f32, y: f32| header_edge(&layout, content, body, &panes, egui::pos2(x, y));

        // Column A ends 64 pixels into the body, which is x = 110 on screen.
        assert_eq!(edge(110.0, 10.0), Some((Axis::Columns, 0)));
        assert_eq!(edge(108.0, 10.0), Some((Axis::Columns, 0)), "left of it");
        assert_eq!(edge(112.0, 10.0), Some((Axis::Columns, 0)), "right of it");
        assert_eq!(edge(140.0, 10.0), None, "the middle of column B");

        // Row 1 ends 20 pixels down, which is y = 40.
        assert_eq!(edge(20.0, 40.0), Some((Axis::Rows, 0)));
        assert_eq!(edge(20.0, 42.0), Some((Axis::Rows, 0)), "below it");
        assert_eq!(edge(20.0, 55.0), None, "the middle of row 2");
    }

    #[test]
    fn the_far_edge_of_the_first_header_resizes_nothing() {
        // There is no column before A and no row before 1, so the left edge of
        // one and the top edge of the other are lines with nothing behind them.
        // A drag there used to be possible and resized whatever came next.
        let (layout, panes, content, body) = ruled();
        let edge = |x: f32, y: f32| header_edge(&layout, content, body, &panes, egui::pos2(x, y));
        assert_eq!(edge(46.0, 10.0), None);
        assert_eq!(edge(20.0, 20.0), None);
        // And the corner box between the two headers is neither.
        assert_eq!(edge(20.0, 10.0), None);
        // Nor is anything outside the widget, which is what stops a pointer
        // over the toolbar from picking up a resize cursor.
        assert_eq!(edge(110.0, 900.0), None);
    }

    #[test]
    fn dragging_a_column_edge_writes_a_width_and_reports_it_once() {
        let ctx = context();
        let mut book = Workbook::blank();
        let mut view = GridView::default();
        frame(&mut view, &mut book, vec![], &ctx);

        // Where the boundary after column A actually is this frame, rather
        // than where the fixture above says it would be.
        let (_, _, layout) = view.layout.as_ref().expect("laid out");
        let x = 46.0 + layout.cols.size(0) as f32;
        let (from, to) = (egui::pos2(x, 10.0), egui::pos2(x + 40.0, 10.0));
        frame(&mut view, &mut book, vec![press(from, true)], &ctx);
        frame(
            &mut view,
            &mut book,
            vec![egui::Event::PointerMoved(to)],
            &ctx,
        );
        frame(&mut view, &mut book, vec![press(to, false)], &ctx);

        let width = book.sheets[0].column_widths.get(&0).copied();
        assert!(
            width.is_some_and(|w| w > super::super::axis::DEFAULT_COLUMN_CHARS),
            "column A got wider: {width:?}"
        );
        assert_eq!(
            view.actions
                .iter()
                .filter(|a| matches!(a, Action::Resized(_)))
                .count(),
            1,
            "one undo entry for the whole drag, not one per frame"
        );
        assert!(
            book.sheets[0].column_widths.len() == 1,
            "only the column that was dragged"
        );
    }

    /// The middle of the header cell for a column, and for a row.
    fn header_of(view: &GridView, axis: Axis, index: u32) -> egui::Pos2 {
        let (_, _, layout) = view.layout.as_ref().expect("laid out");
        let (hw, hh) = (layout.header_width as f32, layout.header_height as f32);
        match axis {
            Axis::Columns => egui::pos2(
                hw + (layout.cols.offset(index) + layout.cols.size(index) / 2.0) as f32,
                hh / 2.0,
            ),
            Axis::Rows => egui::pos2(
                hw / 2.0,
                hh + (layout.rows.offset(index) + layout.rows.size(index) / 2.0) as f32,
            ),
        }
    }

    #[test]
    fn a_column_stays_selected_while_the_pointer_is_still_down() {
        // The bug this exists for: a press on a header selected the whole
        // column and set an ordinary selection drag. The next frame extended
        // that drag towards "the cell under the pointer" — and the pointer was
        // over the header, which `cell_at` answers for anyway rather than
        // refusing — so the column collapsed to one cell before the button
        // came back up. The selection looked like it cleared itself.
        let ctx = context();
        let mut book = Workbook::blank();
        let mut view = GridView::default();
        frame(&mut view, &mut book, vec![], &ctx);

        let at = header_of(&view, Axis::Columns, 2);
        frame(&mut view, &mut book, vec![press(at, true)], &ctx);
        let whole = CellRange::new(CellRef::new(0, 2), CellRef::new(MAX_ROWS - 1, 2));
        assert_eq!(view.selection.ranges(), [whole], "on the press");
        // Two more frames with the button still down, which is where it went.
        frame(&mut view, &mut book, vec![], &ctx);
        frame(&mut view, &mut book, vec![], &ctx);
        assert_eq!(view.selection.ranges(), [whole], "still down");
        frame(&mut view, &mut book, vec![press(at, false)], &ctx);
        assert_eq!(view.selection.ranges(), [whole], "after the release");
    }

    #[test]
    fn a_row_stays_selected_too() {
        let ctx = context();
        let mut book = Workbook::blank();
        let mut view = GridView::default();
        frame(&mut view, &mut book, vec![], &ctx);

        let at = header_of(&view, Axis::Rows, 3);
        frame(&mut view, &mut book, vec![press(at, true)], &ctx);
        frame(&mut view, &mut book, vec![], &ctx);
        frame(&mut view, &mut book, vec![press(at, false)], &ctx);
        assert_eq!(
            view.selection.ranges(),
            [CellRange::new(
                CellRef::new(3, 0),
                CellRef::new(3, MAX_COLS - 1)
            )]
        );
    }

    #[test]
    fn sweeping_along_a_header_grows_one_band_rather_than_many() {
        let ctx = context();
        let mut book = Workbook::blank();
        let mut view = GridView::default();
        frame(&mut view, &mut book, vec![], &ctx);

        let from = header_of(&view, Axis::Columns, 1);
        let to = header_of(&view, Axis::Columns, 4);
        frame(&mut view, &mut book, vec![press(from, true)], &ctx);
        frame(
            &mut view,
            &mut book,
            vec![egui::Event::PointerMoved(to)],
            &ctx,
        );
        frame(&mut view, &mut book, vec![press(to, false)], &ctx);
        assert_eq!(
            view.selection.ranges(),
            [CellRange::new(
                CellRef::new(0, 1),
                CellRef::new(MAX_ROWS - 1, 4)
            )],
            "B through E, as one band"
        );
    }

    #[test]
    fn a_selected_column_can_be_dragged_somewhere_else() {
        let ctx = context();
        let mut book = Workbook::blank();
        let mut view = GridView::default();
        frame(&mut view, &mut book, vec![], &ctx);

        // Select B, then pick it up and drop it past D.
        let b = header_of(&view, Axis::Columns, 1);
        frame(&mut view, &mut book, vec![press(b, true)], &ctx);
        frame(&mut view, &mut book, vec![press(b, false)], &ctx);
        view.actions.clear();

        let d = header_of(&view, Axis::Columns, 3);
        frame(&mut view, &mut book, vec![press(b, true)], &ctx);
        assert!(
            matches!(
                view.drag,
                Some(Drag::MoveBand {
                    first: 1,
                    last: 1,
                    ..
                })
            ),
            "a press inside the band picks it up: {:?}",
            view.drag
        );
        frame(
            &mut view,
            &mut book,
            vec![egui::Event::PointerMoved(d)],
            &ctx,
        );
        // Past the middle of D, so it lands after it rather than before.
        let past = egui::pos2(d.x + 20.0, d.y);
        frame(
            &mut view,
            &mut book,
            vec![egui::Event::PointerMoved(past)],
            &ctx,
        );
        assert_eq!(view.move_target, Some(4), "the line sits after D");
        frame(&mut view, &mut book, vec![press(past, false)], &ctx);

        assert_eq!(
            view.actions,
            [Action::MoveBand {
                axis: Axis::Columns,
                first: 1,
                last: 1,
                before: 4,
            }]
        );
    }

    #[test]
    fn a_press_on_a_header_nobody_selected_still_selects_it() {
        // The two gestures share a press, so the one that is not wanted has to
        // stay out of the way: you cannot move what you have not selected.
        let ctx = context();
        let mut book = Workbook::blank();
        let mut view = GridView::default();
        frame(&mut view, &mut book, vec![], &ctx);

        let at = header_of(&view, Axis::Columns, 2);
        frame(&mut view, &mut book, vec![press(at, true)], &ctx);
        assert!(
            matches!(view.drag, Some(Drag::SelectHeaders { .. })),
            "{:?}",
            view.drag
        );
    }

    #[test]
    fn dropping_a_band_back_on_itself_asks_for_nothing() {
        let ctx = context();
        let mut book = Workbook::blank();
        let mut view = GridView::default();
        frame(&mut view, &mut book, vec![], &ctx);

        let b = header_of(&view, Axis::Columns, 1);
        frame(&mut view, &mut book, vec![press(b, true)], &ctx);
        frame(&mut view, &mut book, vec![press(b, false)], &ctx);
        view.actions.clear();

        frame(&mut view, &mut book, vec![press(b, true)], &ctx);
        frame(
            &mut view,
            &mut book,
            vec![egui::Event::PointerMoved(b)],
            &ctx,
        );
        assert_eq!(
            view.move_target, None,
            "no line, because nothing would move"
        );
        frame(&mut view, &mut book, vec![press(b, false)], &ctx);
        assert!(view.actions.is_empty(), "{:?}", view.actions);
    }

    #[test]
    fn double_clicking_a_boundary_asks_for_that_one_column_to_be_fitted() {
        let ctx = context();
        let mut book = Workbook::blank();
        let mut view = GridView::default();
        frame(&mut view, &mut book, vec![], &ctx);

        let (_, _, layout) = view.layout.as_ref().expect("laid out");
        let at = egui::pos2(46.0 + layout.cols.size(0) as f32, 10.0);
        for _ in 0..2 {
            frame(&mut view, &mut book, vec![press(at, true)], &ctx);
            frame(&mut view, &mut book, vec![press(at, false)], &ctx);
        }

        assert!(
            view.actions.contains(&Action::AutoFitAt {
                axis: Axis::Columns,
                index: 0,
            }),
            "the column the pointer named: {:?}",
            view.actions
        );
        // The first click of the pair still opened and closed a resize — a
        // press on a boundary cannot know a second one is coming. What matters
        // is that it moved nothing, so what it reports is the geometry the
        // sheet still has and the application drops it rather than stacking
        // two do-nothing entries on top of the fit.
        for action in &view.actions {
            if let Action::Resized(before) = action {
                assert_eq!(before.column_widths, book.sheets[0].column_widths);
                assert_eq!(before.row_heights, book.sheets[0].row_heights);
            }
        }
    }

    /// The middle of a cell, well clear of any boundary.
    fn cell_of(view: &GridView, at: CellRef) -> egui::Pos2 {
        let (_, _, layout) = view.layout.as_ref().expect("laid out");
        egui::pos2(
            layout.header_width as f32
                + (layout.cols.offset(at.col) + layout.cols.size(at.col) / 2.0) as f32,
            layout.header_height as f32
                + (layout.rows.offset(at.row) + layout.rows.size(at.row) / 2.0) as f32,
        )
    }

    #[test]
    fn double_clicking_a_cell_opens_it_for_editing() {
        // The same trap the boundary fit fell into: the second click arrives
        // while the first one's selection drag still owns the pointer, and the
        // drag branch returns before anything else is looked at.
        let ctx = context();
        let mut book = Workbook::blank();
        let mut view = GridView::default();
        frame(&mut view, &mut book, vec![], &ctx);

        let at = CellRef::new(3, 2);
        let pos = cell_of(&view, at);
        for _ in 0..2 {
            frame(&mut view, &mut book, vec![press(pos, true)], &ctx);
            frame(&mut view, &mut book, vec![press(pos, false)], &ctx);
        }

        let editor = view.editor.as_ref().expect("the cell opened");
        assert_eq!(editor.at, at);
        assert!(view.drag.is_none(), "the sweep does not outlive the pair");
    }

    #[test]
    fn double_clicking_a_header_selects_it_rather_than_opening_an_editor() {
        let ctx = context();
        let mut book = Workbook::blank();
        let mut view = GridView::default();
        frame(&mut view, &mut book, vec![], &ctx);

        let at = header_of(&view, Axis::Columns, 2);
        for _ in 0..2 {
            frame(&mut view, &mut book, vec![press(at, true)], &ctx);
            frame(&mut view, &mut book, vec![press(at, false)], &ctx);
        }
        assert!(view.editor.is_none(), "there is no cell up there to edit");
    }

    #[test]
    fn the_resize_arrow_stays_put_for_as_long_as_the_drag_does() {
        // The pointer leaves the boundary the moment it starts dragging it, so
        // an icon chosen by hovering drops back to a plain arrow halfway
        // through — which reads as the drag having been let go.
        let ctx = context();
        let mut book = Workbook::blank();
        let mut view = GridView::default();
        frame(&mut view, &mut book, vec![], &ctx);

        let (_, _, layout) = view.layout.as_ref().expect("laid out");
        let edge = egui::pos2(46.0 + layout.cols.size(0) as f32, 10.0);
        frame(&mut view, &mut book, vec![press(edge, true)], &ctx);
        assert!(matches!(view.drag, Some(Drag::ResizeColumn { .. })));

        let away = edge + egui::vec2(120.0, 300.0);
        let icon = frame(
            &mut view,
            &mut book,
            vec![egui::Event::PointerMoved(away)],
            &ctx,
        );
        assert_eq!(icon, egui::CursorIcon::ResizeHorizontal);
    }

    fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }
    }

    fn right_press(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }
    }

    #[test]
    fn a_right_click_inside_the_selection_leaves_it_alone() {
        // The bug this exists for: every press ran the selection gesture,
        // whichever button it came from — so right-clicking a selected range
        // collapsed it to one cell before the menu opened, and "right-click,
        // Sort" sorted a single cell.
        let ctx = context();
        let mut book = Workbook::blank();
        let mut view = GridView::default();
        frame(&mut view, &mut book, vec![], &ctx);

        let from = cell_of(&view, CellRef::new(1, 1));
        let to = cell_of(&view, CellRef::new(3, 2));
        frame(&mut view, &mut book, vec![press(from, true)], &ctx);
        frame(&mut view, &mut book, vec![egui::Event::PointerMoved(to)], &ctx);
        frame(&mut view, &mut book, vec![press(to, false)], &ctx);
        let range = CellRange::new(CellRef::new(1, 1), CellRef::new(3, 2));
        assert_eq!(view.selection.ranges(), [range]);

        let inside = cell_of(&view, CellRef::new(2, 2));
        frame(&mut view, &mut book, vec![right_press(inside, true)], &ctx);
        frame(&mut view, &mut book, vec![right_press(inside, false)], &ctx);
        assert_eq!(view.selection.ranges(), [range], "the selection is kept");
        assert_eq!(view.selection.cursor(), CellRef::new(1, 1), "so is the cursor");

        // Outside the selection it moves there first, as Excel's does.
        let outside = cell_of(&view, CellRef::new(6, 4));
        frame(&mut view, &mut book, vec![right_press(outside, true)], &ctx);
        assert_eq!(view.selection.cursor(), CellRef::new(6, 4));
        assert!(view.selection.is_single_cell());
        assert!(view.drag.is_none(), "a right press never starts a drag");
    }

    #[test]
    fn dragging_a_selection_keeps_the_active_cell_where_it_started() {
        let ctx = context();
        let mut book = Workbook::blank();
        let mut view = GridView::default();
        frame(&mut view, &mut book, vec![], &ctx);

        let from = cell_of(&view, CellRef::new(1, 1));
        let to = cell_of(&view, CellRef::new(4, 3));
        frame(&mut view, &mut book, vec![press(from, true)], &ctx);
        frame(&mut view, &mut book, vec![egui::Event::PointerMoved(to)], &ctx);
        frame(&mut view, &mut book, vec![press(to, false)], &ctx);
        assert_eq!(
            view.selection.active_range(),
            CellRange::new(CellRef::new(1, 1), CellRef::new(4, 3))
        );
        assert_eq!(
            view.selection.cursor(),
            CellRef::new(1, 1),
            "typing lands where the drag began, not where it ended"
        );
    }

    #[test]
    fn holding_a_drag_past_the_edge_scrolls_the_view() {
        let ctx = context();
        let mut book = Workbook::blank();
        let mut view = GridView::default();
        frame(&mut view, &mut book, vec![], &ctx);

        let from = cell_of(&view, CellRef::new(2, 1));
        frame(&mut view, &mut book, vec![press(from, true)], &ctx);
        // Park the pointer below the bottom edge and let frames pass. Each
        // one nudges the scroll further, even with no new pointer events.
        let below = egui::pos2(from.x, 5000.0);
        frame(&mut view, &mut book, vec![egui::Event::PointerMoved(below)], &ctx);
        let after_one = view.scroll.y;
        assert!(after_one > 0.0, "the view moved: {after_one}");
        frame(&mut view, &mut book, vec![], &ctx);
        frame(&mut view, &mut book, vec![], &ctx);
        assert!(view.scroll.y > after_one, "and keeps moving while held");
        frame(&mut view, &mut book, vec![press(below, false)], &ctx);
        assert!(
            view.selection.active_range().rows() > 1,
            "the sweep selected past the first screenful: {:?}",
            view.selection.active_range()
        );
    }

    #[test]
    fn the_pointer_over_the_fill_handle_is_a_thin_cross() {
        let ctx = context();
        let mut book = Workbook::blank();
        let mut view = GridView::default();
        frame(&mut view, &mut book, vec![], &ctx);

        // The handle sits on the bottom-right corner of A1.
        let (_, _, layout) = view.layout.as_ref().expect("laid out");
        let corner = egui::pos2(
            layout.header_width as f32 + layout.cols.size(0) as f32,
            layout.header_height as f32 + layout.rows.size(0) as f32,
        );
        let icon = frame(
            &mut view,
            &mut book,
            vec![egui::Event::PointerMoved(corner)],
            &ctx,
        );
        assert_eq!(icon, egui::CursorIcon::Crosshair);

        // And it stays a cross for the whole fill drag, even though the
        // pointer leaves the handle on the first frame of it.
        frame(&mut view, &mut book, vec![press(corner, true)], &ctx);
        let away = cell_of(&view, CellRef::new(4, 0));
        let icon = frame(
            &mut view,
            &mut book,
            vec![egui::Event::PointerMoved(away)],
            &ctx,
        );
        assert_eq!(icon, egui::CursorIcon::Crosshair);
        frame(&mut view, &mut book, vec![press(away, false)], &ctx);
    }

    /// A workbook with one picture anchored over B2:D5, near enough.
    fn with_picture() -> Workbook {
        use ss_model::chart::{Anchor, AnchorPoint};
        let mut book = Workbook::blank();
        let sheet = book.sheet_mut(0).expect("a sheet");
        sheet.pictures.push(ss_model::Picture {
            part: "/xl/media/image1.png".into(),
            drawing_part: "/xl/drawings/drawing1.xml".into(),
            anchor_index: 0,
            name: "Picture 3".into(),
            anchor: Anchor::TwoCell {
                from: AnchorPoint {
                    col: 1,
                    col_offset: 0,
                    row: 1,
                    row_offset: 0,
                },
                to: AnchorPoint {
                    col: 4,
                    col_offset: 0,
                    row: 5,
                    row_offset: 0,
                },
            },
            data: std::sync::Arc::from(Vec::new().into_boxed_slice()),
            content_type: "image/png".into(),
        });
        book
    }

    /// Where a picture is on screen, given the same layout the view uses.
    fn picture_rect(view: &GridView, book: &Workbook) -> egui::Rect {
        let sheet = book.sheet(0).expect("a sheet");
        let layout = Layout::for_sheet(book, sheet, view.zoom);
        let origin = egui::pos2(layout.header_width as f32, layout.header_height as f32);
        let rect = crate::grid::picture::sheet_rect(&layout, &sheet.pictures[0].anchor);
        rect.translate(origin.to_vec2())
    }

    #[test]
    fn clicking_a_picture_selects_it_rather_than_the_cell_under_it() {
        let ctx = context();
        let mut book = with_picture();
        let mut view = GridView::default();
        // A first frame so the view has a layout; the rect is the same either way.
        frame(&mut view, &mut book, Vec::new(), &ctx);
        let middle = picture_rect(&view, &book).center();

        frame(&mut view, &mut book, vec![press(middle, true)], &ctx);
        frame(&mut view, &mut book, vec![press(middle, false)], &ctx);

        assert_eq!(view.selected_picture, Some(0));
        // And a click away from it puts the selection back on the cells.
        let away = egui::pos2(700.0, 600.0);
        frame(&mut view, &mut book, vec![press(away, true)], &ctx);
        frame(&mut view, &mut book, vec![press(away, false)], &ctx);
        assert_eq!(view.selected_picture, None);
    }

    #[test]
    fn dragging_a_picture_moves_it_and_reports_where_it_was() {
        let ctx = context();
        let mut book = with_picture();
        let mut view = GridView::default();
        frame(&mut view, &mut book, Vec::new(), &ctx);
        let before = book.sheet(0).expect("a sheet").pictures[0].anchor.clone();
        let middle = picture_rect(&view, &book).center();

        frame(&mut view, &mut book, vec![press(middle, true)], &ctx);
        frame(
            &mut view,
            &mut book,
            vec![egui::Event::PointerMoved(middle + egui::vec2(60.0, 40.0))],
            &ctx,
        );
        frame(
            &mut view,
            &mut book,
            vec![press(middle + egui::vec2(60.0, 40.0), false)],
            &ctx,
        );

        let after = &book.sheet(0).expect("a sheet").pictures[0].anchor;
        assert_ne!(after, &before, "the drag moved it");
        assert!(
            view.actions
                .iter()
                .any(|a| matches!(a, Action::PicturesMoved(_))),
            "{:?}",
            view.actions
        );
    }

    #[test]
    fn dragging_the_east_handle_stretches_without_moving_the_left_edge() {
        let ctx = context();
        let mut book = with_picture();
        let mut view = GridView::default();
        frame(&mut view, &mut book, Vec::new(), &ctx);
        let rect = picture_rect(&view, &book);
        let middle = rect.center();

        // Select first: handles only exist on a selected picture.
        frame(&mut view, &mut book, vec![press(middle, true)], &ctx);
        frame(&mut view, &mut book, vec![press(middle, false)], &ctx);

        let handle = crate::grid::picture::Handle::East.at(rect);
        let pulled = handle + egui::vec2(80.0, 0.0);
        frame(&mut view, &mut book, vec![press(handle, true)], &ctx);
        frame(
            &mut view,
            &mut book,
            vec![egui::Event::PointerMoved(pulled)],
            &ctx,
        );
        frame(&mut view, &mut book, vec![press(pulled, false)], &ctx);

        let after = picture_rect(&view, &book);
        assert!((after.left() - rect.left()).abs() < 0.5, "{after:?}");
        assert!((after.top() - rect.top()).abs() < 0.5, "{after:?}");
        assert!((after.bottom() - rect.bottom()).abs() < 0.5, "{after:?}");
        assert!(after.right() > rect.right() + 60.0, "{after:?}");
    }

    #[test]
    fn delete_with_a_picture_selected_removes_the_picture_not_the_cells() {
        let ctx = context();
        let mut book = with_picture();
        let mut view = GridView::default();
        frame(&mut view, &mut book, Vec::new(), &ctx);
        let middle = picture_rect(&view, &book).center();
        frame(&mut view, &mut book, vec![press(middle, true)], &ctx);
        frame(&mut view, &mut book, vec![press(middle, false)], &ctx);
        view.actions.clear();

        frame(&mut view, &mut book, vec![plain(egui::Key::Delete)], &ctx);

        assert_eq!(view.actions, vec![Action::DeletePicture(0)]);
        assert_eq!(view.selected_picture, None);
    }

    #[test]
    fn delete_with_nothing_but_cells_selected_still_clears_cells() {
        let ctx = context();
        let mut book = with_picture();
        let mut view = GridView::default();
        frame(&mut view, &mut book, Vec::new(), &ctx);
        frame(&mut view, &mut book, vec![plain(egui::Key::Delete)], &ctx);
        assert_eq!(view.actions, vec![Action::Clear]);
    }

    #[test]
    fn a_click_does_not_leave_a_drag_running() {
        let ctx = context();
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
        (context(), Workbook::blank(), GridView::default())
    }

    /// Fills the named cells with a number, so there is data to navigate.
    fn seed(book: &mut Workbook, cells: &[&str]) {
        let sheet = book.sheet_mut(0).expect("a sheet");
        for at in cells {
            sheet.set(
                CellRef::from_a1(at).expect("test address"),
                ss_model::Cell {
                    value: ss_model::CellValue::Number(1.0),
                    ..Default::default()
                },
            );
        }
    }

    #[test]
    fn ctrl_a_selects_the_region_first_and_the_sheet_second() {
        let (ctx, mut book, mut view) = typing();
        seed(&mut book, &["B2", "B3", "C2", "C3"]);
        frame(&mut view, &mut book, vec![], &ctx);
        let b2 = cell_of(&view, CellRef::new(1, 1));
        frame(&mut view, &mut book, vec![press(b2, true)], &ctx);
        frame(&mut view, &mut book, vec![press(b2, false)], &ctx);

        frame(&mut view, &mut book, vec![ctrl(egui::Key::A)], &ctx);
        assert_eq!(
            view.selection.ranges(),
            [CellRange::new(CellRef::new(1, 1), CellRef::new(2, 2))],
            "first press: the island"
        );
        frame(&mut view, &mut book, vec![ctrl(egui::Key::A)], &ctx);
        assert_eq!(
            view.selection.active_range().rows(),
            MAX_ROWS,
            "second press: everything"
        );
        assert_eq!(
            view.selection.cursor(),
            CellRef::new(1, 1),
            "the active cell never moved"
        );
    }

    #[test]
    fn ctrl_shift_space_selects_everything() {
        let (ctx, mut book, mut view) = typing();
        frame(&mut view, &mut book, vec![], &ctx);
        frame(
            &mut view,
            &mut book,
            vec![key(egui::Key::Space, egui::Modifiers::CTRL | egui::Modifiers::SHIFT)],
            &ctx,
        );
        let all = view.selection.active_range();
        assert_eq!((all.rows(), all.cols()), (MAX_ROWS, MAX_COLS));
    }

    #[test]
    fn end_and_ctrl_end_mirror_home() {
        let (ctx, mut book, mut view) = typing();
        seed(&mut book, &["A1", "C1", "B5"]);
        frame(&mut view, &mut book, vec![], &ctx);

        frame(&mut view, &mut book, vec![plain(egui::Key::End)], &ctx);
        assert_eq!(
            view.selection.cursor(),
            CellRef::new(0, 2),
            "End: the last filled cell of the row"
        );
        frame(&mut view, &mut book, vec![ctrl(egui::Key::End)], &ctx);
        assert_eq!(
            view.selection.cursor(),
            CellRef::new(4, 2),
            "Ctrl+End: the bottom-right of everything used"
        );
        frame(&mut view, &mut book, vec![plain(egui::Key::Home)], &ctx);
        assert_eq!(view.selection.cursor(), CellRef::new(4, 0));
    }

    #[test]
    fn ctrl_nine_and_zero_hide_what_shift_unhides() {
        let (ctx, mut book, mut view) = typing();
        frame(&mut view, &mut book, vec![], &ctx);
        frame(&mut view, &mut book, vec![ctrl(egui::Key::Num9)], &ctx);
        frame(&mut view, &mut book, vec![ctrl(egui::Key::Num0)], &ctx);
        frame(
            &mut view,
            &mut book,
            vec![key(egui::Key::Num9, egui::Modifiers::CTRL | egui::Modifiers::SHIFT)],
            &ctx,
        );
        assert_eq!(
            view.actions,
            [
                Action::Visibility {
                    axis: Axis::Rows,
                    hide: true
                },
                Action::Visibility {
                    axis: Axis::Columns,
                    hide: true
                },
                Action::Visibility {
                    axis: Axis::Rows,
                    hide: false
                },
            ]
        );
    }

    #[test]
    fn the_number_format_row_reaches_the_grid() {
        let (ctx, mut book, mut view) = typing();
        frame(&mut view, &mut book, vec![], &ctx);
        // Ctrl+Shift+5 is percent; bare Ctrl+5 stays strikethrough.
        frame(
            &mut view,
            &mut book,
            vec![key(egui::Key::Num5, egui::Modifiers::CTRL | egui::Modifiers::SHIFT)],
            &ctx,
        );
        frame(&mut view, &mut book, vec![ctrl(egui::Key::Num5)], &ctx);
        assert_eq!(
            view.actions,
            [
                Action::Format(Format::NumberFormat("0%".into())),
                Action::Format(Format::Strike),
            ]
        );
    }

    #[test]
    fn clicking_a_cell_mid_formula_points_at_it() {
        let (ctx, mut book, mut view) = typing();
        frame(&mut view, &mut book, vec![egui::Event::Text("=".into())], &ctx);

        // Click C3: the reference goes into the formula, the editor stays.
        let c3 = cell_of(&view, CellRef::new(2, 2));
        frame(&mut view, &mut book, vec![press(c3, true)], &ctx);
        assert_eq!(view.editor.as_ref().map(|e| e.text.as_str()), Some("=C3"));
        // Dragging stretches it into a range before the button comes up.
        let d4 = cell_of(&view, CellRef::new(3, 3));
        frame(&mut view, &mut book, vec![egui::Event::PointerMoved(d4)], &ctx);
        frame(&mut view, &mut book, vec![press(d4, false)], &ctx);
        assert_eq!(view.editor.as_ref().map(|e| e.text.as_str()), Some("=C3:D4"));

        // An operator locks it; the next click starts the second reference.
        view.editor.as_mut().expect("open").text.push('+');
        let b1 = cell_of(&view, CellRef::new(0, 1));
        frame(&mut view, &mut book, vec![press(b1, true)], &ctx);
        frame(&mut view, &mut book, vec![press(b1, false)], &ctx);
        assert_eq!(
            view.editor.as_ref().map(|e| e.text.as_str()),
            Some("=C3:D4+B1")
        );

        // While the reference is live, another click replaces it — Excel
        // keeps replacing until an operator or Enter locks it in.
        let a5 = cell_of(&view, CellRef::new(4, 0));
        frame(&mut view, &mut book, vec![press(a5, true)], &ctx);
        frame(&mut view, &mut book, vec![press(a5, false)], &ctx);
        assert_eq!(
            view.editor.as_ref().map(|e| e.text.as_str()),
            Some("=C3:D4+A5")
        );
        frame(&mut view, &mut book, vec![plain(egui::Key::Enter)], &ctx);
        assert!(view.editor.is_none());
        assert!(!view.actions.is_empty());
    }

    #[test]
    fn arrows_point_while_a_formula_wants_a_reference() {
        let (ctx, mut book, mut view) = typing();
        frame(&mut view, &mut book, vec![], &ctx);
        // Start at B2 so there is room to arrow in every direction.
        let b2 = cell_of(&view, CellRef::new(1, 1));
        frame(&mut view, &mut book, vec![press(b2, true)], &ctx);
        frame(&mut view, &mut book, vec![press(b2, false)], &ctx);
        frame(&mut view, &mut book, vec![egui::Event::Text("=".into())], &ctx);

        frame(&mut view, &mut book, vec![plain(egui::Key::ArrowDown)], &ctx);
        assert_eq!(view.editor.as_ref().map(|e| e.text.as_str()), Some("=B3"));
        frame(&mut view, &mut book, vec![plain(egui::Key::ArrowRight)], &ctx);
        assert_eq!(
            view.editor.as_ref().map(|e| e.text.as_str()),
            Some("=C3"),
            "arrows move the pointed reference, not the caret"
        );
        frame(
            &mut view,
            &mut book,
            vec![key(egui::Key::ArrowDown, egui::Modifiers::SHIFT)],
            &ctx,
        );
        assert_eq!(
            view.editor.as_ref().map(|e| e.text.as_str()),
            Some("=C3:C4"),
            "shift stretches it"
        );
        // F4 pins it, and pinning takes the reference out of point mode.
        frame(&mut view, &mut book, vec![plain(egui::Key::F4)], &ctx);
        assert_eq!(
            view.editor.as_ref().map(|e| e.text.as_str()),
            Some("=$C$3:$C$4")
        );
        // Enter still commits.
        frame(&mut view, &mut book, vec![plain(egui::Key::Enter)], &ctx);
        assert!(view.editor.is_none());
    }

    #[test]
    fn ctrl_enter_asks_for_the_whole_selection() {
        let (ctx, mut book, mut view) = typing();
        frame(&mut view, &mut book, vec![], &ctx);
        let from = cell_of(&view, CellRef::new(1, 1));
        let to = cell_of(&view, CellRef::new(3, 1));
        frame(&mut view, &mut book, vec![press(from, true)], &ctx);
        frame(&mut view, &mut book, vec![egui::Event::PointerMoved(to)], &ctx);
        frame(&mut view, &mut book, vec![press(to, false)], &ctx);
        frame(&mut view, &mut book, vec![egui::Event::Text("7".into())], &ctx);
        frame(
            &mut view,
            &mut book,
            vec![key(egui::Key::Enter, egui::Modifiers::CTRL)],
            &ctx,
        );
        assert!(view.editor.is_none());
        assert_eq!(
            view.actions,
            [Action::CommitAll {
                at: CellRef::new(1, 1),
                text: "7".into()
            }]
        );
    }

    #[test]
    fn ctrl_semicolon_types_todays_date() {
        let (ctx, mut book, mut view) = typing();
        frame(&mut view, &mut book, vec![], &ctx);
        frame(&mut view, &mut book, vec![ctrl(egui::Key::Semicolon)], &ctx);
        let open = view.editor.as_ref().expect("an editor opened");
        assert_eq!(open.text.matches('/').count(), 2, "m/d/yyyy: {}", open.text);
        assert_eq!(open.mode, Mode::Enter, "commits like typed digits");
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
        let ctx = context();
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
