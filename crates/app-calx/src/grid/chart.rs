//! Where a chart sits on the grid, and what numbers it plots.
//!
//! The picture itself is drawn by [`ui_kit::chart`], which Scriva draws its
//! documents' charts with too. What is here is the half a sheet owns: the
//! anchor arithmetic that turns cells into a rectangle, and the resolution
//! that re-reads a series out of the cells rather than taking the cache the
//! file carries — so editing B7 redraws the bar above it. The cache is the
//! fallback, for a reference we cannot resolve: a chart whose data lives in a
//! workbook that is not open.

use ss_model::chart::{Anchor, Chart, EMU_PER_POINT};
use ss_model::Workbook;
use ui_kit::chart::{Plotted, SERIES_COLORS};
use ui_kit::egui;

pub use ui_kit::chart::{draw, Style};

use super::{Layout, Scroll};

/// Where a chart sits on screen.
///
/// The offset into the anchor cell is honoured. Without it a chart snaps to a
/// column edge, which moves it by up to a whole column and makes it overlap the
/// data it is drawn from.
pub fn rect_of(layout: &Layout, anchor: &Anchor, view: egui::Rect, scroll: Scroll) -> egui::Rect {
    let corner = |col: u32, col_off: i64, row: u32, row_off: i64| {
        let x = layout.cols.offset(col) + emu_to_pixels(col_off, layout.zoom);
        let y = layout.rows.offset(row) + emu_to_pixels(row_off, layout.zoom);
        view.min + egui::vec2((x - scroll.x) as f32, (y - scroll.y) as f32)
    };
    match anchor {
        Anchor::TwoCell { from, to } => egui::Rect::from_two_pos(
            corner(from.col, from.col_offset, from.row, from.row_offset),
            corner(to.col, to.col_offset, to.row, to.row_offset),
        ),
        Anchor::OneCell {
            from,
            width,
            height,
        } => egui::Rect::from_min_size(
            corner(from.col, from.col_offset, from.row, from.row_offset),
            egui::vec2(
                emu_to_pixels(*width, layout.zoom) as f32,
                emu_to_pixels(*height, layout.zoom) as f32,
            ),
        ),
        Anchor::Absolute {
            x,
            y,
            width,
            height,
        } => egui::Rect::from_min_size(
            view.min
                + egui::vec2(
                    (emu_to_pixels(*x, layout.zoom) - scroll.x) as f32,
                    (emu_to_pixels(*y, layout.zoom) - scroll.y) as f32,
                ),
            egui::vec2(
                emu_to_pixels(*width, layout.zoom) as f32,
                emu_to_pixels(*height, layout.zoom) as f32,
            ),
        ),
    }
}

/// EMUs are Office's internal unit: 12,700 to the point. The grid works in
/// points at 100% zoom, so this is the whole conversion.
fn emu_to_pixels(emu: i64, zoom: f64) -> f64 {
    emu as f64 / EMU_PER_POINT * zoom
}

/// Reads a chart's series out of the workbook, falling back to the cache.
pub fn resolve(book: &Workbook, chart: &Chart) -> Vec<Plotted> {
    chart
        .plot
        .series
        .iter()
        .enumerate()
        .map(|(index, series)| {
            let live = series
                .values_ref
                .as_deref()
                .and_then(|f| ss_formula::workbook::range_values(book, f))
                .map(|values| {
                    values
                        .iter()
                        .map(|v| match v {
                            ss_formula::Value::Number(n) => Some(*n),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                })
                .filter(|values: &Vec<Option<f64>>| !values.is_empty());
            Plotted {
                name: series
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("Series {}", index + 1)),
                values: live.unwrap_or_else(|| series.values.clone()),
                rgb: series
                    .color
                    .unwrap_or(SERIES_COLORS[index % SERIES_COLORS.len()]),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ss_model::chart::{AnchorPoint, ChartKind, Grouping, Plot, Series};
    use ss_model::{Cell, CellRef, CellValue, Sheet, Workbook};

    fn chart(kind: ChartKind) -> Chart {
        Chart {
            part: "/xl/charts/chart1.xml".to_string(),
            drawing_part: "/xl/drawings/drawing1.xml".to_string(),
            anchor_index: 0,
            anchor: Anchor::TwoCell {
                from: AnchorPoint {
                    col: 2,
                    col_offset: 0,
                    row: 1,
                    row_offset: 0,
                },
                to: AnchorPoint {
                    col: 6,
                    col_offset: 0,
                    row: 11,
                    row_offset: 0,
                },
            },
            plot: Plot {
                kind,
                grouping: Grouping::Clustered,
                horizontal: false,
                title: None,
                title_ref: None,
                legend: None,
                series: vec![Series {
                    name: Some("Sales".to_string()),
                    values_ref: Some("Sheet1!$A$1:$A$3".to_string()),
                    values: vec![Some(1.0), Some(2.0), Some(3.0)],
                    ..Default::default()
                }],
                ..Plot::default()
            },
        }
    }

    #[test]
    fn an_anchor_lands_where_its_cells_are() {
        let sheet = Sheet::new("S");
        let layout = Layout::for_sheet(&Workbook::blank(), &sheet, 1.0);
        let view = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let rect = rect_of(
            &layout,
            &chart(ChartKind::Bar).anchor,
            view,
            Scroll::default(),
        );

        assert_eq!(rect.left(), layout.cols.offset(2) as f32);
        assert_eq!(rect.top(), layout.rows.offset(1) as f32);
        assert_eq!(rect.right(), layout.cols.offset(6) as f32);
    }

    #[test]
    fn the_offset_into_the_anchor_cell_is_honoured() {
        // Half an inch into the column. Snapping to the column edge would move
        // a chart by up to a whole column and put it over its own data.
        let sheet = Sheet::new("S");
        let layout = Layout::for_sheet(&Workbook::blank(), &sheet, 1.0);
        let view = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let anchor = Anchor::OneCell {
            from: AnchorPoint {
                col: 0,
                col_offset: 457_200,
                row: 0,
                row_offset: 0,
            },
            width: 914_400,
            height: 914_400,
        };
        let rect = rect_of(&layout, &anchor, view, Scroll::default());
        assert_eq!(rect.left(), 36.0, "457200 EMU is 36 points");
        assert_eq!(rect.width(), 72.0, "914400 EMU is an inch, 72 points");
    }

    #[test]
    fn the_numbers_come_from_the_cells_and_not_from_the_cache() {
        // The whole point of keeping the reference: a chart has to redraw when
        // the data under it changes, and the cache is what Excel last computed.
        let mut book = Workbook::blank();
        for (row, value) in [(0, 10.0), (1, 20.0), (2, 30.0)] {
            book.sheets[0].set(
                CellRef::new(row, 0),
                Cell {
                    value: CellValue::Number(value),
                    ..Default::default()
                },
            );
        }
        let plotted = resolve(&book, &chart(ChartKind::Bar));
        assert_eq!(plotted[0].values, [Some(10.0), Some(20.0), Some(30.0)]);
        assert_eq!(plotted[0].name, "Sales");
    }

    #[test]
    fn a_reference_that_resolves_to_nothing_falls_back_to_the_cache() {
        let mut chart = chart(ChartKind::Bar);
        chart.plot.series[0].values_ref = Some("'Closed Book.xlsx'!$A$1:$A$3".to_string());
        let plotted = resolve(&Workbook::blank(), &chart);
        assert_eq!(
            plotted[0].values,
            [Some(1.0), Some(2.0), Some(3.0)],
            "what the producing application last computed"
        );
    }
}
