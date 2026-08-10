//! Where rows and columns sit, and which of them a viewport can see.
//!
//! A sheet has 1,048,576 rows. Nothing may ever be O(rows): not finding the row
//! under the mouse, not deciding what to paint, not scrolling. Everything here
//! is O(log n) or better, and the tests hold it to that by working on a full
//! million-row axis rather than a convenient small one.
//!
//! Positions are `f64`, not `f32`. A million rows of twenty pixels is a
//! twenty-million-pixel axis, and above sixteen million an `f32` can only count
//! in twos — the row under the mouse would be off by one for the bottom
//! nineteen-twentieths of the sheet, and a cell would be painted in one place
//! and clicked in another. Screen coordinates stay `f32`, computed relative to
//! the viewport where the numbers are small.
//!
//! The representation is a default size plus a sorted list of exceptions, which
//! is exactly how a spreadsheet file stores it — almost every row is the default
//! height, and the handful that are not are listed. A running total alongside
//! the exceptions turns "where does row N start?" into one binary search.

use std::collections::BTreeMap;

use ss_model::cell::{MAX_COLS, MAX_ROWS};

/// Excel's default column width, in the character units the file stores.
pub const DEFAULT_COLUMN_CHARS: f64 = 8.43;

/// Excel's default row height, in points.
pub const DEFAULT_ROW_POINTS: f64 = 15.0;

/// Width of a digit in the default font, in pixels at 96 DPI.
///
/// Column widths are stored as a count of these. The constant belongs to
/// Calibri 11, which is what a file written by any Excel since 2007 assumes; a
/// document that overrides the default font will be slightly off until C11
/// resolves fonts properly.
const DIGIT_WIDTH: f64 = 7.0;

/// Cell padding either side of the text, in pixels. Part of the stored width.
const CELL_PADDING: f64 = 5.0;

/// Converts a stored column width to pixels.
pub fn column_pixels(chars: f64, scale: f64) -> f64 {
    ((chars * DIGIT_WIDTH).round() + CELL_PADDING) * scale
}

/// The stored column width a pixel width corresponds to — the inverse of
/// [`column_pixels`], for turning a dragged edge back into what the file holds.
pub fn pixels_to_column(pixels: f64, scale: f64) -> f64 {
    ((pixels / scale) - CELL_PADDING).max(0.0) / DIGIT_WIDTH
}

pub fn pixels_to_row(pixels: f64, scale: f64) -> f64 {
    (pixels / scale) * 72.0 / 96.0
}

/// Converts a stored row height in points to pixels. 96 DPI over 72 points.
pub fn row_pixels(points: f64, scale: f64) -> f64 {
    points * 96.0 / 72.0 * scale
}

/// One axis of the grid: rows or columns.
pub struct Axis {
    default: f64,
    /// Sizes that differ from the default, sorted ascending by index.
    exceptions: Vec<(u32, f64)>,
    /// `prefix[k]` is the total of `size - default` over `exceptions[..k]`.
    ///
    /// This is what makes `offset` a binary search instead of a walk. Without
    /// it, finding the top of row 900,000 means adding up 900,000 heights.
    prefix: Vec<f64>,
    count: u32,
}

impl Axis {
    /// Builds an axis from a sheet's stored sizes.
    ///
    /// `to_pixels` converts one stored size, so the caller decides whether the
    /// numbers are character units or points.
    pub fn new(
        default: f64,
        count: u32,
        stored: &BTreeMap<u32, f64>,
        to_pixels: impl Fn(f64) -> f64,
    ) -> Self {
        let mut exceptions: Vec<(u32, f64)> = stored
            .iter()
            .filter(|(index, _)| **index < count)
            .map(|(index, size)| (*index, to_pixels(*size).max(0.0)))
            .filter(|(_, size)| *size != default)
            .collect();
        exceptions.sort_by_key(|(index, _)| *index);

        let mut prefix = Vec::with_capacity(exceptions.len() + 1);
        let mut running = 0.0;
        for (_, size) in &exceptions {
            prefix.push(running);
            running += size - default;
        }
        prefix.push(running);

        Axis {
            default,
            exceptions,
            prefix,
            count,
        }
    }

