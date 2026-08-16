//! Drawing a chart.
//!
//! Shared, because a chart in a document and a chart on a sheet are the same
//! picture of the same numbers — only the rectangle differs, and the caller
//! knows that. What is drawn is an approximation of what Office draws: the
//! shape, the series, the labels and the legend, and no gradients, no 3-D, no
//! trendlines. That approximation costs nothing, because both applications put
//! back the bytes their file came with.

use chart::{ChartKind, LegendPosition, Plot};

use crate::egui;

/// The default palette Office assigns to series, accent 1 through 6.
pub const SERIES_COLORS: [[u8; 3]; 6] = [
    [0x44, 0x72, 0xC4],
    [0xED, 0x7D, 0x31],
    [0xA5, 0xA5, 0xA5],
    [0xFF, 0xC0, 0x00],
    [0x5B, 0x9B, 0xD5],
    [0x70, 0xAD, 0x47],
];

/// One series, resolved to the numbers to draw.
///
/// A workbook resolves them from its cells so that editing B7 redraws the bar
/// above it; a document has only the cache the file carries.
pub struct Plotted {
    pub name: String,
    pub values: Vec<Option<f64>>,
    pub color: egui::Color32,
}

/// Everything the painter needs that is not the chart itself.
pub struct Style {
    pub background: egui::Color32,
    pub outline: egui::Color32,
    pub text: egui::Color32,
    pub grid: egui::Color32,
    pub zoom: f32,
    /// How a number becomes an axis label.
    ///
    /// The caller's business, not the painter's: a workbook has Excel's
    /// General format already written and tested, and handing the painter a
    /// second implementation of it is how the two would come to disagree.
    pub label: fn(f64) -> String,
}

pub fn draw(
    painter: &egui::Painter,
    rect: egui::Rect,
    plot: &Plot,
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
    let mut area = rect.shrink(8.0 * style.zoom);

    if let Some(title) = &plot.title {
        let galley = painter.layout_no_wrap(
            title.clone(),
            egui::FontId::proportional(13.0 * style.zoom),
            style.text,
        );
        painter.galley(
            egui::pos2(rect.center().x - galley.size().x / 2.0, area.top()),
            galley.clone(),
            style.text,
        );
        area.min.y += galley.size().y + 4.0 * style.zoom;
    }

    // The legend takes its strip before the plot area is measured, so the plot
    // never draws underneath it.
    if let Some(position) = plot.legend {
        area = draw_legend(painter, area, series, style, &small, position);
    }
    if area.width() < 16.0 || area.height() < 16.0 {
        return;
    }

    let categories = plot.categories();
    match plot.kind {
        ChartKind::Pie | ChartKind::Doughnut => {
            draw_pie(painter, area, series, plot.kind == ChartKind::Doughnut);
        }
        ChartKind::Other(_) => {
            let galley = painter.layout_no_wrap("chart".to_string(), font, style.grid);
            painter.galley(area.center() - galley.size() / 2.0, galley, style.grid);
        }
        _ => draw_axes_chart(painter, area, plot, series, categories, style, &small),
    }
}

