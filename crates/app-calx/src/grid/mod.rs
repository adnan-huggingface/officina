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
pub mod chart;
pub mod editor;
pub mod paint;
pub mod picture;
pub mod selection;

use ss_formula::cond::{Effect, Formatting, Overlay};
use ss_formula::edit::Geometry;
use ss_model::numfmt::FormatValue;
use ss_model::style::{BorderStyle, Edge, HAlign, VAlign};
use ss_model::{Axis, CellRange, CellRef, CellValue, Sheet, Workbook};
use ui_kit::egui;

pub use axis::Layout;
pub use editor::{Editor, Mode};
pub use selection::{Direction, Selection};

/// How close to a boundary the pointer has to be to grab it, either side.
///
/// Four pixels each way is eight in total, about twice what Excel gives you.
/// It does not want to be much more: a default row is twenty pixels tall, and
/// a zone any wider would leave less of the row header for selecting the row
/// than for resizing it.
const RESIZE_GRAB: f32 = 4.0;

/// The most columns a long label may overflow into.
///
/// Excel has no limit, but each one is a cell lookup on every frame, and a label
/// wider than this is unreadable anyway.
const MAX_OVERFLOW: u32 = 12;

/// One edge of a cell's border, resolved to something drawable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintEdge {
    pub style: BorderStyle,
    pub color: [u8; 3],
}

/// Where the four edges of a cell go. Indexed by [`LEFT`], [`RIGHT`],
/// [`TOP`], [`BOTTOM`].
pub type Edges = [Option<PaintEdge>; 4];

pub const LEFT: usize = 0;
pub const RIGHT: usize = 1;
pub const TOP: usize = 2;
pub const BOTTOM: usize = 3;

/// Everything about how a cell is drawn that came from its style, with the
/// theme colours already resolved and any conditional format merged in.
///
/// Resolved here rather than in the painter because this is the part that is
/// testable without a window, and because a conditional format is a *partial*
/// override — working out what actually wins is exactly the sort of thing that
/// should not be re-derived inside a paint loop.
#[derive(Debug, Clone, PartialEq)]
pub struct CellLook {
    pub fill: Option<[u8; 3]>,
    pub text: Option<[u8; 3]>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// In points, as the file stores it.
    pub size: f32,
    /// Which of the three installed families the cell's font name resolves to.
    pub family: ui_kit::Family,
    pub horizontal: HAlign,
    pub vertical: VAlign,
    pub wrap: bool,
    pub indent: u32,
    /// Excel's encoding: 0-90 anticlockwise, 91-180 for -1 to -90, 255 stacked.
    pub rotation: u32,
    pub edges: Edges,
}

impl Default for CellLook {
    fn default() -> Self {
        CellLook {
            fill: None,
            text: None,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            size: ss_model::style::DEFAULT_FONT_SIZE as f32,
            family: ui_kit::Family::Sans,
            horizontal: HAlign::General,
            vertical: VAlign::Bottom,
            wrap: false,
            indent: 0,
            rotation: 0,
            edges: [None; 4],
        }
    }
}

impl CellLook {
    /// True when the cell would look like any other empty cell.
    ///
    /// What decides whether a *valueless* cell is worth drawing at all. A blank
    /// cell with a fill or a border is content; a blank cell with a font size is
    /// not, because nothing shows.
    pub fn is_invisible(&self) -> bool {
        self.fill.is_none() && self.edges.iter().all(Option::is_none)
    }
}

/// One cell to draw.
pub struct PaintCell {
    pub rect: egui::Rect,
    pub text: String,
    pub color: Option<[u8; 3]>,
    /// Numbers sit right, text sits left.
    pub numeric: bool,
    /// Extra width borrowed from empty neighbours, for a label that overruns.
    pub overflow: f32,
    pub look: CellLook,
    /// A data bar or colour scale drawn behind the value.
    pub overlay: Option<Overlay>,
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
    conditional: &Formatting,
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

    // A sheet index is needed to ask the conditional formats about a cell, and
    // `Sheet` does not know its own position in the workbook.
    let index = book
        .sheets
        .iter()
        .position(|s| std::ptr::eq(s, sheet))
        .unwrap_or(0);

    for row in rows.clone() {
        for col in cols.clone() {
            let at = CellRef::new(row, col);
            // A cell covered by a merge is drawn by the merge's anchor.
            if sheet.merge_at(at).is_some_and(|m| m.start != at) {
                continue;
            }
            if let Some(cell) = plan_cell(book, sheet, layout, view, scroll, conditional, index, at)
            {
                cells.push(cell);
            }
        }
    }

    // A merge whose anchor is off-screen still has to be drawn. Its rectangle
    // reaches into this pane even though its top-left cell does not, and its
    // text is centred over the *whole* merge — which is how a banner across a
    // sheet is written. Left to the loop above, a heading merged from column A
    // vanishes the moment column A scrolls away, and on a sheet frozen at F4 it
    // vanishes always: the anchor is in the frozen pane and the centre of the
    // text is in the scrolling one.
    for merge in &sheet.merges {
        let anchored_here = rows.contains(&merge.start.row) && cols.contains(&merge.start.col);
        let reaches_here = merge.start.row <= *rows.end()
            && merge.end.row >= *rows.start()
            && merge.start.col <= *cols.end()
            && merge.end.col >= *cols.start();
        if anchored_here || !reaches_here {
            continue;
        }
        if let Some(cell) = plan_cell(
            book,
            sheet,
            layout,
            view,
            scroll,
            conditional,
            index,
            merge.start,
        ) {
            cells.push(cell);
        }
    }

    Plan { rows, cols, cells }
}

