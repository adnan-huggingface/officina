//! Drawing a chart on the grid.
//!
//! The chart's numbers are re-read from the cells rather than taken from the
//! cache the file carries, so editing B7 redraws the bar above it. The cache is
//! the fallback, for a reference we cannot resolve — a chart whose data lives in
//! a workbook that is not open.
//!
//! Nothing here writes anything. A chart is a view over three parts that are
//! preserved verbatim, and what is drawn is an approximation of what Excel
//! draws: the shape, the series, the labels and the legend, and no gradients,
//! no 3-D, no trendlines. That approximation costs nothing, because saving puts
//! back the bytes the file came with.

use ss_model::chart::{Anchor, Chart, ChartKind, LegendPosition, EMU_PER_POINT};
use ss_model::{Color, Workbook};
use ui_kit::egui;

use super::{Layout, Scroll};

/// The default palette Office assigns to series, accent 1 through 6.
const SERIES_COLORS: [[u8; 3]; 6] = [
    [0x44, 0x72, 0xC4],
    [0xED, 0x7D, 0x31],
    [0xA5, 0xA5, 0xA5],
    [0xFF, 0xC0, 0x00],
    [0x5B, 0x9B, 0xD5],
    [0x70, 0xAD, 0x47],
];

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

/// One series, resolved to the numbers actually on the sheet.
pub struct Plotted {
    pub name: String,
    pub values: Vec<Option<f64>>,
    pub color: egui::Color32,
}

/// Reads a chart's series out of the workbook, falling back to the cache.
pub fn resolve(book: &Workbook, chart: &Chart) -> Vec<Plotted> {
    chart
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
            let fallback = [SERIES_COLORS[index % SERIES_COLORS.len()]];
            let [r, g, b] = series
                .color
                .and_then(|c: Color| c.resolve(book.styles.theme()))
                .unwrap_or(fallback[0]);
            Plotted {
                name: series
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("Series {}", index + 1)),
                values: live.unwrap_or_else(|| series.values.clone()),
                color: egui::Color32::from_rgb(r, g, b),
            }
        })
        .collect()
}

/// Everything the painter needs that is not the chart itself.
pub struct Style {
    pub background: egui::Color32,
    pub outline: egui::Color32,
    pub text: egui::Color32,
    pub grid: egui::Color32,
    pub zoom: f32,
}

pub fn draw(
    painter: &egui::Painter,
    rect: egui::Rect,
    chart: &Chart,
    series: &[Plotted],
    style: &Style,
) {
    if rect.width() < 24.0 || rect.height() < 24.0 {
        return;
    }
    painter.rect_filled(rect, 2.0, style.background);
    painter.rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, style.outline),
        egui::StrokeKind::Inside,
    );

    let font = egui::FontId::proportional(11.0 * style.zoom);
    let small = egui::FontId::proportional(9.0 * style.zoom);
    let mut plot = rect.shrink(8.0 * style.zoom);

    if let Some(title) = &chart.title {
        let galley = painter.layout_no_wrap(
            title.clone(),
            egui::FontId::proportional(13.0 * style.zoom),
            style.text,
        );
        painter.galley(
            egui::pos2(rect.center().x - galley.size().x / 2.0, plot.top()),
            galley.clone(),
            style.text,
        );
        plot.min.y += galley.size().y + 4.0 * style.zoom;
    }

    // The legend takes its strip before the plot area is measured, so the plot
    // never draws underneath it.
    if let Some(position) = chart.legend {
        plot = draw_legend(painter, plot, series, style, &small, position);
    }
    if plot.width() < 16.0 || plot.height() < 16.0 {
        return;
    }

    let categories = chart.categories();
    match chart.kind {
        ChartKind::Pie | ChartKind::Doughnut => {
            draw_pie(painter, plot, series, chart.kind == ChartKind::Doughnut);
        }
        ChartKind::Other(_) => {
            let galley = painter.layout_no_wrap("chart".to_string(), font, style.grid);
            painter.galley(plot.center() - galley.size() / 2.0, galley, style.grid);
        }
        _ => draw_axes_chart(painter, plot, chart, series, categories, style, &small),
    }
}

