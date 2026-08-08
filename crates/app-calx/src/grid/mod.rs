//! The spreadsheet grid.
//!
//! The grid is not built from egui widgets. A sheet is a million rows by sixteen
//! thousand columns; nothing that allocates per cell can exist. What happens
//! each frame is:
//!
//! 1. Ask the axes which rows and columns the viewport covers — two binary
//!    searches, not a walk.
//! 2. Build a [`Plan`] of what to draw, which touches only those cells.
//! 3. Paint it.
//!
//! Step 2 is a pure function, which is what lets the frame cost be measured in
//! a test with no window and no GPU. The exit criterion for this chunk is that a
//! million-row sheet scrolls at frame rate, and the only way to know is to
//! measure the part that scales with the sheet.

pub mod axis;
pub mod editor;
pub mod paint;
pub mod selection;

use ss_formula::edit::Geometry;
use ss_model::numfmt::FormatValue;
use ss_model::{Axis, CellRange, CellRef, CellValue, Sheet, Workbook};
use ui_kit::egui;

pub use axis::Layout;
pub use editor::{Editor, Mode};
pub use selection::{Direction, Selection};

/// How far a click may be from a header edge and still grab it.
const RESIZE_GRAB: f32 = 4.0;

/// The most columns a long label may overflow into.
///
/// Excel has no limit, but each one is a cell lookup on every frame, and a label
/// wider than this is unreadable anyway.
const MAX_OVERFLOW: u32 = 12;

/// One cell to draw.
pub struct PaintCell {
    pub rect: egui::Rect,
    pub text: String,
    pub color: Option<[u8; 3]>,
    /// Numbers sit right, text sits left.
    pub numeric: bool,
    /// Extra width borrowed from empty neighbours, for a label that overruns.
    pub overflow: f32,
}

/// Everything one frame needs to draw, and nothing about the rest of the sheet.
pub struct Plan {
    pub rows: std::ops::RangeInclusive<u32>,
    pub cols: std::ops::RangeInclusive<u32>,
    pub cells: Vec<PaintCell>,
}

/// Where a pane's top-left corner sits in the sheet, in sheet pixels.
///
/// Separate from the screen rectangle because the two live in different number
/// systems: the sheet is twenty million pixels tall and needs `f64`, while a
/// screen coordinate is a small `f32`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Scroll {
    pub x: f64,
    pub y: f64,
}

impl Scroll {
    pub fn clamped(self) -> Self {
        Scroll {
            x: self.x.max(0.0),
            y: self.y.max(0.0),
        }
    }
}

/// Builds the draw list for one pane.
///
/// `view` is where the pane sits on screen and `scroll` is the part of the sheet
/// it shows. Every rectangle comes back in screen coordinates, worked out
/// relative to `view`, so nothing downstream ever handles a sheet-sized number.
pub fn plan(
    book: &Workbook,
    sheet: &Sheet,
    layout: &Layout,
    view: egui::Rect,
    scroll: Scroll,
) -> Plan {
    let rows = layout
        .rows
        .visible(scroll.y, scroll.y + f64::from(view.height()));
    let cols = layout
        .cols
        .visible(scroll.x, scroll.x + f64::from(view.width()));

    let mut cells = Vec::new();
    if rows.is_empty() || cols.is_empty() {
        return Plan { rows, cols, cells };
    }

    for row in rows.clone() {
        for col in cols.clone() {
            let at = CellRef::new(row, col);
            // A cell covered by a merge is drawn by the merge's anchor.
            let merge = sheet.merge_at(at);
            if merge.is_some_and(|m| m.start != at) {
                continue;
            }
            let Some(cell) = sheet.get(at) else { continue };
            if cell.value.is_blank() {
                continue;
            }

            let value = match cell.value {
                CellValue::Blank => FormatValue::Blank,
                CellValue::Number(n) => FormatValue::Number(n),
                CellValue::Bool(b) => FormatValue::Bool(b),
                CellValue::Error(e) => FormatValue::Error(e),
                CellValue::Text(id) => FormatValue::Text(book.strings.resolve(id)),
            };
            let shown = book.styles.number_format(cell.style).format(value);
            if shown.text.is_empty() {
                continue;
            }

            let rect = match merge {
                Some(m) => rect_of_range(layout, *m, view, scroll),
                None => rect_of(layout, at, view, scroll),
            };
            // Text runs on into empty neighbours; a number never does — it
            // turns into `####` instead, which the painter decides once it can
            // measure the glyphs.
            let overflow = if shown.numeric || merge.is_some() {
                0.0
            } else {
                overflow_width(sheet, at, layout)
            };
            cells.push(PaintCell {
                rect,
                text: shown.text,
                color: shown.color,
                numeric: shown.numeric,
                overflow,
            });
        }
    }
    Plan { rows, cols, cells }
}