/// One cell, resolved to something drawable, or `None` when nothing shows.
///
/// `at` must be a merge's anchor or an unmerged cell; a covered cell is drawn
/// by whatever anchors it.
#[allow(clippy::too_many_arguments)]
fn plan_cell(
    book: &Workbook,
    sheet: &Sheet,
    layout: &Layout,
    view: egui::Rect,
    scroll: Scroll,
    conditional: &Formatting,
    index: usize,
    at: CellRef,
) -> Option<PaintCell> {
    let merge = sheet.merge_at(at);
    let effect = conditional.effect(book, index, at);
    let look = look_at(book, sheet, at, effect.as_ref());
    let cell = sheet.get(at);
    let blank = cell.is_none_or(|c| c.value.is_blank());

    // A cell with nothing in it is still drawn when it is shaded or bordered —
    // which is how a whole formatted-but-empty table looks like a table.
    if blank && look.is_invisible() && effect.is_none() {
        return None;
    }

    let shown = if blank {
        ss_model::Formatted::default()
    } else {
        let cell = cell.expect("not blank");
        let value = match cell.value {
            CellValue::Blank => FormatValue::Blank,
            CellValue::Number(n) => FormatValue::Number(n),
            CellValue::Bool(b) => FormatValue::Bool(b),
            CellValue::Error(e) => FormatValue::Error(e),
            CellValue::Text(id) => FormatValue::Text(book.strings.resolve(id)),
        };
        // A conditional format may replace the number format too, not only the
        // colours.
        match effect.as_ref().and_then(|e| e.dxf.number_format.as_deref()) {
            Some(code) => ss_model::NumberFormat::parse(code).format(value),
            None => book.styles.number_format(sheet.style_at(at)).format(value),
        }
    };
    let hidden = effect.as_ref().is_some_and(|e| e.hide_value);
    let text = if hidden { String::new() } else { shown.text };
    let overlay = effect.as_ref().and_then(|e| e.overlay);
    if text.is_empty() && look.is_invisible() && overlay.is_none() {
        return None;
    }

    let rect = match merge {
        Some(m) => rect_of_range(layout, *m, view, scroll),
        None => rect_of(layout, at, view, scroll),
    };
    // Text runs on into empty neighbours; a number never does — it turns into
    // `####` instead, which the painter decides once it can measure the glyphs.
    // Wrapped text stays in its own cell too.
    let overflow = if shown.numeric || merge.is_some() || look.wrap {
        0.0
    } else {
        overflow_width(sheet, at, layout)
    };
    let color = look.text.or(shown.color);
    Some(PaintCell {
        rect,
        text,
        color,
        numeric: shown.numeric,
        overflow,
        look,
        overlay,
    })
}

/// Works out what a cell actually looks like: the table it is in, then its own
/// style, the style its row or column carries, and then whatever a conditional
/// format overrides.
///
/// The precedence for colour is worth stating because all four can disagree:
/// a conditional format wins over the number format's own colour (the `[Red]`
/// in `0.00;[Red]-0.00`), which wins over the font's, which wins over the
/// table's. The table is *underneath* everything, which is what makes a table
/// style visible at all: the cells of a formatted table usually carry no style
/// of their own, and the ones that do have chosen to differ from it.
pub fn look_at(book: &Workbook, sheet: &Sheet, at: CellRef, effect: Option<&Effect>) -> CellLook {
    let styles = &book.styles;
    let theme = styles.theme();
    let style = sheet.style_at(at);
    let font = styles.font(style);
    let alignment = styles.alignment(style);
    let border = styles.border(style);

    let mut look = CellLook {
        fill: styles.fill(style).shade(theme),
        text: font.color.resolve(theme),
        bold: font.bold,
        italic: font.italic,
        underline: !font.underline.is_none(),
        strike: font.strike,
        size: font.size as f32,
        family: ui_kit::Family::of(&font.name),
        horizontal: alignment.horizontal,
        vertical: alignment.vertical,
        wrap: alignment.wrap,
        indent: alignment.indent,
        rotation: alignment.rotation,
        edges: [
            edge(border.left, theme),
            edge(border.right, theme),
            edge(border.top, theme),
            edge(border.bottom, theme),
        ],
    };

    // A table style shows through wherever the cell says nothing. Bold is a
    // union rather than an override: a heading's own font is very often plain
    // Arial 10, and letting `bold = false` win would erase the header row.
    if let Some(table) = table_look(book, sheet, at, theme) {
        if look.fill.is_none() {
            look.fill = table.fill.as_ref().and_then(|f| f.shade(theme));
        }
        if look.text.is_none() {
            look.text = table.color.and_then(|c| c.resolve(theme));
        }
        look.bold |= table.bold.unwrap_or(false);
        if let Some(border) = table.border {
            for (slot, edge_of) in [
                (LEFT, border.left),
                (RIGHT, border.right),
                (TOP, border.top),
                (BOTTOM, border.bottom),
            ] {
                if look.edges[slot].is_none() {
                    look.edges[slot] = edge(edge_of, theme);
                }
            }
        }
    }

    if let Some(effect) = effect {
        let dxf = &effect.dxf;
        if let Some(bold) = dxf.bold {
            look.bold = bold;
        }
        if let Some(italic) = dxf.italic {
            look.italic = italic;
        }
        if let Some(underline) = dxf.underline {
            look.underline = !underline.is_none();
        }
        if let Some(strike) = dxf.strike {
            look.strike = strike;
        }
        if let Some(color) = dxf.color {
            look.text = color.resolve(theme).or(look.text);
        }
        if let Some(fill) = &dxf.fill {
            look.fill = fill.shade(theme).or(look.fill);
        }
        if let Some(border) = dxf.border {
            for (slot, edge_of) in [
                (LEFT, border.left),
                (RIGHT, border.right),
                (TOP, border.top),
                (BOTTOM, border.bottom),
            ] {
                if let Some(drawn) = edge(edge_of, theme) {
                    look.edges[slot] = Some(drawn);
                }
            }
        }
    }
    look
}