fn draw_legend(
    painter: &egui::Painter,
    plot: egui::Rect,
    series: &[Plotted],
    style: &Style,
    font: &egui::FontId,
    position: LegendPosition,
) -> egui::Rect {
    let swatch = 7.0 * style.zoom;
    let line = 13.0 * style.zoom;
    let mut left = plot;
    match position {
        LegendPosition::Bottom => {
            let strip = plot.bottom() - line;
            let mut x = plot.left();
            for entry in series {
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(x, strip + (line - swatch) / 2.0),
                        egui::vec2(swatch, swatch),
                    ),
                    1.0,
                    entry.color,
                );
                x += swatch + 3.0;
                let galley = painter.layout_no_wrap(entry.name.clone(), font.clone(), style.text);
                painter.galley(egui::pos2(x, strip), galley.clone(), style.text);
                x += galley.size().x + 10.0 * style.zoom;
            }
            left.max.y -= line + 2.0;
        }
        _ => {
            // Top, left, right, and top-right all get the right-hand column:
            // at cell size the difference is a few pixels and the alternative
            // is four near-identical blocks of layout arithmetic.
            let width = (plot.width() * 0.28).min(110.0 * style.zoom);
            let strip = egui::Rect::from_min_size(
                egui::pos2(plot.right() - width, plot.top()),
                egui::vec2(width, plot.height()),
            );
            let mut y = strip.top();
            for entry in series {
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(strip.left(), y + (line - swatch) / 2.0),
                        egui::vec2(swatch, swatch),
                    ),
                    1.0,
                    entry.color,
                );
                let galley = painter.layout_no_wrap(entry.name.clone(), font.clone(), style.text);
                painter.galley(
                    egui::pos2(strip.left() + swatch + 3.0, y),
                    galley,
                    style.text,
                );
                y += line;
            }
            left.max.x -= width + 4.0;
        }
    }
    left
}