/// How far a label may run past its own column before hitting something.
fn overflow_width(sheet: &Sheet, at: CellRef, layout: &Layout) -> f32 {
    let mut extra = 0.0;
    for step in 1..=MAX_OVERFLOW {
        let next = CellRef::new(at.row, at.col + step);
        if !next.is_valid() {
            break;
        }
        let occupied = sheet.get(next).is_some_and(|c| !c.value.is_blank());
        if occupied || sheet.merge_at(next).is_some() {
            break;
        }
        extra += layout.cols.size(next.col) as f32;
    }
    extra
}

/// A cell's rectangle on screen.
///
/// The subtraction happens in `f64` and only the result becomes an `f32`, which
/// is what keeps a cell three-quarters of the way down a million rows landing on
/// the pixel it was painted at.
pub fn rect_of(layout: &Layout, at: CellRef, view: egui::Rect, scroll: Scroll) -> egui::Rect {
    let x = layout.cols.offset(at.col) - scroll.x;
    let y = layout.rows.offset(at.row) - scroll.y;
    egui::Rect::from_min_size(
        view.min + egui::vec2(x as f32, y as f32),
        egui::vec2(
            layout.cols.size(at.col) as f32,
            layout.rows.size(at.row) as f32,
        ),
    )
}

pub fn rect_of_range(
    layout: &Layout,
    range: CellRange,
    view: egui::Rect,
    scroll: Scroll,
) -> egui::Rect {
    rect_of(layout, range.start, view, scroll).union(rect_of(layout, range.end, view, scroll))
}

/// Something the user asked for that the grid cannot do on its own.
///
/// The grid owns geometry and selection; it does not own the document. Editing
/// a cell has to go through undo, and undo is the application's, so the grid
/// reports the intent and the application performs it. That is also what makes
/// the whole keyboard table testable without a document.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// The editor was closed with Enter, Tab, or a click elsewhere.
    Commit {
        at: CellRef,
        text: String,
        advance: Option<Direction>,
    },
    /// Delete: empty the selection but keep its formatting.
    Clear,
    Insert(Axis),
    Delete(Axis),
    Undo,
    Redo,
    Copy {
        cut: bool,
    },
    /// Text arrived from the system clipboard.
    Paste(String),
    /// The fill handle was dragged from `from` out to `to`.
    Fill {
        from: CellRange,
        to: CellRange,
    },
    /// A row or column resize finished; the payload is how the sheet looked
    /// before it started, which is the whole undo entry.
    Resized(Geometry),
}

/// What the formula bar and the editor show for a cell: its formula if it has
/// one, otherwise its value written the way the user would type it.
///
/// Deliberately not the *displayed* text. A cell showing `15-Jan-24` is edited
/// as the date it holds, and a cell showing `1,235` is edited as `1234.56`.
pub fn source_text(book: &Workbook, sheet: usize, at: CellRef) -> String {
    let Some(sheet) = book.sheet(sheet) else {
        return String::new();
    };
    if let Some(formula) = sheet.formula_at(at) {
        if !formula.text.is_empty() {
            return format!("={}", formula.text);
        }
    }
    match sheet.get(at).map(|c| c.value) {
        Some(CellValue::Text(id)) => book.strings.resolve(id).to_string(),
        Some(CellValue::Number(n)) => ss_model::format_general(n),
        Some(CellValue::Bool(b)) => if b { "TRUE" } else { "FALSE" }.to_string(),
        Some(CellValue::Error(e)) => e.as_str().to_string(),
        _ => String::new(),
    }
}