/// What the table covering a cell, if any, says it should look like.
///
/// The `dxf` the *file* carries for the band wins over the built-in style's
/// own idea, because it is the one thing here that is not an approximation.
fn table_look(
    book: &Workbook,
    sheet: &Sheet,
    at: CellRef,
    theme: &ss_model::Theme,
) -> Option<ss_model::style::Dxf> {
    let table = sheet.tables.iter().find(|t| t.contains(at))?;
    let mut look = table.look(at).unwrap_or_default();

    let id = match table.band_at(at)? {
        ss_model::Band::Header => table.header_dxf,
        ss_model::Band::Totals => table.totals_dxf,
        ss_model::Band::Body { .. } => table.data_dxf,
    };
    if let Some(over) = id.and_then(|id| book.styles.dxf(id)) {
        // A dxf attribute is present or it is absent — but `<color auto="1"/>`
        // is *present and says nothing*, and so is a fill with no pattern.
        // Excel writes exactly that on a table's header dxf, and taking it as
        // an override paints the white headings of a black header row in
        // "automatic", which is black on black.
        if over.bold.is_some() {
            look.bold = over.bold;
        }
        if over.italic.is_some() {
            look.italic = over.italic;
        }
        if over.color.is_some_and(|c| c.resolve(theme).is_some()) {
            look.color = over.color;
        }
        if over.fill.as_ref().is_some_and(|f| f.shade(theme).is_some()) {
            look.fill = over.fill.clone();
        }
        if over.border.is_some() {
            look.border = over.border;
        }
    }
    (!look.is_empty()).then_some(look)
}

fn edge(edge: Edge, theme: &ss_model::Theme) -> Option<PaintEdge> {
    if edge.is_none() {
        return None;
    }
    Some(PaintEdge {
        style: edge.style,
        // An unset border colour is the window's foreground, which at cell size
        // reads as the grid's own text colour. Mid-grey is close enough in both
        // a light and a dark theme, and is what Excel's "automatic" resolves to.
        color: edge.color.resolve(theme).unwrap_or([0x40, 0x40, 0x40]),
    })
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
    /// Ctrl+Enter: the entry goes to every cell of the selection at once,
    /// relative references travelling as a copy would carry them.
    CommitAll {
        at: CellRef,
        text: String,
    },
    /// The selection dragged by its border to a new top-left. A move by
    /// default; `copy` when Ctrl was held at the drop, as Excel reads it.
    MoveRange {
        from: CellRange,
        to: CellRef,
        copy: bool,
    },
    /// Enter with the marching ants up: complete the paste from the
    /// application's clipboard and dismiss them.
    PasteClip,
    /// Escape with the ants up: dismiss them, and forget any pending cut.
    CancelClipboard,
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
    /// The fill handle was dragged from `from` out to `to`. `toggle` is
    /// Ctrl, which asks the series for the opposite of its default: a lone
    /// number counts instead of copying, a run copies instead of counting.
    Fill {
        from: CellRange,
        to: CellRange,
        toggle: bool,
    },
    /// A row or column resize finished; the payload is how the sheet looked
    /// before it started, which is the whole undo entry.
    Resized(Geometry),
    /// Formatting applied to the selection.
    Format(Format),
    /// Ctrl+PageDown and Ctrl+PageUp: the next or previous visible sheet.
    StepSheet(i32),
    /// Merge the selection into one cell, or take a merge apart.
    Merge(bool),
    /// Freeze above and left of the cursor, or unfreeze.
    Freeze(bool),
    /// Split the sheet at the cursor into panes that scroll on their own, or
    /// put it back together.
    Split(bool),
    /// Hide or show the selected rows or columns.
    Visibility {
        axis: Axis,
        hide: bool,
    },
    /// Deepen or shallow the outline level of the selected rows or columns.
    Group {
        axis: Axis,
        ungroup: bool,
    },
    /// A collapse button in the outline margin: `index` is the summary row
    /// or column just past the group it controls.
    ToggleOutline {
        axis: Axis,
        index: u32,
    },
    /// Size the selected columns to their contents.
    AutoFit(Axis),
    /// A band of rows or columns dragged somewhere else. `before` names the
    /// index it comes to sit in front of, counted in the grid as it is now.
    MoveBand {
        axis: Axis,
        first: u32,
        last: u32,
        before: u32,
    },
    /// A header boundary was double-clicked: fit the one row or column before
    /// it, whatever happens to be selected elsewhere.
    AutoFitAt {
        axis: Axis,
        index: u32,
    },
    /// A picture was moved or resized. The payload is the sheet's pictures as
    /// they were before the drag: the model already holds the new geometry,
    /// because a drag has to be shown while it happens.
    PicturesMoved(Vec<ss_model::Picture>),
    /// Delete pressed with a picture selected.
    DeletePicture(usize),
    /// A chart was moved or resized; the payload is the charts as they were.
    ChartsMoved(Vec<ss_model::Chart>),
    /// Delete pressed with a chart selected.
    DeleteChart(usize),
    /// A filter arrow on the header row was clicked. The payload is the column
    /// as an offset into the filter's range, which is what `colId` means.
    FilterMenu(u32),
}

/// A formatting command, as the toolbar and the keyboard both produce it.
///
/// The toggles carry no value because they are toggles *of the selection*: what
/// Ctrl+B does depends on whether the cursor's cell is already bold, which the
/// grid does not decide — the application does, when it has the workbook.
#[derive(Debug, Clone, PartialEq)]
pub enum Format {
    Bold,
    Italic,
    Underline,
    Strike,
    Align(HAlign),
    Vertical(VAlign),
    Wrap,
    /// Positive to indent, negative to outdent.
    Indent(i32),
    FontSize(f64),
    FontName(String),
    /// `None` clears back to automatic.
    TextColor(Option<ss_model::Color>),
    Fill(Option<ss_model::Color>),
    Border(BorderPreset),
    NumberFormat(String),
    /// Everything at once, as the Format Cells dialog answers.
    ///
    /// A whole look rather than a list of deltas, because that is what the
    /// dialog is: five tabs of one cell's formatting, edited together and
    /// pressed OK. Excel applies all of it to the whole selection too, which
    /// is why a mixed selection comes out uniform.
    ///
    /// Boxed because a `Look` carries five structs and a string, and every
    /// other command in this enum would otherwise be as large as the largest.
    Whole(Box<ss_model::style::Look>),
    /// Back to the workbook default, keeping the value.
    Clear,
}