    /// An axis with no exceptions at all, for tests and blank sheets.
    pub fn uniform(default: f64, count: u32) -> Self {
        Axis {
            default,
            exceptions: Vec::new(),
            prefix: vec![0.0],
            count,
        }
    }

    /// The size of one row or column.
    pub fn size(&self, index: u32) -> f64 {
        match self.exceptions.binary_search_by_key(&index, |(i, _)| *i) {
            Ok(k) => self.exceptions[k].1,
            Err(_) => self.default,
        }
    }

    /// The distance from the start of the axis to the start of `index`.
    ///
    /// Defined for `index == count`, which is the total length.
    pub fn offset(&self, index: u32) -> f64 {
        let index = index.min(self.count);
        let before = self.exceptions.partition_point(|(i, _)| *i < index);
        f64::from(index) * self.default + self.prefix[before]
    }

    pub fn total(&self) -> f64 {
        self.offset(self.count)
    }

    /// The row or column containing `pos`, clamped to the axis.
    ///
    /// A run of zero-height rows all start at the same offset; this returns the
    /// last of them, which is the one whose body actually contains `pos`.
    pub fn index_at(&self, pos: f64) -> u32 {
        if pos <= 0.0 {
            return 0;
        }
        if pos >= self.total() {
            return self.count.saturating_sub(1);
        }
        let (mut lo, mut hi) = (0u32, self.count);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.offset(mid) <= pos {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo.saturating_sub(1)
    }

    /// The indices whose extents overlap `[from, to)`, as an inclusive range.
    ///
    /// This is the whole reason the grid can hold a million rows: the painter
    /// asks for the forty it can see and never learns the rest exist.
    // An inclusive range that runs backwards is the empty one, which is exactly
    // what "nothing is visible" should iterate as.
    #[allow(clippy::reversed_empty_ranges)]
    pub fn visible(&self, from: f64, to: f64) -> std::ops::RangeInclusive<u32> {
        if self.count == 0 || to <= from {
            return 1..=0;
        }
        let first = self.index_at(from.max(0.0));
        let last = self.index_at((to - 1.0).max(0.0));
        first..=last
    }
}

/// The two axes of a sheet, in pixels at the current zoom.
pub struct Layout {
    pub rows: Axis,
    pub cols: Axis,
    /// Width of the row-number gutter down the left edge.
    pub header_width: f64,
    /// Height of the column-letter strip across the top.
    pub header_height: f64,
    /// The scale this layout was built at. Anything measured in the file's own
    /// units — a drawing's EMU offsets — has to be scaled the same way.
    pub zoom: f64,
}

impl Layout {
    pub fn for_sheet(book: &ss_model::Workbook, sheet: &ss_model::Sheet, scale: f64) -> Self {
        let mut heights = auto_row_heights(book, sheet);
        // A stored height is the user's own answer and overrides the fitted
        // one — that is exactly what `customHeight="1"` means in the file.
        heights.extend(sheet.row_heights.iter().map(|(r, h)| (*r, *h)));

        Layout {
            rows: Axis::new(
                row_pixels(DEFAULT_ROW_POINTS, scale),
                MAX_ROWS,
                &heights,
                |points| row_pixels(points, scale),
            ),
            cols: Axis::new(
                column_pixels(DEFAULT_COLUMN_CHARS, scale),
                MAX_COLS,
                &sheet.column_widths,
                |chars| column_pixels(chars, scale),
            ),
            // Wide enough for seven digits, which is one more than the largest
            // row number needs.
            header_width: if sheet.view.headings {
                46.0 * scale
            } else {
                0.0
            },
            header_height: if sheet.view.headings {
                row_pixels(DEFAULT_ROW_POINTS, scale)
            } else {
                0.0
            },
            zoom: scale,
        }
    }
}

/// How many points of row a line of text at this font size needs.
///
/// Anchored so that the default font gives back exactly the default row: 11 pt
/// text in a 15 pt row. Anything else scales from there, which is why a row of
/// 22 pt headings is twice as tall without anybody storing a height.
fn line_points(font_size: f64) -> f64 {
    font_size * DEFAULT_ROW_POINTS / ss_model::style::DEFAULT_FONT_SIZE
}

/// A cell whose text is taller than one line makes its row taller.
///
/// Excel recomputes this on every repaint and stores nothing, which is why a
/// row holding three lines has no `ht` in the file at all. A reader that treats
/// "no stored height" as "the default height" draws all three lines in the
/// space of one, and they spill over the rows above and below — legible, if you
/// squint, as two different rows of the spreadsheet at once.
///
/// Only rows that need *more* than the default are returned. Nothing here
/// shrinks a row, and a merged cell is skipped because Excel does not fit rows
/// to merges either — the text is allowed to be clipped instead.
fn auto_row_heights(book: &ss_model::Workbook, sheet: &ss_model::Sheet) -> BTreeMap<u32, f64> {
    /// Past this a cell is a paragraph in the wrong container, and a row two
    /// screens tall helps nobody.
    const MAX_LINES: usize = 12;

    // Whether a style could ever want more than the default row, answered once
    // per style rather than once per cell. This runs over every cell in the
    // sheet on every edit, and on a sheet of a million cells the difference
    // between a table lookup and three map lookups plus a font is the
    // difference between a repaint and a pause.
    let taller: Vec<bool> = (0..book.styles.len())
        .map(|i| {
            let id = ss_model::StyleId(i as u32);
            book.styles.font(id).size > ss_model::style::DEFAULT_FONT_SIZE
                || book.styles.alignment(id).wrap
        })
        .collect();

    // Whether any string in the workbook holds a line break at all. Asked of
    // the table, where a status column is four strings rather than a million:
    // when the answer is no, no cell's text has to be looked at, and the loop
    // below never leaves the style table.
    let any_break = book.strings.iter().any(|s| s.contains('\n'));

    let mut fitted: BTreeMap<u32, f64> = BTreeMap::new();
    // `iter` is row-major, so a row's own answers are found once and then hold
    // for the whole row.
    let mut row = (u32::MAX, false, None);
    for (at, cell) in sheet.cells.iter() {
        if at.row != row.0 {
            row = (
                at.row,
                sheet.row_heights.contains_key(&at.row),
                sheet.row_styles.get(&at.row).copied(),
            );
        }
        if row.1 {
            continue; // the user's own height, which overrides any fit
        }

        let style = if cell.style != ss_model::StyleId::DEFAULT {
            cell.style
        } else {
            row.2.unwrap_or_else(|| {
                sheet
                    .column_styles
                    .get(&at.col)
                    .copied()
                    .unwrap_or(ss_model::StyleId::DEFAULT)
            })
        };
        let taller_style = taller.get(style.0 as usize).copied().unwrap_or(false);

        // Only a string can hold a line break. A number never does, whatever
        // its format, so its value never has to be rendered to count lines.
        let text = match cell.value {
            ss_model::CellValue::Text(id) if any_break || taller_style => {
                Some(book.strings.resolve(id))
            }
            _ => None,
        };
        // The one question left that the style cannot answer, and the one the
        // overwhelming majority of cells answer "no" to.
        let broken = text.is_some_and(|t| t.contains('\n'));
        if !broken && !taller_style {
            continue;
        }

        let font = book.styles.font(style);
        let mut lines = 1usize;
        if let Some(text) = text {
            lines = text.lines().count().max(1);
            if book.styles.alignment(style).wrap && sheet.merge_at(at).is_none() {
                lines = lines.max(wrapped_lines(text, font.size, width_of(sheet, at.col)));
            }
        }
        let wanted = line_points(font.size) * lines.min(MAX_LINES) as f64;
        if wanted > DEFAULT_ROW_POINTS {
            let slot = fitted.entry(at.row).or_insert(DEFAULT_ROW_POINTS);
            *slot = slot.max(wanted);
        }
    }
    fitted
}

/// A column's width in pixels at 100%, for estimating where text wraps.
fn width_of(sheet: &ss_model::Sheet, col: u32) -> f64 {
    let chars = sheet
        .column_widths
        .get(&col)
        .copied()
        .unwrap_or(DEFAULT_COLUMN_CHARS);
    column_pixels(chars, 1.0)
}

/// How many lines wrapped text takes, estimated from the glyph count.
///
/// An estimate rather than a measurement, and it has to be: the real answer
/// needs a font atlas, and this runs while building the layout, which happens
/// in tests with no window. Half the font size is close to the average advance
/// of a proportional face across ordinary prose, and being one line out on a
/// wrapped paragraph is a much smaller error than being four lines out on a
/// cell that holds four explicit lines.
fn wrapped_lines(text: &str, font_size: f64, column_pixels: f64) -> usize {
    let room = (column_pixels - 6.0).max(8.0);
    let advance = (font_size * 96.0 / 72.0) * 0.5;
    text.lines()
        .map(|line| {
            let width = line.chars().count() as f64 * advance;
            (width / room).ceil().max(1.0) as usize
        })
        .sum::<usize>()
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sizes(pairs: &[(u32, f64)]) -> BTreeMap<u32, f64> {
        pairs.iter().copied().collect()
    }

    #[test]
    fn a_uniform_axis_is_pure_multiplication() {
        let axis = Axis::uniform(20.0, MAX_ROWS);
        assert_eq!(axis.offset(0), 0.0);
        assert_eq!(axis.offset(1), 20.0);
        assert_eq!(axis.offset(1_000_000), 20_000_000.0);
        assert_eq!(axis.index_at(45.0), 2);
        assert_eq!(axis.total(), f64::from(MAX_ROWS) * 20.0);
    }

    #[test]
    fn exceptions_shift_everything_after_them() {
        // Rows 2 and 5 are twice the height; every row after them starts lower.
        let axis = Axis::new(20.0, 100, &sizes(&[(2, 30.0), (5, 30.0)]), |points| points);
        assert_eq!(axis.offset(2), 40.0);
        assert_eq!(axis.offset(3), 70.0, "row 2 was ten pixels taller");
        assert_eq!(axis.offset(6), 140.0, "both exceptions counted");
        assert_eq!(axis.size(2), 30.0);
        assert_eq!(axis.size(3), 20.0);
    }

    #[test]
    fn a_position_maps_back_to_the_index_it_came_from() {
        let axis = Axis::new(20.0, 100, &sizes(&[(2, 30.0), (5, 30.0)]), |p| p);
        // Round-tripping every index is the property that matters: the cell
        // under the mouse must be the cell that was painted there.
        for i in 0..100 {
            let start = axis.offset(i);
            assert_eq!(axis.index_at(start), i, "start of {i}");
            assert_eq!(axis.index_at(start + axis.size(i) - 0.5), i, "end of {i}");
        }
    }

    #[test]
    fn a_million_rows_are_addressed_without_walking_them() {
        // If any of this were linear the test would still pass — but it would
        // take long enough to notice, which is the point of using the real
        // sheet size rather than a small one.
        let axis = Axis::new(20.0, MAX_ROWS, &sizes(&[(1_000_000, 40.0)]), |p| p);
        assert_eq!(axis.offset(999_999), 19_999_980.0);
        assert_eq!(axis.offset(1_000_001), 20_000_040.0, "past the taller row");
        assert_eq!(axis.index_at(20_000_010.0), 1_000_000);
        assert_eq!(axis.index_at(axis.total() + 100.0), MAX_ROWS - 1);
    }

    #[test]
    fn a_viewport_sees_only_what_it_covers() {
        let axis = Axis::uniform(20.0, MAX_ROWS);
        // A 800-pixel window over a 20-million-pixel axis: forty rows.
        let visible = axis.visible(20_000_000.0 - 800.0, 20_000_000.0);
        assert_eq!(visible.clone().count(), 40);
        assert_eq!(*visible.start(), 999_960);
        assert_eq!(*visible.end(), 999_999);
        // An empty window asks for nothing rather than for one row.
        assert!(axis.visible(100.0, 100.0).is_empty());
    }

    #[test]
    fn a_zero_height_row_takes_no_space_and_is_not_lost() {
        let axis = Axis::new(20.0, 10, &sizes(&[(3, 0.0)]), |p| p);
        assert_eq!(axis.offset(3), 60.0);
        assert_eq!(axis.offset(4), 60.0, "the hidden row occupies nothing");
        assert_eq!(axis.size(3), 0.0);
        // Row 4 is what is actually drawn at that position.
        assert_eq!(axis.index_at(60.0), 4);
    }

    #[test]
    fn a_cell_holding_three_lines_makes_its_row_three_lines_tall() {
        // The bug this exists for: a workbook stores no height at all for a row
        // whose cell holds three lines, because Excel measures it at paint
        // time. Reading that as the default height draws all three lines in the
        // space of one, and they spill over the rows above and below.
        let mut book = ss_model::Workbook::blank();
        let id = book.strings.intern(
            "first
second
third",
        );
        let sheet = book.sheet_mut(0).expect("sheet 0");
        sheet.set(
            ss_model::CellRef::new(4, 0),
            ss_model::Cell {
                value: ss_model::CellValue::Text(id),
                ..Default::default()
            },
        );

        let sheet = book.sheet(0).expect("sheet 0");
        let layout = Layout::for_sheet(&book, sheet, 1.0);
        assert_eq!(layout.rows.size(4), row_pixels(45.0, 1.0), "three lines");
        assert_eq!(layout.rows.size(3), row_pixels(15.0, 1.0), "its neighbour");
    }

    #[test]
    fn a_stored_height_wins_over_the_fitted_one() {
        // `customHeight` is the user saying they have decided; a row they
        // squashed on purpose must not spring back open.
        let mut book = ss_model::Workbook::blank();
        let id = book.strings.intern(
            "one
two
three",
        );
        let sheet = book.sheet_mut(0).expect("sheet 0");
        sheet.set(
            ss_model::CellRef::new(0, 0),
            ss_model::Cell {
                value: ss_model::CellValue::Text(id),
                ..Default::default()
            },
        );
        sheet.row_heights.insert(0, 12.0);

        let sheet = book.sheet(0).expect("sheet 0");
        let layout = Layout::for_sheet(&book, sheet, 1.0);
        assert_eq!(layout.rows.size(0), row_pixels(12.0, 1.0));
    }

    #[test]
    fn a_sheet_with_no_headings_gives_their_room_back_to_the_cells() {
        let mut book = ss_model::Workbook::blank();
        book.sheet_mut(0).expect("sheet 0").view.headings = false;
        let sheet = book.sheet(0).expect("sheet 0");
        let layout = Layout::for_sheet(&book, sheet, 1.0);
        assert_eq!(layout.header_width, 0.0);
        assert_eq!(layout.header_height, 0.0);
    }

    #[test]
    fn stored_sizes_convert_to_the_pixels_excel_shows() {
        // The default column is 64 pixels wide and the default row 20 tall,
        // which is what every screenshot of Excel at 100% shows.
        assert_eq!(column_pixels(DEFAULT_COLUMN_CHARS, 1.0), 64.0);
        assert_eq!(row_pixels(DEFAULT_ROW_POINTS, 1.0), 20.0);
        assert_eq!(row_pixels(DEFAULT_ROW_POINTS, 2.0), 40.0);
    }
}