#[allow(clippy::too_many_arguments)]
fn draw_axes_chart(
    painter: &egui::Painter,
    plot: egui::Rect,
    chart: &Chart,
    series: &[Plotted],
    categories: &[String],
    style: &Style,
    font: &egui::FontId,
) {
    let points = series.iter().map(|s| s.values.len()).max().unwrap_or(0);
    if points == 0 {
        return;
    }
    let stacked = chart.grouping.stacked();
    let (low, high) = bounds(series, stacked);
    if !(high - low).is_finite() || high <= low {
        return;
    }

    // Room for the value labels down the left.
    let gutter = 34.0 * style.zoom;
    let footer = 12.0 * style.zoom;
    let area = egui::Rect::from_min_max(
        egui::pos2(plot.left() + gutter, plot.top()),
        egui::pos2(plot.right(), plot.bottom() - footer),
    );
    if area.width() < 8.0 || area.height() < 8.0 {
        return;
    }

    let y_of = |value: f64| {
        area.bottom() - ((value - low) / (high - low) * f64::from(area.height())) as f32
    };

    // Four gridlines and their labels, which is what makes the heights readable.
    for step in 0..=4 {
        let value = low + (high - low) * f64::from(step) / 4.0;
        let y = y_of(value);
        painter.hline(
            area.x_range(),
            y,
            egui::Stroke::new(1.0, style.grid.gamma_multiply(0.5)),
        );
        let galley = painter.layout_no_wrap(
            ss_model::format_general(round_label(value, high - low)),
            font.clone(),
            style.text,
        );
        painter.galley(
            egui::pos2(
                area.left() - 3.0 - galley.size().x,
                y - galley.size().y / 2.0,
            ),
            galley,
            style.text,
        );
    }
    painter.hline(
        area.x_range(),
        y_of(0.0f64.clamp(low, high)),
        egui::Stroke::new(1.0, style.grid),
    );

    let slot = area.width() / points as f32;
    match chart.kind {
        ChartKind::Line | ChartKind::Scatter => {
            for entry in series {
                let path: Vec<egui::Pos2> = entry
                    .values
                    .iter()
                    .enumerate()
                    .filter_map(|(i, v)| {
                        v.map(|v| egui::pos2(area.left() + slot * (i as f32 + 0.5), y_of(v)))
                    })
                    .collect();
                if path.len() > 1 {
                    painter.add(egui::Shape::line(
                        path.clone(),
                        egui::Stroke::new(1.6 * style.zoom, entry.color),
                    ));
                }
                for point in path {
                    painter.circle_filled(point, 1.8 * style.zoom, entry.color);
                }
            }
        }
        ChartKind::Area => {
            for entry in series {
                let mut path: Vec<egui::Pos2> = entry
                    .values
                    .iter()
                    .enumerate()
                    .filter_map(|(i, v)| {
                        v.map(|v| egui::pos2(area.left() + slot * (i as f32 + 0.5), y_of(v)))
                    })
                    .collect();
                if path.len() < 2 {
                    continue;
                }
                let base = y_of(0.0f64.clamp(low, high));
                path.push(egui::pos2(path[path.len() - 1].x, base));
                path.push(egui::pos2(path[0].x, base));
                painter.add(egui::Shape::convex_polygon(
                    path,
                    entry.color.gamma_multiply(0.45),
                    egui::Stroke::new(1.2 * style.zoom, entry.color),
                ));
            }
        }
        _ => {
            // Bars. Clustered puts the series side by side inside the slot;
            // stacked puts each one on top of the running total.
            let lanes = if stacked { 1 } else { series.len().max(1) };
            let bar = (slot * 0.72 / lanes as f32).max(1.0);
            let mut totals = vec![0.0f64; points];
            for (index, entry) in series.iter().enumerate() {
                for (i, value) in entry.values.iter().enumerate() {
                    let Some(value) = value else { continue };
                    let centre = area.left() + slot * (i as f32 + 0.5);
                    let x = if stacked {
                        centre - bar / 2.0
                    } else {
                        centre - slot * 0.36 + bar * index as f32
                    };
                    let (from, to) = if stacked {
                        let base = totals[i];
                        totals[i] += value;
                        (base, totals[i])
                    } else {
                        (0.0f64.clamp(low, high), *value)
                    };
                    let rect = egui::Rect::from_x_y_ranges(
                        x..=x + bar,
                        y_of(from.min(to))..=y_of(from.max(to)),
                    );
                    painter.rect_filled(rect, 0.0, entry.color);
                }
            }
        }
    }

    // Category labels, thinned so they never overlap.
    let step = ((painter
        .layout_no_wrap("MMM".to_string(), font.clone(), style.text)
        .size()
        .x
        / slot)
        .ceil() as usize)
        .max(1);
    for (i, label) in categories.iter().enumerate().step_by(step) {
        if label.is_empty() {
            continue;
        }
        let galley = painter.layout_no_wrap(label.clone(), font.clone(), style.text);
        painter.galley(
            egui::pos2(
                area.left() + slot * (i as f32 + 0.5) - galley.size().x / 2.0,
                area.bottom() + 1.0,
            ),
            galley,
            style.text,
        );
    }
}

/// The value range to plot over, always including zero.
///
/// A bar chart whose axis starts at the smallest value exaggerates every
/// difference on it, which is the most common way a chart lies.
fn bounds(series: &[Plotted], stacked: bool) -> (f64, f64) {
    let mut low = 0.0f64;
    let mut high = 0.0f64;
    if stacked {
        let points = series.iter().map(|s| s.values.len()).max().unwrap_or(0);
        for i in 0..points {
            let total: f64 = series
                .iter()
                .filter_map(|s| s.values.get(i).copied().flatten())
                .sum();
            low = low.min(total);
            high = high.max(total);
        }
    } else {
        for value in series.iter().flat_map(|s| s.values.iter().flatten()) {
            low = low.min(*value);
            high = high.max(*value);
        }
    }
    if high == low {
        high = low + 1.0;
    }
    (low, high)
}