/// The border buttons, which set several edges at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderPreset {
    None,
    All,
    Outline,
    Bottom,
    Top,
    Left,
    Right,
    Thick,
}

impl BorderPreset {
    /// Applies the preset to a border, leaving edges it does not name alone.
    ///
    /// `All` and `Outline` differ only in a rectangle: a single cell gets the
    /// same four edges from either, which is why the grid sends the preset and
    /// not a computed set of edges.
    pub fn apply(self, border: &mut ss_model::Border) {
        use ss_model::style::{BorderStyle, Edge};
        let thin = Edge::new(BorderStyle::Thin);
        let thick = Edge::new(BorderStyle::Thick);
        match self {
            BorderPreset::None => *border = ss_model::Border::default(),
            BorderPreset::All | BorderPreset::Outline => {
                *border = ss_model::Border {
                    left: thin,
                    right: thin,
                    top: thin,
                    bottom: thin,
                    ..*border
                }
            }
            BorderPreset::Thick => {
                *border = ss_model::Border {
                    left: thick,
                    right: thick,
                    top: thick,
                    bottom: thick,
                    ..*border
                }
            }
            BorderPreset::Bottom => border.bottom = thin,
            BorderPreset::Top => border.top = thin,
            BorderPreset::Left => border.left = thin,
            BorderPreset::Right => border.right = thin,
        }
    }
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

/// Sum, count, and the rest, over whatever is selected.
///
/// Driven from the sheet's *stored* cells rather than from the selected
/// addresses, because selecting a column selects 1,048,576 of them and only a
/// few hundred exist. The cost is the size of the sheet, not of the selection.
pub fn summarize(book: &Workbook, sheet: usize, ranges: &[CellRange]) -> Summary {
    let mut summary = Summary::default();
    let Some(sheet) = book.sheet(sheet) else {
        return summary;
    };
    let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
    for (at, cell) in sheet.cells.iter() {
        if cell.value.is_blank() || !ranges.iter().any(|r| r.contains(at)) {
            continue;
        }
        summary.count += 1;
        if let CellValue::Number(n) = cell.value {
            summary.numeric += 1;
            summary.sum += n;
            min = min.min(n);
            max = max.max(n);
        }
    }
    if summary.numeric > 0 {
        summary.min = min;
        summary.max = max;
    }
    summary
}

/// Where a sheet is divided into panes, as a cell — `A1` when it is not.
///
/// The one place the model's `Option<Panes>` is flattened, because everything
/// that lays panes out wants a corner to measure from and treats "no division"
/// as a division at the sheet's own corner.
pub(crate) fn division(sheet: &ss_model::Sheet) -> CellRef {
    sheet.panes.map_or(CellRef::new(0, 0), |p| p.at)
}

/// Whether the sheet's division pins what is above and left of it.
pub(crate) fn pins(sheet: &ss_model::Sheet) -> bool {
    sheet.panes.is_none_or(|p| p.frozen)
}

/// The grid widget's own state, kept between frames.
pub struct GridView {
    pub sheet_index: usize,
    pub selection: Selection,
    /// Scroll position in sheet pixels, for the scrolling pane.
    pub scroll: Scroll,
    /// Where the *other* panes of a split sheet are scrolled to.
    ///
    /// Zero on a sheet that is frozen or undivided, and that is the whole
    /// difference between the two: a freeze pins its bands, a split lets them
    /// move on their own, which is how two distant parts of one sheet are read
    /// beside each other.
    pub(crate) pinned: Scroll,
    pub zoom: f64,
    /// The open cell editor, if any. The formula bar edits the same buffer.
    pub editor: Option<Editor>,
    /// What the last frame asked the application to do. Drained by the caller.
    pub actions: Vec<Action>,
    /// Rebuilt only when the sheet, the zoom, or a size changes — building it
    /// per frame would sort every row height on a sheet that has many.
    pub(crate) layout: Option<(usize, u32, Layout)>,
    /// The sheet's conditional formats, prepared. Cached on the same key as the
    /// layout because preparing one scans every region it covers, and a colour
    /// scale's answer for one cell depends on all of them.
    pub(crate) conditional: Option<(usize, u32, Formatting)>,
    /// Bumped whenever a row or column is resized, to invalidate the cache.
    pub(crate) generation: u32,
    /// The chart the last click landed on, if any. Charts float above the
    /// cells, so a click on one is not a click on the cell underneath.
    pub selected_chart: Option<usize>,
    /// The picture the last click landed on. At most one of this and
    /// `selected_chart` is ever set: they are the same kind of selection, and
    /// showing handles on two things at once would say the wrong thing about
    /// what Delete would remove.
    pub selected_picture: Option<usize>,
    /// How the sheet's pictures looked before the drag in progress started,
    /// which is the whole undo entry. Same shape as `before_resize`.
    pub(crate) before_pictures: Option<Vec<ss_model::Picture>>,
    /// And the charts, for a chart drag.
    pub(crate) before_charts: Option<Vec<ss_model::Chart>>,
    pub(crate) drag: Option<Drag>,
    /// How the sheet looked before the resize drag in progress started.
    pub(crate) before_resize: Option<Geometry>,
    /// The rectangle the fill drag currently covers.
    pub(crate) fill_target: Option<CellRange>,
    /// Where a range dragged by its border would land, drawn as an outline.
    pub(crate) move_range_target: Option<CellRange>,
    /// Where a band being dragged would land: the index it would sit in front
    /// of. Drawn as a line between two rows or columns, because that is what a
    /// drop between them is.
    pub(crate) move_target: Option<u32>,
    /// Where the scrollbars were drawn last frame, and how far they reach.
    /// Recomputed every frame; kept so input can hit-test them.
    pub(crate) bars: paint::Bars,
    /// What the status bar says about the selection, and what it was computed
    /// from — a whole-column selection is a million addresses, so this is
    /// recomputed when the selection changes rather than every frame.
    pub summary: Summary,
    pub(crate) summarized: Option<(usize, u32, Vec<CellRange>)>,
    /// Pictures decoded and uploaded, by package part. Not part of the
    /// document: a cache of what the document's bytes mean on this GPU.
    pub(crate) textures: picture::Textures,
    /// The marching ants: the copied or cut range, on the sheet it lives on.
    /// Set by the application on copy, dismissed by Escape, any edit, or the
    /// paste that spends a cut.
    pub marquee: Option<(usize, CellRange)>,
    /// Where the open editor was drawn last frame, which is not its cell: it
    /// grows over the neighbours to hold what is being typed. Kept so that a
    /// click in the part hanging over column D counts as a click in the
    /// editor rather than a click on D.
    pub(crate) editor_box: Option<egui::Rect>,
}

/// What the status bar says about the selection.
///
/// Excel puts this at the bottom right and it is the most-used feature of the
/// whole status bar: select a column of numbers and the sum is *there*, with no
/// formula written and nothing changed in the document.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Summary {
    /// Cells holding anything at all, which is what Excel calls Count.
    pub count: usize,
    /// Cells holding a number, which is what the other four are over.
    pub numeric: usize,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
}