/// The grid widget's own state, kept between frames.
pub struct GridView {
    pub sheet_index: usize,
    pub selection: Selection,
    /// Scroll position in sheet pixels, for the scrolling pane.
    pub scroll: Scroll,
    pub zoom: f64,
    /// The open cell editor, if any. The formula bar edits the same buffer.
    pub editor: Option<Editor>,
    /// What the last frame asked the application to do. Drained by the caller.
    pub actions: Vec<Action>,
    /// Rebuilt only when the sheet, the zoom, or a size changes — building it
    /// per frame would sort every row height on a sheet that has many.
    pub(crate) layout: Option<(usize, u32, Layout)>,
    /// Bumped whenever a row or column is resized, to invalidate the cache.
    pub(crate) generation: u32,
    pub(crate) drag: Option<Drag>,
    /// How the sheet looked before the resize drag in progress started.
    pub(crate) before_resize: Option<Geometry>,
    /// The rectangle the fill drag currently covers.
    pub(crate) fill_target: Option<CellRange>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Drag {
    /// Sweeping a selection out from the anchor.
    Select,
    /// Dragging a column's right edge, or a row's bottom edge.
    ResizeColumn {
        index: u32,
        origin: f32,
        start: f32,
    },
    ResizeRow {
        index: u32,
        origin: f32,
        start: f32,
    },
    /// Dragging the small square at the corner of the selection.
    Fill {
        from: CellRange,
    },
}

impl Default for GridView {
    fn default() -> Self {
        GridView {
            sheet_index: 0,
            selection: Selection::default(),
            scroll: Scroll::default(),
            zoom: 1.0,
            editor: None,
            actions: Vec::new(),
            layout: None,
            generation: 0,
            drag: None,
            before_resize: None,
            fill_target: None,
        }
    }
}

impl GridView {
    /// Takes everything the grid asked for since the last call.
    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    /// Closes the editor, reporting what was in it.
    pub fn commit(&mut self, advance: Option<Direction>) {
        if let Some(editor) = self.editor.take() {
            self.actions.push(Action::Commit {
                at: editor.at,
                text: editor.text,
                advance,
            });
        }
    }

    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn set_zoom(&mut self, zoom: f64) {
        let zoom = zoom.clamp(0.25, 4.0);
        if zoom != self.zoom {
            self.zoom = zoom;
            self.invalidate();
        }
    }

    /// Scrolls the least distance that brings `at` fully into view.
    pub fn scroll_into_view(&mut self, at: CellRef, body: egui::Vec2, sheet: &Sheet) {
        let layout = Layout::for_sheet(sheet, self.zoom);
        let (width, height) = (f64::from(body.x), f64::from(body.y));
        let (x, w) = (layout.cols.offset(at.col), layout.cols.size(at.col));
        let (y, h) = (layout.rows.offset(at.row), layout.rows.size(at.row));
        if x < self.scroll.x {
            self.scroll.x = x;
        } else if x + w > self.scroll.x + width {
            self.scroll.x = x + w - width;
        }
        if y < self.scroll.y {
            self.scroll.y = y;
        } else if y + h > self.scroll.y + height {
            self.scroll.y = y + h - height;
        }
        self.scroll = self.scroll.clamped();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ss_model::{Cell, StyleId};

    fn book_with_million_rows() -> Workbook {
        let mut book = Workbook::blank();
        let id = book.strings.intern("label");
        let sheet = book.sheet_mut(0).expect("sheet 0");
        // Data at both ends and in the middle, so no shortcut based on the used
        // range can accidentally make this fast.
        for row in [0u32, 1, 2, 500_000, 500_001, 1_048_575] {
            for col in 0..10 {
                sheet.set(
                    CellRef::new(row, col),
                    Cell {
                        value: if col % 2 == 0 {
                            CellValue::Number(row as f64)
                        } else {
                            CellValue::Text(id)
                        },
                        style: StyleId::DEFAULT,
                        formula: None,
                    },
                );
            }
        }
        book
    }

    fn viewport() -> egui::Rect {
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))
    }

    #[test]
    fn a_frame_touches_only_the_cells_it_can_see() {
        let book = book_with_million_rows();
        let sheet = book.sheet(0).expect("sheet 0");
        let layout = Layout::for_sheet(sheet, 1.0);

        // Scrolled to the very bottom of a million rows.
        let scroll = Scroll {
            x: 0.0,
            y: layout.rows.offset(1_048_536),
        };
        let plan = plan(&book, sheet, &layout, viewport(), scroll);
        assert_eq!(plan.rows.clone().count(), 40, "800 pixels of 20-pixel rows");
        assert!(plan.cols.clone().count() < 25);
        // Only the last row has anything in it down here.
        assert_eq!(plan.cells.len(), 10);
    }

    #[test]
    fn scrolling_a_million_rows_stays_flat() {
        // The exit criterion for this chunk, measured on the part that scales
        // with the sheet. A frame's worth of work at the top and at the bottom
        // of a million rows must cost the same; if anything walked the rows,
        // the second would be thousands of times slower.
        let book = book_with_million_rows();
        let sheet = book.sheet(0).expect("sheet 0");
        let layout = Layout::for_sheet(sheet, 1.0);

        let frames = 200;
        let time_at = |first_row: u32| {
            let scroll = Scroll {
                x: 0.0,
                y: layout.rows.offset(first_row),
            };
            let start = std::time::Instant::now();
            for _ in 0..frames {
                let p = plan(&book, sheet, &layout, viewport(), scroll);
                std::hint::black_box(&p);
            }
            start.elapsed()
        };

        let top = time_at(0);
        let bottom = time_at(1_048_540);
        // Generous: the point is orders of magnitude, not a benchmark. A debug
        // build with a walk in it would blow past this by a factor of a
        // thousand.
        assert!(
            bottom < top * 20 + std::time::Duration::from_millis(50),
            "a frame at the bottom of the sheet cost {bottom:?} against {top:?} at the top"
        );
        // And in absolute terms, a frame must fit inside a frame.
        let per_frame = bottom / frames;
        assert!(
            per_frame < std::time::Duration::from_millis(16),
            "one frame's cell planning took {per_frame:?}"
        );
    }