fn draw_legend(
    painter: &egui::Painter,
    area: egui::Rect,
    series: &[Plotted],
    style: &Style,
    font: &egui::FontId,
    position: LegendPosition,
) -> egui::Rect {
    let swatch = 7.0 * style.zoom;
    let line = 13.0 * style.zoom;
    let mut left = area;
    match position {
        LegendPosition::Bottom => {
            let strip = area.bottom() - line;
            let mut x = area.left();
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
            let width = (area.width() * 0.28).min(110.0 * style.zoom);
            let strip = egui::Rect::from_min_size(
                egui::pos2(area.right() - width, area.top()),
                egui::vec2(width, area.height()),
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
    area: egui::Rect,
    plot: &Plot,
    series: &[Plotted],
    categories: &[String],
    style: &Style,
    font: &egui::FontId,
) {
    let points = series.iter().map(|s| s.values.len()).max().unwrap_or(0);
    if points == 0 {
        return;
    }
    let stacked = plot.grouping.stacked();
    let (low, high) = bounds(series, stacked);
    if !(high - low).is_finite() || high <= low {
        return;
    }

    // Room for the value labels down the left.
    let gutter = 34.0 * style.zoom;
    let footer = 12.0 * style.zoom;
    let area = egui::Rect::from_min_max(
        egui::pos2(area.left() + gutter, area.top()),
        egui::pos2(area.right(), area.bottom() - footer),
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
            (style.label)(round_label(value, high - low)),
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
    match plot.kind {
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
                    // From the two corners rather than from two ranges: a
                    // bigger value is a *smaller* y, so a range written low to
                    // high is upside down, and egui fills a negative rectangle
                    // with nothing at all. Every bar chart drew its axes, its
                    // gridlines and no bars. `from_two_pos` sorts the corners,
                    // so the mistake cannot come back.
                    let rect = egui::Rect::from_two_pos(
                        egui::pos2(x, y_of(from)),
                        egui::pos2(x + bar, y_of(to)),
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

/// The value range to area over, always including zero.
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

fn draw_pie(painter: &egui::Painter, area: egui::Rect, series: &[Plotted], doughnut: bool) {
    let Some(first) = series.first() else { return };
    let total: f64 = first.values.iter().flatten().map(|v| v.abs()).sum();
    if total <= 0.0 {
        return;
    }
    let centre = area.center();
    let radius = area.width().min(area.height()) / 2.0 - 2.0;
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
    use chart::{ChartKind, Grouping, Plot, Series};

    fn plot(kind: ChartKind) -> Plot {
        Plot {
            kind,
            grouping: Grouping::Clustered,
            horizontal: false,
            title: None,
            title_ref: None,
            legend: None,
            series: vec![Series {
                name: Some("Sales".to_string()),
                values: vec![Some(1.0), Some(2.0), Some(3.0)],
                ..Default::default()
            }],
        }
    }

    /// Every shape a chart draws into a 400x300 box, so a test can look for
    /// the ones that matter rather than at a screenshot.
    fn painted(plot: &Plot, series: &[Plotted]) -> Vec<egui::Shape> {
        let ctx = egui::Context::default();
        crate::fonts::register(&ctx, &[]);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(500.0, 400.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0));
            let style = Style {
                background: egui::Color32::WHITE,
                outline: egui::Color32::GRAY,
                text: egui::Color32::BLACK,
                grid: egui::Color32::GRAY,
                zoom: 1.0,
                label: |v| format!("{v}"),
            };
            draw(ui.painter(), rect, plot, series, &style);
        });
        out.textures_delta.clear();
        out.shapes.into_iter().map(|s| s.shape).collect()
    }

    #[test]
    fn a_bar_chart_draws_a_bar_for_every_point() {
        // It did not, for as long as bar charts have been drawn: the bar was
        // built from a y range written low to high, which is upside down on a
        // screen, and egui fills an upside-down rectangle with nothing. The
        // axes and the gridlines were all anyone ever saw.
        let series = vec![Plotted {
            name: "Sales".to_string(),
            values: vec![Some(1.0), Some(2.0), Some(3.0)],
            color: egui::Color32::RED,
        }];
        let bars: Vec<egui::Rect> = painted(&plot(ChartKind::Bar), &series)
            .iter()
            .filter_map(|shape| match shape {
                egui::Shape::Rect(r) if r.fill == egui::Color32::RED => Some(r.rect),
                _ => None,
            })
            .collect();
        assert_eq!(bars.len(), 3, "one bar per point");
        for bar in &bars {
            assert!(
                bar.height() > 1.0 && bar.width() > 1.0,
                "a bar nobody can see is not a bar: {bar:?}"
            );
        }
        // Taller values, taller bars — and all of them stand on one baseline.
        assert!(bars[0].height() < bars[1].height() && bars[1].height() < bars[2].height());
        assert!(bars.windows(2).all(|w| w[0].bottom() == w[1].bottom()));
    }

    #[test]
    fn a_document_chart_draws_from_its_cache_alone() {
        // A chart in a .docx has no cells behind it: whatever the producing
        // application wrote into `<c:numCache>` is the whole of the picture.
        let mut plot = plot(ChartKind::Bar);
        plot.series[0].values_ref = None;
        let series = vec![Plotted {
            name: "Column 1".to_string(),
            values: vec![Some(9.1), Some(2.4), Some(3.1)],
            color: egui::Color32::BLUE,
        }];
        let bars = painted(&plot, &series)
            .iter()
            .filter(|shape| matches!(shape, egui::Shape::Rect(r) if r.fill == egui::Color32::BLUE))
            .count();
        assert_eq!(bars, 3);
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