impl Summary {
    pub fn average(&self) -> Option<f64> {
        (self.numeric > 0).then(|| self.sum / self.numeric as f64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Drag {
    /// Sweeping a selection out from the anchor.
    Select,
    /// Sweeping whole rows or columns out along a header.
    ///
    /// Its own kind rather than a `Select` that happens to have started in a
    /// header, because the two extend differently: this one grows a band of
    /// entire rows or columns, and `Select` grows a rectangle towards whatever
    /// cell is under the pointer. Routing a header sweep through the second
    /// collapsed the band to a single cell the moment the pointer moved.
    SelectHeaders {
        axis: Axis,
        anchor: u32,
    },
    /// Dragging a band of selected rows or columns somewhere else.
    ///
    /// Started from a header that is already inside the selection, which is
    /// what tells a move apart from the sweep that would otherwise begin
    /// there: a press on a header nobody selected can only mean "select this".
    MoveBand {
        axis: Axis,
        first: u32,
        last: u32,
    },
    /// Dragging a split bar to divide the sheet somewhere else.
    MoveSplit {
        axis: Axis,
    },
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
    /// Dragging a selected picture around the sheet. `grab` is where in the
    /// picture the pointer took hold, so it does not jump to the corner.
    MovePicture {
        index: usize,
        grab: egui::Vec2,
    },
    /// Dragging one of a selected picture's eight handles. `start` is the
    /// picture's rectangle in sheet space when the drag began — the edges this
    /// handle does not move come from there, unchanged by anything since.
    ResizePicture {
        index: usize,
        handle: picture::Handle,
        start: egui::Rect,
    },
    /// Dragging a selected chart around the sheet, exactly as a picture.
    MoveChart {
        index: usize,
        grab: egui::Vec2,
    },
    /// Dragging one of a selected chart's eight handles.
    ResizeChart {
        index: usize,
        handle: picture::Handle,
        start: egui::Rect,
    },
    /// Dragging the small square at the corner of the selection.
    Fill {
        from: CellRange,
    },
    /// Sweeping a range *into an open formula*: the press named a cell in
    /// point mode, and dragging stretches that reference instead of moving
    /// the selection.
    Point,
    /// The selection picked up by its border and carried. `grab` is which
    /// cell of the block the pointer took hold of, as offsets from its
    /// top-left, so the block does not jump to put its corner under the
    /// cursor.
    MoveRange {
        from: CellRange,
        grab: (i64, i64),
    },
    /// Dragging a scrollbar thumb. The payload is how far into the thumb the
    /// pointer took hold, so the thumb does not jump to centre itself under
    /// the cursor the moment it is grabbed.
    ScrollThumb {
        axis: Axis,
        grab: f32,
    },
}

impl Default for GridView {
    fn default() -> Self {
        GridView {
            sheet_index: 0,
            selection: Selection::default(),
            scroll: Scroll::default(),
            pinned: Scroll::default(),
            zoom: 1.0,
            editor: None,
            actions: Vec::new(),
            layout: None,
            conditional: None,
            generation: 0,
            selected_chart: None,
            selected_picture: None,
            before_pictures: None,
            before_charts: None,
            drag: None,
            before_resize: None,
            fill_target: None,
            move_range_target: None,
            move_target: None,
            bars: paint::Bars::default(),
            summary: Summary::default(),
            summarized: None,
            textures: picture::Textures::default(),
            marquee: None,
            editor_box: None,
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
        self.summarized = None;
    }

    /// Shows a different sheet, the way the file says it should be shown.
    ///
    /// A sheet carries its own zoom and its own idea of where it was scrolled
    /// to, so switching tabs is not just changing an index: coming back to a
    /// sheet you left at 90% halfway down should not put you at 100% at A1.
    pub fn open_sheet(&mut self, book: &Workbook, index: usize) {
        let Some(sheet) = book.sheet(index) else {
            return;
        };
        self.editor = None;
        self.selected_chart = None;
        self.sheet_index = index;
        self.zoom = sheet.view.zoom.clamp(0.25, 4.0);
        self.selection = Selection::at(sheet.view.selection.unwrap_or(CellRef::new(0, 0)));

        // `topLeftCell` names the first cell of the *scrolling* pane, and the
        // scroll offset is measured from the frozen split rather than from A1 —
        // so on a sheet frozen at F4, `topLeftCell="F4"` means "scrolled
        // nowhere". Not subtracting the split scrolls the sheet by the height
        // of everything above the freeze, twice over.
        let layout = Layout::for_sheet(book, sheet, self.zoom);
        let frozen = division(sheet);
        self.pinned = Scroll::default();
        self.scroll = match sheet.view.top_left {
            Some(at) => Scroll {
                x: layout.cols.offset(at.col) - layout.cols.offset(frozen.col),
                y: layout.rows.offset(at.row) - layout.rows.offset(frozen.row),
            },
            None => Scroll::default(),
        }
        .clamped();
        self.invalidate();
    }

    /// Keeps [`summary`](Self::summary) in step with the selection.
    pub(crate) fn ensure_summary(&mut self, book: &Workbook) {
        let key = (
            self.sheet_index,
            self.generation,
            self.selection.ranges().to_vec(),
        );
        if self.summarized.as_ref() == Some(&key) {
            return;
        }
        self.summary = summarize(book, self.sheet_index, self.selection.ranges());
        self.summarized = Some(key);
    }

    pub fn set_zoom(&mut self, zoom: f64) {
        let zoom = zoom.clamp(0.25, 4.0);
        if zoom != self.zoom {
            self.zoom = zoom;
            self.invalidate();
        }
    }

    /// Scrolls the least distance that brings `at` fully into view.
    pub fn scroll_into_view(
        &mut self,
        at: CellRef,
        body: egui::Vec2,
        book: &Workbook,
        sheet: &Sheet,
    ) {
        let layout = Layout::for_sheet(book, sheet, self.zoom);
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

    /// A banner merged across the whole width, anchored in column A.
    fn book_with_a_wide_merge() -> Workbook {
        let mut book = Workbook::blank();
        let text = book.strings.intern("b0000 0000 0000 (GROUP 0, 128 msgs)");
        let sheet = book.sheet_mut(0).expect("sheet 0");
        sheet.set(
            CellRef::new(3, 0),
            Cell {
                value: CellValue::Text(text),
                ..Cell::default()
            },
        );
        sheet
            .merges
            .push(CellRange::new(CellRef::new(3, 0), CellRef::new(3, 94)));
        book
    }

    #[test]
    fn a_merge_reaching_into_a_pane_is_drawn_there_even_though_it_starts_outside() {
        // The reference workbook is frozen at F4, so the banner across row 4 is
        // anchored in the frozen pane while the centre of its text is in the
        // scrolling one. Drawing it only where its anchor is means drawing it
        // nowhere: the fill is clipped to five columns and the text, centred
        // over ninety-five, lands outside the clip entirely.
        let book = book_with_a_wide_merge();
        let sheet = book.sheet(0).expect("sheet 0");
        let layout = Layout::for_sheet(&book, sheet, 1.0);

        // A pane that starts at column F: the anchor is not in it.
        let scroll = Scroll {
            x: layout.cols.offset(5),
            y: 0.0,
        };
        let plan = plan(
            &book,
            sheet,
            &layout,
            viewport(),
            scroll,
            &Formatting::empty(),
        );
        let banner = plan
            .cells
            .iter()
            .find(|c| c.text.starts_with("b0000"))
            .expect("the banner is drawn in a pane it reaches into");
        // And it keeps the whole merge's rectangle, which is what centres the
        // text where Excel centres it — off to the left of this pane.
        assert!(banner.rect.left() < viewport().left(), "{:?}", banner.rect);
        assert!(
            banner.rect.width() > viewport().width(),
            "{:?}",
            banner.rect
        );
    }

    #[test]
    fn a_merge_is_not_drawn_twice_in_the_pane_that_holds_its_anchor() {
        let book = book_with_a_wide_merge();
        let sheet = book.sheet(0).expect("sheet 0");
        let layout = Layout::for_sheet(&book, sheet, 1.0);
        let plan = plan(
            &book,
            sheet,
            &layout,
            viewport(),
            Scroll::default(),
            &Formatting::empty(),
        );
        assert_eq!(
            plan.cells
                .iter()
                .filter(|c| c.text.starts_with("b0000"))
                .count(),
            1
        );
    }

    #[test]
    fn a_merge_nowhere_near_the_viewport_is_not_drawn() {
        let book = book_with_a_wide_merge();
        let sheet = book.sheet(0).expect("sheet 0");
        let layout = Layout::for_sheet(&book, sheet, 1.0);
        let scroll = Scroll {
            x: 0.0,
            y: layout.rows.offset(400),
        };
        let plan = plan(
            &book,
            sheet,
            &layout,
            viewport(),
            scroll,
            &Formatting::empty(),
        );
        assert!(plan.cells.iter().all(|c| !c.text.starts_with("b0000")));
    }

    /// The CRC calculator sheet in miniature: a table whose cells carry no
    /// style at all, and a style name that decides how all of it looks.
    fn book_with_a_styled_table() -> Workbook {
        let mut book = Workbook::blank();
        let heading = book.strings.intern("Fix Nibble");
        let sheet = book.sheet_mut(0).expect("sheet 0");
        sheet.set(
            CellRef::new(6, 4),
            Cell {
                value: CellValue::Text(heading),
                ..Cell::default()
            },
        );
        sheet.set(
            CellRef::new(7, 4),
            Cell {
                value: CellValue::Number(3.0),
                ..Cell::default()
            },
        );
        sheet.tables.push(ss_model::Table {
            part: "/xl/tables/table1.xml".into(),
            name: "Table1".into(),
            range: CellRange::new(CellRef::new(6, 4), CellRef::new(7, 12)),
            header_rows: 1,
            totals_rows: 0,
            style: ss_model::TableStyle {
                name: Some("TableStyleMedium15".into()),
                row_stripes: true,
                ..ss_model::TableStyle::default()
            },
            header_dxf: None,
            data_dxf: None,
            totals_dxf: None,
        });
        book
    }

    #[test]
    fn a_table_style_reaches_cells_that_carry_no_style_at_all() {
        // Everything visible about this table is a name in tables/table1.xml.
        // The cells themselves say nothing, so a reader that only looks at
        // styles.xml draws bare text on white.
        let book = book_with_a_styled_table();
        let sheet = book.sheet(0).expect("sheet 0");
        let look = look_at(&book, sheet, CellRef::new(6, 4), None);
        assert_eq!(look.fill, Some([0x00, 0x00, 0x00]), "the header is black");
        assert_eq!(look.text, Some([0xFF, 0xFF, 0xFF]), "and its text white");
        assert!(look.bold, "a plain Arial heading is still bold in a table");

        let body = look_at(&book, sheet, CellRef::new(7, 4), None);
        assert_eq!(body.fill, Some([0xD9, 0xD9, 0xD9]));
        assert!(!body.bold);
    }

    #[test]
    fn a_dxf_that_says_automatic_is_not_an_override() {
        // Excel writes `<dxf><font>…<color auto="1"/>…</font></dxf>` as a
        // table's header dxf. It is present and it says nothing. Taken as an
        // override it paints the white headings of a black header row in
        // "automatic", which on black is nothing at all.
        let mut book = book_with_a_styled_table();
        let id = book.styles.add_dxf(ss_model::style::Dxf {
            color: Some(ss_model::Color::Auto),
            ..ss_model::style::Dxf::default()
        });
        book.sheet_mut(0).expect("sheet 0").tables[0].header_dxf = Some(id);

        let sheet = book.sheet(0).expect("sheet 0");
        let look = look_at(&book, sheet, CellRef::new(6, 4), None);
        assert_eq!(look.text, Some([0xFF, 0xFF, 0xFF]));
    }

    #[test]
    fn a_dxf_that_does_say_something_wins_over_the_built_in_style() {
        // The one part of a table's appearance the file itself carries.
        let mut book = book_with_a_styled_table();
        let id = book.styles.add_dxf(ss_model::style::Dxf {
            color: Some(ss_model::Color::rgb(0xFF, 0x00, 0x00)),
            ..ss_model::style::Dxf::default()
        });
        book.sheet_mut(0).expect("sheet 0").tables[0].header_dxf = Some(id);

        let sheet = book.sheet(0).expect("sheet 0");
        let look = look_at(&book, sheet, CellRef::new(6, 4), None);
        assert_eq!(look.text, Some([0xFF, 0x00, 0x00]));
    }

    #[test]
    fn a_cells_own_style_wins_over_the_table_it_is_in() {
        let mut book = book_with_a_styled_table();
        let yellow = book.styles.restyle(StyleId::DEFAULT, |look| {
            look.fill = ss_model::Fill::solid(ss_model::Color::rgb(0xFF, 0xEB, 0x9C))
        });
        let sheet = book.sheet_mut(0).expect("sheet 0");
        sheet.set(
            CellRef::new(7, 4),
            Cell {
                value: CellValue::Number(3.0),
                style: yellow,
                ..Cell::default()
            },
        );
        let sheet = book.sheet(0).expect("sheet 0");
        let look = look_at(&book, sheet, CellRef::new(7, 4), None);
        assert_eq!(
            look.fill,
            Some([0xFF, 0xEB, 0x9C]),
            "a cell that has chosen a fill has chosen to differ from the table"
        );
    }

    #[test]
    fn an_empty_cell_inside_a_table_is_still_drawn() {
        // The nine header cells and nine body cells of the reference table are
        // mostly empty. Skipping them leaves a shaded band with holes in it.
        let book = book_with_a_styled_table();
        let sheet = book.sheet(0).expect("sheet 0");
        let layout = Layout::for_sheet(&book, sheet, 1.0);
        let plan = plan(
            &book,
            sheet,
            &layout,
            viewport(),
            Scroll::default(),
            &Formatting::empty(),
        );
        let filled = plan.cells.iter().filter(|c| c.look.fill.is_some()).count();
        assert_eq!(filled, 18, "nine header cells and nine body cells");
    }

    #[test]
    fn a_shaded_empty_cell_is_drawn_and_an_unshaded_one_is_not() {
        // A blank cell with a fill is content: it is how a formatted-but-empty
        // table looks like a table. A blank cell with only a font size is not.
        let mut book = Workbook::blank();
        let shaded = book.styles.restyle(StyleId::DEFAULT, |look| {
            look.fill = ss_model::Fill::solid(ss_model::Color::rgb(0xFF, 0xEB, 0x9C))
        });
        let bigger = book
            .styles
            .restyle(StyleId::DEFAULT, |look| look.font.size = 24.0);
        let sheet = book.sheet_mut(0).expect("sheet 0");
        sheet.set(
            CellRef::new(0, 0),
            Cell {
                style: shaded,
                ..Cell::default()
            },
        );
        sheet.set(
            CellRef::new(0, 1),
            Cell {
                style: bigger,
                ..Cell::default()
            },
        );

        let sheet = book.sheet(0).expect("sheet 0");
        let layout = Layout::for_sheet(&book, sheet, 1.0);
        let plan = plan(
            &book,
            sheet,
            &layout,
            viewport(),
            Scroll::default(),
            &Formatting::empty(),
        );
        assert_eq!(plan.cells.len(), 1);
        assert_eq!(plan.cells[0].look.fill, Some([0xFF, 0xEB, 0x9C]));
        assert!(plan.cells[0].text.is_empty());
    }

    #[test]
    fn a_column_style_reaches_a_cell_that_has_none_of_its_own() {
        let mut book = Workbook::blank();
        let bold = book
            .styles
            .restyle(StyleId::DEFAULT, |look| look.font.bold = true);
        let sheet = book.sheet_mut(0).expect("sheet 0");
        sheet.column_styles.insert(0, bold);
        sheet.set(
            CellRef::new(0, 0),
            Cell {
                value: CellValue::Number(1.0),
                ..Cell::default()
            },
        );

        let sheet = book.sheet(0).expect("sheet 0");
        let layout = Layout::for_sheet(&book, sheet, 1.0);
        let plan = plan(
            &book,
            sheet,
            &layout,
            viewport(),
            Scroll::default(),
            &Formatting::empty(),
        );
        assert!(plan.cells[0].look.bold);
    }

    #[test]
    fn a_conditional_format_wins_over_the_cells_own_look() {
        use ss_model::cond::{CfKind, CfOperator, CfRule, ConditionalFormat};

        let mut book = Workbook::blank();
        book.styles = ss_model::StyleTable::from_parts(ss_model::style::Parts {
            cell_xfs: vec![ss_model::CellFormat::default()],
            dxfs: vec![ss_model::Dxf {
                bold: Some(true),
                fill: Some(ss_model::Fill::solid(ss_model::Color::rgb(
                    0xFF, 0xC7, 0xCE,
                ))),
                ..Default::default()
            }],
            ..Default::default()
        });
        let plain = book.styles.restyle(StyleId::DEFAULT, |look| {
            look.fill = ss_model::Fill::solid(ss_model::Color::rgb(0xEE, 0xEE, 0xEE))
        });
        let sheet = book.sheet_mut(0).expect("sheet 0");
        sheet.set(
            CellRef::new(0, 0),
            Cell {
                value: CellValue::Number(99.0),
                style: plain,
                ..Cell::default()
            },
        );
        sheet.conditional_formats.push(ConditionalFormat {
            ranges: vec![CellRange::new(CellRef::new(0, 0), CellRef::new(0, 0))],
            rules: vec![CfRule {
                kind: CfKind::CellIs {
                    operator: CfOperator::GreaterThan,
                    formulas: vec!["10".to_string()],
                },
                dxf: Some(0),
                priority: 1,
                stop_if_true: false,
            }],
        });

        let conditional = Formatting::prepare(&book, 0);
        let sheet = book.sheet(0).expect("sheet 0");
        let layout = Layout::for_sheet(&book, sheet, 1.0);
        let plan = plan(
            &book,
            sheet,
            &layout,
            viewport(),
            Scroll::default(),
            &conditional,
        );
        assert!(plan.cells[0].look.bold, "the rule's dxf applied");
        assert_eq!(
            plan.cells[0].look.fill,
            Some([0xFF, 0xC7, 0xCE]),
            "and its fill won over the cell's own"
        );
    }

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
        let layout = Layout::for_sheet(&book, sheet, 1.0);

        // Scrolled to the very bottom of a million rows.
        let scroll = Scroll {
            x: 0.0,
            y: layout.rows.offset(1_048_536),
        };
        let plan = plan(
            &book,
            sheet,
            &layout,
            viewport(),
            scroll,
            &Formatting::empty(),
        );
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
        let layout = Layout::for_sheet(&book, sheet, 1.0);

        let frames = 200;
        let time_at = |first_row: u32| {
            let scroll = Scroll {
                x: 0.0,
                y: layout.rows.offset(first_row),
            };
            let start = std::time::Instant::now();
            for _ in 0..frames {
                let p = plan(
                    &book,
                    sheet,
                    &layout,
                    viewport(),
                    scroll,
                    &Formatting::empty(),
                );
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
        let layout = Layout::for_sheet(&book, sheet, 1.0);
        let plan = plan(
            &book,
            sheet,
            &layout,
            viewport(),
            Scroll::default(),
            &Formatting::empty(),
        );
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
        let layout = Layout::for_sheet(&book, sheet, 1.0);
        let plan = plan(
            &book,
            sheet,
            &layout,
            viewport(),
            Scroll::default(),
            &Formatting::empty(),
        );
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
        let layout = Layout::for_sheet(&book, sheet, 1.0);
        let plan = plan(
            &book,
            sheet,
            &layout,
            viewport(),
            Scroll::default(),
            &Formatting::empty(),
        );
        assert_eq!(plan.cells[0].text, "03-01-24");
        assert!(plan.cells[0].numeric, "a date sits right, like a number");
    }

    #[test]
    fn opening_a_frozen_sheet_at_its_own_top_left_scrolls_nowhere() {
        // `topLeftCell` is measured from A1 and the scroll offset from the
        // frozen split. Taking one for the other opens the sheet scrolled past
        // everything the freeze was there to keep on screen.
        let mut book = Workbook::blank();
        let sheet = book.sheet_mut(0).expect("sheet 0");
        sheet.panes = Some(ss_model::Panes::frozen(CellRef::new(3, 5)));
        sheet.view.top_left = Some(CellRef::new(3, 5));
        sheet.view.selection = Some(CellRef::new(4, 1));

        let mut view = GridView::default();
        view.open_sheet(&book, 0);
        assert_eq!(view.scroll, Scroll::default());
        assert_eq!(view.selection.cursor(), CellRef::new(4, 1));

        // And a sheet genuinely scrolled two rows past the split says so.
        let sheet = book.sheet_mut(0).expect("sheet 0");
        sheet.view.top_left = Some(CellRef::new(5, 5));
        view.open_sheet(&book, 0);
        let layout = Layout::for_sheet(&book, book.sheet(0).expect("sheet 0"), 1.0);
        assert_eq!(view.scroll.y, layout.rows.size(4) + layout.rows.size(3));
        assert_eq!(view.scroll.x, 0.0);
    }

    #[test]
    fn scrolling_into_view_moves_the_least_it_can() {
        let book = book_with_million_rows();
        let sheet = book.sheet(0).expect("sheet 0");
        let mut view = GridView::default();
        let body = egui::vec2(600.0, 400.0);

        // Already visible: nothing moves.
        view.scroll_into_view(CellRef::new(1, 1), body, &book, sheet);
        assert_eq!(view.scroll, Scroll::default());

        // Below the fold: scroll just far enough to show it.
        view.scroll_into_view(CellRef::new(30, 0), body, &book, sheet);
        assert_eq!(view.scroll.y, 31.0 * 20.0 - 400.0);
        assert_eq!(view.scroll.x, 0.0);

        // Back up: the top edge is what moves this time.
        view.scroll_into_view(CellRef::new(5, 0), body, &book, sheet);
        assert_eq!(view.scroll.y, 5.0 * 20.0);
    }
}