    #[test]
    fn a_merge_is_drawn_once_across_its_whole_rectangle() {
        let mut book = Workbook::blank();
        let sheet = book.sheet_mut(0).expect("sheet 0");
        sheet
            .merges
            .push(CellRange::new(CellRef::new(0, 0), CellRef::new(1, 2)));
        sheet.set(
            CellRef::new(0, 0),
            Cell {
                value: CellValue::Number(1.0),
                ..Cell::default()
            },
        );
        // A stray value in a covered cell must not produce a second draw.
        sheet.set(
            CellRef::new(1, 2),
            Cell {
                value: CellValue::Number(2.0),
                ..Cell::default()
            },
        );
        let sheet = book.sheet(0).expect("sheet 0");
        let layout = Layout::for_sheet(sheet, 1.0);
        let plan = plan(&book, sheet, &layout, viewport(), Scroll::default());
        assert_eq!(plan.cells.len(), 1);
        let cell = &plan.cells[0];
        assert_eq!(cell.rect.width(), layout.cols.size(0) as f32 * 3.0);
        assert_eq!(cell.rect.height(), layout.rows.size(0) as f32 * 2.0);
    }

    #[test]
    fn a_label_runs_into_empty_neighbours_but_not_into_full_ones() {
        let mut book = Workbook::blank();
        let id = book.strings.intern("a rather long label");
        let sheet = book.sheet_mut(0).expect("sheet 0");
        sheet.set(
            CellRef::new(0, 0),
            Cell {
                value: CellValue::Text(id),
                ..Cell::default()
            },
        );
        sheet.set(
            CellRef::new(1, 0),
            Cell {
                value: CellValue::Text(id),
                ..Cell::default()
            },
        );
        sheet.set(
            CellRef::new(1, 1),
            Cell {
                value: CellValue::Number(1.0),
                ..Cell::default()
            },
        );
        let sheet = book.sheet(0).expect("sheet 0");
        let layout = Layout::for_sheet(sheet, 1.0);
        let plan = plan(&book, sheet, &layout, viewport(), Scroll::default());
        let first = plan
            .cells
            .iter()
            .find(|c| c.rect.top() == 0.0)
            .expect("A1 drawn");
        assert!(first.overflow > 0.0, "A1 has empty cells to its right");
        let second = plan
            .cells
            .iter()
            .find(|c| c.rect.top() > 0.0 && c.rect.left() == 0.0)
            .expect("A2 drawn");
        assert_eq!(second.overflow, 0.0, "B2 is occupied");
    }

    #[test]
    fn a_date_is_drawn_as_a_date_and_not_as_its_serial() {
        // The whole reason styles are read: without the format table this cell
        // shows 45352.
        let mut book = Workbook::blank();
        book.styles = ss_model::StyleTable::build(&std::collections::BTreeMap::new(), &[0, 14]);
        let sheet = book.sheet_mut(0).expect("sheet 0");
        sheet.set(
            CellRef::new(0, 0),
            Cell {
                value: CellValue::Number(45352.0),
                style: StyleId(1),
                formula: None,
            },
        );
        let sheet = book.sheet(0).expect("sheet 0");
        let layout = Layout::for_sheet(sheet, 1.0);
        let plan = plan(&book, sheet, &layout, viewport(), Scroll::default());
        assert_eq!(plan.cells[0].text, "03-01-24");
        assert!(plan.cells[0].numeric, "a date sits right, like a number");
    }

    #[test]
    fn scrolling_into_view_moves_the_least_it_can() {
        let book = book_with_million_rows();
        let sheet = book.sheet(0).expect("sheet 0");
        let mut view = GridView::default();
        let body = egui::vec2(600.0, 400.0);

        // Already visible: nothing moves.
        view.scroll_into_view(CellRef::new(1, 1), body, sheet);
        assert_eq!(view.scroll, Scroll::default());

        // Below the fold: scroll just far enough to show it.
        view.scroll_into_view(CellRef::new(30, 0), body, sheet);
        assert_eq!(view.scroll.y, 31.0 * 20.0 - 400.0);
        assert_eq!(view.scroll.x, 0.0);

        // Back up: the top edge is what moves this time.
        view.scroll_into_view(CellRef::new(5, 0), body, sheet);
        assert_eq!(view.scroll.y, 5.0 * 20.0);
    }
}