/// Rounds an axis label to something readable at the scale it is drawn.
fn round_label(value: f64, span: f64) -> f64 {
    if span == 0.0 {
        return value;
    }
    let magnitude = 10f64.powf(span.abs().log10().floor() - 1.0);
    (value / magnitude).round() * magnitude
}

fn draw_pie(painter: &egui::Painter, plot: egui::Rect, series: &[Plotted], doughnut: bool) {
    let Some(first) = series.first() else { return };
    let total: f64 = first.values.iter().flatten().map(|v| v.abs()).sum();
    if total <= 0.0 {
        return;
    }
    let centre = plot.center();
    let radius = plot.width().min(plot.height()) / 2.0 - 2.0;
    let hole = if doughnut { radius * 0.5 } else { 0.0 };
    let mut from = -std::f32::consts::FRAC_PI_2;

    for (index, value) in first.values.iter().enumerate() {
        let Some(value) = value else { continue };
        let sweep = (value.abs() / total) as f32 * std::f32::consts::TAU;
        // A slice is drawn as a fan of triangles: epaint has no arc, and a
        // polygon approximation is indistinguishable at this size.
        let steps = ((sweep / 0.15).ceil() as usize).max(2);
        let [r, g, b] = SERIES_COLORS[index % SERIES_COLORS.len()];
        let color = egui::Color32::from_rgb(r, g, b);
        for step in 0..steps {
            let a = from + sweep * step as f32 / steps as f32;
            let b_angle = from + sweep * (step + 1) as f32 / steps as f32;
            let outer = |angle: f32| centre + egui::vec2(angle.cos(), angle.sin()) * radius;
            let inner = |angle: f32| centre + egui::vec2(angle.cos(), angle.sin()) * hole;
            painter.add(egui::Shape::convex_polygon(
                vec![inner(a), outer(a), outer(b_angle), inner(b_angle)],
                color,
                egui::Stroke::NONE,
            ));
        }
        from += sweep;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ss_model::chart::{AnchorPoint, Grouping, Series};
    use ss_model::{Cell, CellRef, CellValue, Sheet, Workbook};

    fn chart(kind: ChartKind) -> Chart {
        Chart {
            part: "/xl/charts/chart1.xml".to_string(),
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
        }
    }

    #[test]
    fn an_anchor_lands_where_its_cells_are() {
        let sheet = Sheet::new("S");
        let layout = Layout::for_sheet(&sheet, 1.0);
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
        let layout = Layout::for_sheet(&sheet, 1.0);
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
        chart.series[0].values_ref = Some("'Closed Book.xlsx'!$A$1:$A$3".to_string());
        let plotted = resolve(&Workbook::blank(), &chart);
        assert_eq!(
            plotted[0].values,
            [Some(1.0), Some(2.0), Some(3.0)],
            "what the producing application last computed"
        );
    }

    #[test]
    fn the_value_axis_always_includes_zero() {
        // An axis starting at the smallest value exaggerates every difference
        // on the chart, which is the most common way a chart lies.
        let series = [Plotted {
            name: "s".into(),
            values: vec![Some(100.0), Some(102.0), Some(101.0)],
            color: egui::Color32::RED,
        }];
        let (low, high) = bounds(&series, false);
        assert_eq!(low, 0.0);
        assert_eq!(high, 102.0);
    }

    #[test]
    fn a_stacked_chart_is_measured_against_its_totals() {
        let series = [
            Plotted {
                name: "a".into(),
                values: vec![Some(30.0), Some(40.0)],
                color: egui::Color32::RED,
            },
            Plotted {
                name: "b".into(),
                values: vec![Some(30.0), Some(40.0)],
                color: egui::Color32::BLUE,
            },
        ];
        assert_eq!(bounds(&series, false).1, 40.0);
        assert_eq!(bounds(&series, true).1, 80.0, "the stack, not the tallest");
    }
}
