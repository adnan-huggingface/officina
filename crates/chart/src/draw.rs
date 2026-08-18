//! Where a chart's ink goes, decided once for every device that draws it.
//!
//! A chart is drawn three times in this workspace — on the screen through
//! egui, into a PDF, onto a printer through GDI — and the way three renderers
//! come to disagree is by each doing its own arithmetic. So the arithmetic is
//! here, in plain numbers, and what comes out is a list of rectangles, lines
//! and strings that a backend only has to choose ink for.
//!
//! Text is the one thing this cannot do alone: where a title starts depends on
//! how wide it is, and only the renderer's fonts know that. Hence [`Measure`],
//! the single question this asks of its caller.
//!
//! What is drawn is an approximation of what Office draws: the shape, the
//! series, the labels and the legend, and no gradients, no 3-D, no trendlines.
//! That approximation costs nothing, because both applications put back the
//! bytes their file came with.

use crate::{Axis, ChartKind, LegendPosition, Paint, Plot};

/// How far the plot area stands clear of whatever is beside it — the value
/// labels on one side, the legend on the other — in label sizes.
///
/// This and the two below are Word's, measured from charts Word rendered
/// itself: probe documents whose chart area carries a visible border, so that
/// the plot rectangle could be read against the chart's own edges rather than
/// guessed from the page. They hold whatever the chart's width, the labels'
/// width, the legend's text, or the number of series — Office's automatic
/// layout is constants, not fractions of the box.
const CLEAR: f64 = 1.74;

/// The margin at the top of a chart and down its right edge.
const MARGIN: f64 = 1.2;

/// What is kept below the plot: the category labels, and the margin under
/// them.
const FOOTER: f64 = 2.45;

/// The default palette Office assigns to series, accent 1 through 6.
pub const SERIES_COLORS: [[u8; 3]; 6] = [
    [0x44, 0x72, 0xC4],
    [0xED, 0x7D, 0x31],
    [0xA5, 0xA5, 0xA5],
    [0xFF, 0xC0, 0x00],
    [0x5B, 0x9B, 0xD5],
    [0x70, 0xAD, 0x47],
];

/// A rectangle in whatever unit the caller passes in — pixels on a screen,
/// points on a page.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    /// From two opposite corners, sorted.
    ///
    /// A bigger value is a *smaller* y, so a bar written from its base to its
    /// top is upside down, and a renderer handed a negative rectangle fills it
    /// with nothing at all. Sorting here is what keeps that bug dead.
    pub fn from_corners(a: (f64, f64), b: (f64, f64)) -> Rect {
        Rect {
            x: a.0.min(b.0),
            y: a.1.min(b.1),
            width: (b.0 - a.0).abs(),
            height: (b.1 - a.1).abs(),
        }
    }

    pub fn left(&self) -> f64 {
        self.x
    }

    pub fn top(&self) -> f64 {
        self.y
    }

    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    pub fn shrink(&self, by: f64) -> Rect {
        Rect {
            x: self.x + by,
            y: self.y + by,
            width: (self.width - by * 2.0).max(0.0),
            height: (self.height - by * 2.0).max(0.0),
        }
    }
}

/// One series, resolved to the numbers to draw.
///
/// A workbook resolves them from its cells so that editing B7 redraws the bar
/// above it; a document has only the cache the file carries.
#[derive(Debug, Clone, PartialEq)]
pub struct Plotted {
    pub name: String,
    pub values: Vec<Option<f64>>,
    pub rgb: [u8; 3],
}

/// Everything the geometry needs that is not the chart itself.
pub struct Style {
    pub background: [u8; 3],
    pub outline: [u8; 3],
    pub text: [u8; 3],
    pub grid: [u8; 3],
    /// Scales the type and the details, so a chart in a cell an inch wide is
    /// not covered in labels. One means points on a page.
    pub zoom: f64,
    /// How a number becomes an axis label.
    ///
    /// The caller's business, not the geometry's: a workbook has Excel's
    /// General format already written and tested, and handing this a second
    /// implementation of it is how the two would come to disagree.
    pub label: fn(f64) -> String,
}

/// One piece of a drawn chart.
///
/// Deliberately fewer shapes than any one backend can draw: everything here
/// survives translation to a PDF content stream and to GDI, which is what
/// makes the screen's picture the printed one.
#[derive(Debug, Clone, PartialEq)]
pub enum Prim {
    /// A filled rectangle: the plot's ground, a bar, a legend swatch.
    Fill {
        rect: Rect,
        rgb: [u8; 3],
        /// Corner radius, which a backend without rounded corners may ignore.
        round: f64,
    },
    /// A stroked rectangle: the border around the whole chart.
    Frame {
        rect: Rect,
        rgb: [u8; 3],
        thickness: f64,
        round: f64,
    },
    /// A polyline: gridlines, axes, and the line chart itself.
    Line {
        points: Vec<(f64, f64)>,
        thickness: f64,
        rgb: [u8; 3],
    },
    /// A filled convex polygon: an area chart's band, a pie's slice.
    Poly {
        points: Vec<(f64, f64)>,
        rgb: [u8; 3],
        edge: Option<([u8; 3], f64)>,
    },
    /// A filled circle: the marker on a line chart's point.
    Dot {
        at: (f64, f64),
        radius: f64,
        rgb: [u8; 3],
    },
    /// A string on one line, positioned by its **top-left** corner — the
    /// corner every renderer here can compute a baseline from, and the only
    /// one that needs no font to place.
    Text {
        at: (f64, f64),
        size: f64,
        text: String,
        rgb: [u8; 3],
    },
}

/// An axis number, as an axis writes it.
///
/// The fallback for a caller with nothing better: a document has no number
/// formats of its own, and a label carrying fifteen decimal places of binary
/// rounding is unreadable. A workbook passes Excel's General format instead.
pub fn plain_label(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let text = format!("{value:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The series of a plot as the file itself cached them.
///
/// What a document draws from: there are no cells behind a chart in a .docx,
/// so whatever the producing application last computed is the whole of the
/// picture. A workbook resolves the references first and falls back to this.
pub fn cached_series(plot: &Plot) -> Vec<Plotted> {
    plot.series
        .iter()
        .enumerate()
        .map(|(index, series)| Plotted {
            name: series
                .name
                .clone()
                .unwrap_or_else(|| format!("Series {}", index + 1)),
            values: series.values.clone(),
            rgb: series
                .color
                .unwrap_or(SERIES_COLORS[index % SERIES_COLORS.len()]),
        })
        .collect()
}

/// The one question the geometry cannot answer for itself.
pub trait Measure {
    /// Width and height of `text` set on one line at `size`.
    fn size(&mut self, text: &str, size: f64) -> (f64, f64);
}

/// Blends `rgb` toward `over` — what a translucent colour would have come to.
///
/// The alternative is carrying alpha through to three backends, one of which
/// would need a PDF graphics state for it. Charts stand on their own opaque
/// ground, so the blend is exact wherever nothing overlaps and close enough
/// where an area chart does.
fn blend(rgb: [u8; 3], over: [u8; 3], alpha: f64) -> [u8; 3] {
    let mix = |a: u8, b: u8| (f64::from(a) * alpha + f64::from(b) * (1.0 - alpha)).round() as u8;
    [
        mix(rgb[0], over[0]),
        mix(rgb[1], over[1]),
        mix(rgb[2], over[2]),
    ]
}

/// The ink a stated paint comes to, or `None` for one the part silenced.
fn ink(paint: Paint, default: [u8; 3]) -> Option<[u8; 3]> {
    match paint {
        Paint::None => None,
        Paint::Auto => Some(default),
        Paint::Rgb(rgb) => Some(rgb),
    }
}

/// The stub an axis marks a major unit with: how far it reaches into the plot
/// area, how far out of it, and in what ink. `None` when it draws none.
fn tick_marks(axis: &Axis, length: f64, default: [u8; 3]) -> Option<(f64, f64, [u8; 3])> {
    if axis.deleted {
        return None;
    }
    let (inside, outside) = axis.major_tick.reach(length);
    if inside == 0.0 && outside == 0.0 {
        return None;
    }
    Some((inside, outside, ink(axis.line, default)?))
}

/// Everything a chart draws inside `rect`, in the order it is drawn.
pub fn primitives(
    rect: Rect,
    plot: &Plot,
    series: &[Plotted],
    style: &Style,
    measure: &mut dyn Measure,
) -> Vec<Prim> {
    let mut out = Vec::new();
    if rect.width < 24.0 || rect.height < 24.0 {
        return out;
    }
    // The chart area's own paint, as the part states it. LibreOffice writes
    // `noFill` on both for a chart in a document — Office draws no box there,
    // and neither does this.
    match plot.area_fill {
        Paint::None => {}
        Paint::Auto => out.push(Prim::Fill {
            rect,
            rgb: style.background,
            round: 2.0,
        }),
        Paint::Rgb(rgb) => out.push(Prim::Fill {
            rect,
            rgb,
            round: 2.0,
        }),
    }
    match plot.area_line {
        Paint::None => {}
        Paint::Auto => out.push(Prim::Frame {
            rect,
            rgb: style.outline,
            thickness: 1.0,
            round: 2.0,
        }),
        Paint::Rgb(rgb) => out.push(Prim::Frame {
            rect,
            rgb,
            thickness: 1.0,
            round: 2.0,
        }),
    }

    let small = 9.0 * style.zoom;
    // Word's insets, which are not the same on all four sides: a margin at the
    // top and down the right, none at all on the left — the value labels begin
    // at the chart's very edge, and what holds the plot off is [`CLEAR`].
    let margin = MARGIN * small;
    let mut area = Rect::new(
        rect.x,
        rect.y + margin,
        (rect.width - margin).max(0.0),
        (rect.height - margin * 2.0).max(0.0),
    );

    if let Some(title) = &plot.title {
        let size = 13.0 * style.zoom;
        let (width, height) = measure.size(title, size);
        out.push(Prim::Text {
            at: (rect.center().0 - width / 2.0, area.top()),
            size,
            text: title.clone(),
            rgb: style.text,
        });
        let taken = height + 4.0 * style.zoom;
        area.y += taken;
        area.height = (area.height - taken).max(0.0);
    }

    // The legend takes its strip before the plot area is measured, so the plot
    // never draws underneath it.
    if let Some(position) = plot.legend {
        area = legend(&mut out, area, series, style, small, position, measure);
    }
    if area.width < 16.0 || area.height < 16.0 {
        return out;
    }

    match plot.kind {
        ChartKind::Pie | ChartKind::Doughnut => {
            pie(&mut out, area, series, plot.kind == ChartKind::Doughnut);
        }
        ChartKind::Other(_) => {
            let size = 11.0 * style.zoom;
            let (width, height) = measure.size("chart", size);
            let (cx, cy) = area.center();
            out.push(Prim::Text {
                at: (cx - width / 2.0, cy - height / 2.0),
                size,
                text: "chart".to_string(),
                rgb: style.grid,
            });
        }
        _ => axes_chart(&mut out, area, plot, series, style, small, measure),
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn legend(
    out: &mut Vec<Prim>,
    area: Rect,
    series: &[Plotted],
    style: &Style,
    size: f64,
    position: LegendPosition,
    measure: &mut dyn Measure,
) -> Rect {
    // Word's swatch is a square six tenths of its text, and stands three
    // tenths clear of it.
    let swatch = 0.6 * size;
    let gap = 0.3 * size;
    let line = 13.0 * style.zoom;
    let mut left = area;
    match position {
        LegendPosition::Bottom => {
            // Office centres a bottom legend on the chart rather than running
            // it from the left edge.
            let strip = area.bottom() - line;
            let total: f64 = series
                .iter()
                .map(|entry| swatch + gap + measure.size(&entry.name, size).0)
                .sum::<f64>()
                + 10.0 * style.zoom * (series.len().saturating_sub(1)) as f64;
            let mut x = area.left() + (area.width - total).max(0.0) / 2.0;
            for entry in series {
                out.push(Prim::Fill {
                    rect: Rect::new(x, strip + (line - swatch) / 2.0, swatch, swatch),
                    rgb: entry.rgb,
                    round: 1.0,
                });
                x += swatch + gap;
                let (width, _) = measure.size(&entry.name, size);
                out.push(Prim::Text {
                    at: (x, strip),
                    size,
                    text: entry.name.clone(),
                    rgb: style.text,
                });
                x += width + 10.0 * style.zoom;
            }
            left.height = (left.height - line - 2.0).max(0.0);
        }
        _ => {
            // Top, left, right, and top-right all get the right-hand column:
            // at cell size the difference is a few pixels and the alternative
            // is four near-identical blocks of layout arithmetic. The column
            // is as wide as its widest entry — a fixed fraction here is a
            // plot area visibly narrower than the one Word draws — and, like
            // Office's, it is centred vertically. A top or top-right legend
            // keeps to the top.
            let widest = series
                .iter()
                .map(|entry| measure.size(&entry.name, size).0)
                .fold(0.0f64, f64::max);
            let width = (swatch + gap + widest).min(area.width * 0.4);
            let strip = Rect::new(area.right() - width, area.top(), width, area.height);
            let block = line * series.len() as f64;
            let mut y = match position {
                LegendPosition::Top | LegendPosition::TopRight => strip.top(),
                _ => strip.top() + (strip.height - block).max(0.0) / 2.0,
            };
            for entry in series {
                out.push(Prim::Fill {
                    rect: Rect::new(strip.left(), y + (line - swatch) / 2.0, swatch, swatch),
                    rgb: entry.rgb,
                    round: 1.0,
                });
                out.push(Prim::Text {
                    at: (strip.left() + swatch + gap, y),
                    size,
                    text: entry.name.clone(),
                    rgb: style.text,
                });
                y += line;
            }
            left.width = (left.width - width - CLEAR * size).max(0.0);
        }
    }
    left
}

#[allow(clippy::too_many_arguments)]
fn axes_chart(
    out: &mut Vec<Prim>,
    area: Rect,
    plot: &Plot,
    series: &[Plotted],
    style: &Style,
    size: f64,
    measure: &mut dyn Measure,
) {
    let points = series.iter().map(|s| s.values.len()).max().unwrap_or(0);
    if points == 0 {
        return;
    }
    let stacked = plot.grouping.stacked();
    let (data_low, data_high) = bounds(series, stacked);
    let (low, high, major) = axis(data_low, data_high);
    if !(high - low).is_finite() || high <= low {
        return;
    }

    // Every tick's label, measured before anything is placed: the gutter is as
    // wide as the widest of them, not a guess that pushes the plot around.
    let ticks = ((high - low) / major).round() as usize;
    let labels: Vec<(f64, String)> = (0..=ticks)
        .map(|step| {
            let value = low + major * step as f64;
            (value, (style.label)(value))
        })
        .collect();
    let widest = labels
        .iter()
        .map(|(_, text)| measure.size(text, size).0)
        .fold(0.0f64, f64::max);
    let gutter = widest + CLEAR * size;
    // What is left below the plot, the outer margin already taken off. Word's
    // is a constant rather than the height of a line, which is why this does
    // not ask the shaper for one.
    let footer = (FOOTER - MARGIN) * size;
    let area = Rect::new(
        area.left() + gutter,
        area.top(),
        area.width - gutter,
        area.height - footer,
    );
    if area.width < 8.0 || area.height < 8.0 {
        return;
    }

    let y_of = |value: f64| area.bottom() - (value - low) / (high - low) * area.height;

    // The plot area's own ground, under everything drawn inside it. Office's
    // modern default is bare paper, so silence paints nothing.
    if let Paint::Rgb(rgb) = plot.plot_fill {
        out.push(Prim::Fill {
            rect: area,
            rgb,
            round: 0.0,
        });
    }

    // Tick marks, in each axis's own ink. A third of the label's size is the
    // length Word's render measures to — a hair under three points against
    // the sample's nine-point labels.
    let stub = size / 3.0;
    let val_tick = tick_marks(&plot.val_axis, stub, style.grid);
    let cat_tick = tick_marks(&plot.cat_axis, stub, style.grid);

    // A gridline and its label at every major unit, which is what makes the
    // heights readable.
    for (value, text) in labels {
        let y = y_of(value);
        out.push(Prim::Line {
            points: vec![(area.left(), y), (area.right(), y)],
            thickness: 1.0,
            rgb: blend(style.grid, style.background, 0.5),
        });
        if let Some((inside, outside, rgb)) = val_tick {
            out.push(Prim::Line {
                points: vec![(area.left() - outside, y), (area.left() + inside, y)],
                thickness: 1.0,
                rgb,
            });
        }
        // Right-aligned one label-size clear of the axis, which puts the
        // widest of them at the chart's own left edge — where Word starts it.
        let (width, height) = measure.size(&text, size);
        out.push(Prim::Text {
            at: (area.left() - size - width, y - height / 2.0),
            size,
            text,
            rgb: style.text,
        });
    }
    // The category axis itself, at the value it crosses. Its own colour when
    // the part names one; an axis whose line the part silences draws none.
    let base = y_of(0.0f64.clamp(low, high));
    if let Some(rgb) = ink(plot.cat_axis.line, style.grid) {
        out.push(Prim::Line {
            points: vec![(area.left(), base), (area.right(), base)],
            thickness: 1.0,
            rgb,
        });
    }
    // The box around the plot area, when the part states one — the frame Word
    // draws around the sample's bars, right edge and ceiling included. Under
    // the bars, like the baseline: a column standing on the axis is not cut
    // by it.
    if let Paint::Rgb(rgb) = plot.plot_line {
        out.push(Prim::Frame {
            rect: area,
            rgb,
            thickness: 1.0,
            round: 0.0,
        });
    }

    let slot = area.width / points as f64;
    // The category axis's stubs stand at the *boundaries* between categories,
    // not under them: one at each end of every slot, both ends of the axis
    // included. The labels are what go in the middle.
    if let Some((inside, outside, rgb)) = cat_tick {
        for step in 0..=points {
            let x = area.left() + slot * step as f64;
            out.push(Prim::Line {
                points: vec![(x, base - inside), (x, base + outside)],
                thickness: 1.0,
                rgb,
            });
        }
    }

    match plot.kind {
        ChartKind::Line | ChartKind::Scatter => {
            for entry in series {
                let path: Vec<(f64, f64)> = entry
                    .values
                    .iter()
                    .enumerate()
                    .filter_map(|(i, v)| {
                        v.map(|v| (area.left() + slot * (i as f64 + 0.5), y_of(v)))
                    })
                    .collect();
                if path.len() > 1 {
                    out.push(Prim::Line {
                        points: path.clone(),
                        thickness: 1.6 * style.zoom,
                        rgb: entry.rgb,
                    });
                }
                for point in path {
                    out.push(Prim::Dot {
                        at: point,
                        radius: 1.8 * style.zoom,
                        rgb: entry.rgb,
                    });
                }
            }
        }
        ChartKind::Area => {
            for entry in series {
                let mut path: Vec<(f64, f64)> = entry
                    .values
                    .iter()
                    .enumerate()
                    .filter_map(|(i, v)| {
                        v.map(|v| (area.left() + slot * (i as f64 + 0.5), y_of(v)))
                    })
                    .collect();
                if path.len() < 2 {
                    continue;
                }
                path.push((path[path.len() - 1].0, base));
                path.push((path[0].0, base));
                out.push(Prim::Poly {
                    points: path,
                    rgb: blend(entry.rgb, style.background, 0.45),
                    edge: Some((entry.rgb, 1.2 * style.zoom)),
                });
            }
        }
        _ => {
            // Bars. Clustered puts the series side by side inside the slot;
            // stacked puts each one on top of the running total. How fat they
            // are is the file's `gapWidth`: the space between groups measured
            // in bars, so a slot holds `lanes + gap/100` bar-widths of room.
            let lanes = if stacked { 1 } else { series.len().max(1) };
            let bar = (slot / (lanes as f64 + plot.gap.max(0.0) / 100.0)).max(1.0);
            let mut totals = vec![0.0f64; points];
            for (index, entry) in series.iter().enumerate() {
                for (i, value) in entry.values.iter().enumerate() {
                    let Some(value) = value else { continue };
                    let centre = area.left() + slot * (i as f64 + 0.5);
                    let x = if stacked {
                        centre - bar / 2.0
                    } else {
                        centre - bar * lanes as f64 / 2.0 + bar * index as f64
                    };
                    let (from, to) = if stacked {
                        let start = totals[i];
                        totals[i] += value;
                        (start, totals[i])
                    } else {
                        (0.0f64.clamp(low, high), *value)
                    };
                    out.push(Prim::Fill {
                        rect: Rect::from_corners((x, y_of(from)), (x + bar, y_of(to))),
                        rgb: entry.rgb,
                        round: 0.0,
                    });
                }
            }
        }
    }

    // Category labels, thinned so they never overlap.
    let step = ((measure.size("MMM", size).0 / slot).ceil() as usize).max(1);
    for (i, label) in plot.categories().iter().enumerate().step_by(step) {
        if label.is_empty() {
            continue;
        }
        let (width, _) = measure.size(label, size);
        out.push(Prim::Text {
            at: (
                area.left() + slot * (i as f64 + 0.5) - width / 2.0,
                area.bottom() + 0.5 * size,
            ),
            size,
            text: label.clone(),
            rgb: style.text,
        });
    }
}

/// The value range to plot over, always including zero.
///
/// A bar chart whose axis starts at the smallest value exaggerates every
/// difference on it, which is the most common way a chart lies.
pub fn bounds(series: &[Plotted], stacked: bool) -> (f64, f64) {
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

/// The axis Office draws over a data range: minimum, maximum, and the major
/// unit its gridlines step by.
///
/// Word's chart of 9.7-at-most runs its axis 0 to 12 in steps of 2 — not 0 to
/// 9.7 in quarters. The rule, measured against what Excel and Word draw: the
/// major unit is the "nice" number (1, 2 or 5 times a power of ten) nearest a
/// fifth of the range, and the ends are the data padded by 5% and pushed out
/// to the next multiple of it. An axis that stops exactly at the tallest bar
/// pins that bar to the ceiling, which no Office chart does.
pub fn axis(low: f64, high: f64) -> (f64, f64, f64) {
    let range = high - low;
    if !range.is_finite() || range <= 0.0 {
        return (low, low + 1.0, 0.25);
    }
    let raw = range / 5.0;
    let magnitude = 10f64.powf(raw.log10().floor());
    let major = [1.0, 2.0, 5.0, 10.0]
        .iter()
        .map(|unit| unit * magnitude)
        .min_by(|a, b| {
            let fit = |x: &f64| (x / raw).ln().abs();
            fit(a).total_cmp(&fit(b))
        })
        .expect("four candidates");
    let pad = range * 0.05;
    let max = if high > 0.0 {
        ((high + pad) / major).ceil() * major
    } else {
        0.0
    };
    let min = if low < 0.0 {
        ((low - pad) / major).floor() * major
    } else {
        0.0
    };
    (min, max, major)
}

fn pie(out: &mut Vec<Prim>, area: Rect, series: &[Plotted], doughnut: bool) {
    let Some(first) = series.first() else { return };
    let total: f64 = first.values.iter().flatten().map(|v| v.abs()).sum();
    if total <= 0.0 {
        return;
    }
    let centre = area.center();
    let radius = area.width.min(area.height) / 2.0 - 2.0;
    let hole = if doughnut { radius * 0.5 } else { 0.0 };
    let mut from = -std::f64::consts::FRAC_PI_2;

    for (index, value) in first.values.iter().enumerate() {
        let Some(value) = value else { continue };
        let sweep = (value.abs() / total) * std::f64::consts::TAU;
        // A slice is drawn as a fan of quads: no backend here has an arc, and
        // a polygon approximation is indistinguishable at this size.
        let steps = ((sweep / 0.15).ceil() as usize).max(2);
        let rgb = SERIES_COLORS[index % SERIES_COLORS.len()];
        for step in 0..steps {
            let a = from + sweep * step as f64 / steps as f64;
            let b = from + sweep * (step + 1) as f64 / steps as f64;
            let outer = |angle: f64| {
                (
                    centre.0 + angle.cos() * radius,
                    centre.1 + angle.sin() * radius,
                )
            };
            let inner = |angle: f64| (centre.0 + angle.cos() * hole, centre.1 + angle.sin() * hole);
            out.push(Prim::Poly {
                points: vec![inner(a), outer(a), outer(b), inner(b)],
                rgb,
                edge: None,
            });
        }
        from += sweep;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Grouping, Series};

    /// A polyline, as [`Prim::Line`] carries one.
    type Path = Vec<(f64, f64)>;

    /// Half the size in width per character, which is what the layout
    /// engine's own test shaper says and is enough to place a label.
    struct Half;

    impl Measure for Half {
        fn size(&mut self, text: &str, size: f64) -> (f64, f64) {
            (text.chars().count() as f64 * size * 0.5, size)
        }
    }

    fn style() -> Style {
        Style {
            background: [255, 255, 255],
            outline: [128, 128, 128],
            text: [0, 0, 0],
            grid: [128, 128, 128],
            zoom: 1.0,
            label: |v| format!("{v}"),
        }
    }

    fn plot(kind: ChartKind) -> Plot {
        Plot {
            kind,
            grouping: Grouping::Clustered,
            series: vec![Series::default()],
            ..Plot::default()
        }
    }

    fn bars(prims: &[Prim], rgb: [u8; 3]) -> Vec<Rect> {
        prims
            .iter()
            .filter_map(|prim| match prim {
                Prim::Fill {
                    rect, rgb: fill, ..
                } if *fill == rgb => Some(*rect),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_bar_chart_draws_a_bar_for_every_point() {
        let series = [Plotted {
            name: "Sales".to_string(),
            values: vec![Some(1.0), Some(2.0), Some(3.0)],
            rgb: [255, 0, 0],
        }];
        let prims = primitives(
            Rect::new(0.0, 0.0, 400.0, 300.0),
            &plot(ChartKind::Bar),
            &series,
            &style(),
            &mut Half,
        );
        let bars = bars(&prims, [255, 0, 0]);
        assert_eq!(bars.len(), 3, "one bar per point");
        for bar in &bars {
            assert!(
                bar.height > 1.0 && bar.width > 1.0,
                "a bar nobody can see is not a bar: {bar:?}"
            );
        }
        // Taller values, taller bars — and all of them stand on one baseline.
        assert!(bars[0].height < bars[1].height && bars[1].height < bars[2].height);
        assert!(bars.windows(2).all(|w| w[0].bottom() == w[1].bottom()));
    }

    #[test]
    fn a_chart_too_small_to_read_draws_nothing() {
        let prims = primitives(
            Rect::new(0.0, 0.0, 12.0, 12.0),
            &plot(ChartKind::Bar),
            &[],
            &style(),
            &mut Half,
        );
        assert!(prims.is_empty());
    }

    #[test]
    fn a_title_is_centred_on_the_chart_and_takes_its_room_from_the_plot() {
        let mut with = plot(ChartKind::Bar);
        with.title = Some("Sales".to_string());
        let series = [Plotted {
            name: "s".to_string(),
            values: vec![Some(1.0)],
            rgb: [255, 0, 0],
        }];
        let rect = Rect::new(0.0, 0.0, 400.0, 300.0);
        let prims = primitives(rect, &with, &series, &style(), &mut Half);
        let Some(Prim::Text { at, text, .. }) = prims
            .iter()
            .find(|p| matches!(p, Prim::Text { text, .. } if text == "Sales"))
            .cloned()
        else {
            panic!("a title");
        };
        assert_eq!(text, "Sales");
        // Five characters at half of thirteen points: 32.5 wide, centred on 200.
        assert!((at.0 - (200.0 - 16.25)).abs() < 0.001, "centred: {at:?}");
        // And the bar below it starts under the title, not behind it.
        let bar = bars(&prims, [255, 0, 0])[0];
        assert!(bar.top() > at.1 + 13.0, "the plot cleared the title");
    }

    #[test]
    fn the_value_axis_always_includes_zero() {
        // An axis starting at the smallest value exaggerates every difference
        // on the chart, which is the most common way a chart lies.
        let series = [Plotted {
            name: "s".into(),
            values: vec![Some(100.0), Some(102.0), Some(101.0)],
            rgb: [255, 0, 0],
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
                rgb: [255, 0, 0],
            },
            Plotted {
                name: "b".into(),
                values: vec![Some(30.0), Some(40.0)],
                rgb: [0, 0, 255],
            },
        ];
        assert_eq!(bounds(&series, false).1, 40.0);
        assert_eq!(bounds(&series, true).1, 80.0, "the stack, not the tallest");
    }

    #[test]
    fn the_axis_runs_past_the_tallest_bar_the_way_offices_does() {
        // Word draws the sample's 9.7 maximum on an axis of 0 to 12 by twos —
        // an axis stopping exactly at the data pins the tallest bar to the
        // ceiling, which no Office chart does.
        assert_eq!(axis(0.0, 9.7), (0.0, 12.0, 2.0));
        assert_eq!(axis(0.0, 102.0), (0.0, 120.0, 20.0));
        assert_eq!(axis(0.0, 3.0), (0.0, 3.5, 0.5));
        // A negative floor is pushed down the same way the ceiling is pushed up.
        let (min, max, major) = axis(-4.0, 9.7);
        assert_eq!(major, 2.0);
        assert_eq!((min, max), (-6.0, 12.0));
    }

    #[test]
    fn a_chart_that_declines_all_paint_draws_no_box() {
        // LibreOffice writes noFill for both the area and its border on every
        // chart it puts in a document; Office draws no box there, and a frame
        // of our own invention would be a box Word does not print.
        let mut bare = plot(ChartKind::Bar);
        bare.area_fill = Paint::None;
        bare.area_line = Paint::None;
        bare.series[0].values = vec![Some(1.0)];
        let series = cached_series(&bare);
        let prims = primitives(
            Rect::new(0.0, 0.0, 400.0, 300.0),
            &bare,
            &series,
            &style(),
            &mut Half,
        );
        let whole = Rect::new(0.0, 0.0, 400.0, 300.0);
        assert!(
            !prims.iter().any(|p| match p {
                Prim::Frame { .. } => true,
                Prim::Fill { rect, .. } => *rect == whole,
                _ => false,
            }),
            "no ground and no frame"
        );
    }

    #[test]
    fn a_stated_plot_border_boxes_the_bars_and_silence_does_not() {
        // The sample declares the plotArea's line as B3B3B3, and Word draws a
        // gray box around the bars — right edge and ceiling included. A part
        // that says nothing gets Office's modern default: no box.
        let mut boxed = plot(ChartKind::Bar);
        boxed.plot_line = Paint::Rgb([0xB3, 0xB3, 0xB3]);
        boxed.series[0].values = vec![Some(1.0)];
        let series = cached_series(&boxed);
        let rect = Rect::new(0.0, 0.0, 400.0, 300.0);
        let prims = primitives(rect, &boxed, &series, &style(), &mut Half);
        let frame = prims
            .iter()
            .find_map(|p| match p {
                Prim::Frame { rect, rgb, .. } if *rgb == [0xB3, 0xB3, 0xB3] => Some(*rect),
                _ => None,
            })
            .expect("the plot box");
        assert!(
            frame.width < rect.width && frame.height < rect.height,
            "the box is around the plot, not the whole chart"
        );

        let mut silent = plot(ChartKind::Bar);
        silent.series[0].values = vec![Some(1.0)];
        let series = cached_series(&silent);
        let prims = primitives(rect, &silent, &series, &style(), &mut Half);
        assert!(
            !prims
                .iter()
                .any(|p| matches!(p, Prim::Frame { rect, .. } if rect.width < 350.0)),
            "no box of our own invention inside the chart"
        );
    }

    #[test]
    fn ticks_stand_outside_the_plot_at_the_units_and_at_the_category_boundaries() {
        // Word writes `majorTickMark="out"` on both of the sample's axes and
        // draws a stub beyond the plot at every gridline and at every seam
        // between categories — the ends of the axis included, which is why
        // four categories get five.
        let mut ticked = plot(ChartKind::Bar);
        ticked.cat_axis = Axis {
            line: Paint::Rgb([0xB3, 0xB3, 0xB3]),
            major_tick: crate::TickMark::Out,
            deleted: false,
        };
        ticked.val_axis = ticked.cat_axis;
        ticked.series[0].values = vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0)];
        let series = cached_series(&ticked);
        let rect = Rect::new(0.0, 0.0, 400.0, 300.0);
        let prims = primitives(rect, &ticked, &series, &style(), &mut Half);

        let stubs: Vec<Path> = prims
            .iter()
            .filter_map(|p| match p {
                Prim::Line { points, rgb, .. } if *rgb == [0xB3, 0xB3, 0xB3] => {
                    Some(points.clone())
                }
                _ => None,
            })
            .collect();
        let upright: Vec<&Path> = stubs.iter().filter(|p| p[0].0 == p[1].0).collect();
        assert_eq!(upright.len(), 5, "a seam at each end of four slots");
        // Evenly spaced, and every one of them hangs *below* the axis rather
        // than cutting up through the bars.
        let xs: Vec<f64> = upright.iter().map(|p| p[0].0).collect();
        let step = xs[1] - xs[0];
        assert!(xs.windows(2).all(|w| (w[1] - w[0] - step).abs() < 1e-9));
        assert!(upright.iter().all(|p| p[1].1 > p[0].1));

        let flat: Vec<&Path> = stubs
            .iter()
            .filter(|p| p[0].1 == p[1].1 && p[0].0 != p[1].0)
            .collect();
        // The flat ones are the stubs at each major unit of an axis running 0
        // to 5 by ones, plus the category axis itself, which is as wide as the
        // plot and is what tells the two apart.
        let (axis_line, value_stubs): (Vec<&Path>, Vec<&Path>) = flat
            .into_iter()
            .partition(|p| (p[1].0 - p[0].0).abs() > 20.0);
        assert_eq!(axis_line.len(), 1, "the category axis itself");
        assert_eq!(value_stubs.len(), 6, "one per major unit of 0..=5");
        // Three points long — a third of the nine-point label — and reaching
        // out of the plot, not into it.
        for stub in &value_stubs {
            assert!((stub[1].0 - stub[0].0 - 3.0).abs() < 1e-9, "{stub:?}");
            assert!(stub[0].0 < axis_line[0][0].0, "outside the plot");
        }
    }

    #[test]
    fn an_axis_that_states_no_ticks_or_no_line_draws_none() {
        let rect = Rect::new(0.0, 0.0, 400.0, 300.0);
        let mut silent = plot(ChartKind::Bar);
        silent.series[0].values = vec![Some(1.0), Some(2.0)];
        let series = cached_series(&silent);
        let prims = primitives(rect, &silent, &series, &style(), &mut Half);
        assert!(
            !prims
                .iter()
                .any(|p| matches!(p, Prim::Line { points, .. } if points[0].0 == points[1].0)),
            "no upright stub anybody asked for"
        );

        // Ticks stated but the axis line silenced: there is no ink to draw
        // them in, and Word draws neither.
        let mut hidden = silent.clone();
        hidden.cat_axis = Axis {
            line: Paint::None,
            major_tick: crate::TickMark::Out,
            deleted: false,
        };
        let prims = primitives(rect, &hidden, &series, &style(), &mut Half);
        assert!(
            !prims
                .iter()
                .any(|p| matches!(p, Prim::Line { points, .. } if points[0].0 == points[1].0)),
            "a silenced axis draws no stubs"
        );

        // And an axis the author deleted keeps its ticks to itself.
        let mut gone = silent.clone();
        gone.val_axis = Axis {
            line: Paint::Rgb([1, 2, 3]),
            major_tick: crate::TickMark::Out,
            deleted: true,
        };
        let prims = primitives(rect, &gone, &series, &style(), &mut Half);
        assert!(!prims
            .iter()
            .any(|p| matches!(p, Prim::Line { rgb, .. } if *rgb == [1, 2, 3])));
    }

    #[test]
    fn the_gap_width_is_what_sets_how_fat_a_bar_is() {
        // gapWidth 100: a group of three bars plus a one-bar gap fills the
        // slot, so each bar is a quarter of it.
        let mut plot = plot(ChartKind::Bar);
        plot.gap = 100.0;
        plot.series = vec![Series::default(), Series::default(), Series::default()];
        for s in &mut plot.series {
            s.values = vec![Some(2.0), Some(3.0)];
        }
        let series = cached_series(&plot);
        let prims = primitives(
            Rect::new(0.0, 0.0, 400.0, 300.0),
            &plot,
            &series,
            &style(),
            &mut Half,
        );
        let first = bars(&prims, SERIES_COLORS[0]);
        assert_eq!(first.len(), 2, "series one, one bar per category");
        // The distance between the two category centres is one slot, and with
        // gapWidth 100 a slot holds four bar-widths: three bars and their gap.
        let slot = (first[1].x - first[0].x).abs();
        assert!(
            (first[0].width - slot / 4.0).abs() < 1e-6,
            "bar {} in slot {}",
            first[0].width,
            slot
        );
    }

    #[test]
    fn a_right_legend_is_as_wide_as_its_widest_entry_and_stands_at_mid_height() {
        // A fixed-fraction column reserved far more room than Word's legend
        // takes, and the plot area came out visibly narrower than Word's.
        let mut with_legend = plot(ChartKind::Bar);
        with_legend.legend = Some(LegendPosition::Right);
        with_legend.series[0].name = Some("Column 1".to_string());
        with_legend.series[0].values = vec![Some(1.0), Some(2.0)];
        let series = cached_series(&with_legend);
        let rect = Rect::new(0.0, 0.0, 400.0, 300.0);
        let prims = primitives(rect, &with_legend, &series, &style(), &mut Half);

        let (entry, label) = prims
            .iter()
            .find_map(|p| match p {
                Prim::Text { at, text, .. } if text == "Column 1" => Some((*at, text.clone())),
                _ => None,
            })
            .expect("the legend entry");
        assert_eq!(label, "Column 1");
        // Swatch + gap + "Column 1" at half-size (8 chars × 4.5), measured
        // from the chart's right margin — not 28% of the chart.
        let area_right = 400.0 - MARGIN * 9.0;
        let width = 0.6 * 9.0 + 0.3 * 9.0 + 8.0 * 4.5;
        assert!(
            (entry.0 - (area_right - width + 0.6 * 9.0 + 0.3 * 9.0)).abs() < 0.001,
            "the column starts at its measured width: {}",
            entry.0
        );
        // One 13pt line centred in the area between the top and bottom
        // margins: its top sits at the middle, not at the top of the chart.
        let inner = 300.0 - MARGIN * 9.0 * 2.0;
        assert!(
            (entry.1 - (MARGIN * 9.0 + (inner - 13.0) / 2.0)).abs() < 0.001,
            "centred vertically: {}",
            entry.1
        );
    }

    #[test]
    fn the_plot_rectangle_sits_where_word_puts_it() {
        // Measured from charts Word rendered itself: the plot stands 1.74
        // label-sizes clear of the widest value label on one side and of the
        // legend on the other, with a margin of 1.2 above and down the right
        // and 2.45 below for the category labels. Everything about a bar —
        // how fat it is, which slot it stands in — follows from this
        // rectangle, so this is the test that keeps the bars where Word's are.
        let mut boxed = plot(ChartKind::Bar);
        boxed.plot_line = Paint::Rgb([1, 2, 3]);
        boxed.series[0].values = vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0)];
        let series = cached_series(&boxed);
        let rect = Rect::new(0.0, 0.0, 400.0, 300.0);
        let prims = primitives(rect, &boxed, &series, &style(), &mut Half);
        let plot_rect = prims
            .iter()
            .find_map(|p| match p {
                Prim::Frame { rect, rgb, .. } if *rgb == [1, 2, 3] => Some(*rect),
                _ => None,
            })
            .expect("the plot box");

        // The axis runs 0 to 5 by ones, so the widest label is one character
        // wide: 4.5 at the test measurer's half-size.
        let size = 9.0;
        let close = |a: f64, b: f64, what: &str| {
            assert!((a - b).abs() < 0.001, "{what}: {a} wanted {b}");
        };
        close(plot_rect.left(), 4.5 + CLEAR * size, "left");
        close(
            plot_rect.right(),
            400.0 - MARGIN * size,
            "right (no legend)",
        );
        close(plot_rect.top(), MARGIN * size, "top");
        close(plot_rect.bottom(), 300.0 - FOOTER * size, "bottom");

        // With a legend the right edge gives up the column and another
        // clearance, and nothing else moves.
        let mut with = boxed.clone();
        with.legend = Some(LegendPosition::Right);
        with.series[0].name = Some("Column 1".to_string());
        let series = cached_series(&with);
        let prims = primitives(rect, &with, &series, &style(), &mut Half);
        let legended = prims
            .iter()
            .find_map(|p| match p {
                Prim::Frame { rect, rgb, .. } if *rgb == [1, 2, 3] => Some(*rect),
                _ => None,
            })
            .expect("the plot box");
        let column = 0.6 * size + 0.3 * size + 8.0 * 4.5;
        close(legended.left(), plot_rect.left(), "left is unmoved");
        close(
            legended.right(),
            400.0 - MARGIN * size - column - CLEAR * size,
            "right",
        );
    }

    #[test]
    fn an_axis_label_keeps_only_the_decimals_it_has() {
        assert_eq!(plain_label(3.0), "3");
        assert_eq!(plain_label(-12.5), "-12.5");
        assert_eq!(plain_label(0.1 + 0.2), "0.3", "not 0.30000000000000004");
    }

    #[test]
    fn a_series_without_a_name_or_a_colour_gets_the_ones_office_would_give_it() {
        let mut plot = plot(ChartKind::Bar);
        plot.series.push(Series::default());
        let series = cached_series(&plot);
        assert_eq!(series[0].name, "Series 1");
        assert_eq!(series[1].name, "Series 2");
        assert_eq!(series[0].rgb, SERIES_COLORS[0]);
        assert_eq!(series[1].rgb, SERIES_COLORS[1]);
    }

    #[test]
    fn a_translucent_colour_is_blended_rather_than_carried() {
        assert_eq!(blend([0, 0, 0], [255, 255, 255], 0.5), [128, 128, 128]);
        assert_eq!(blend([10, 20, 30], [10, 20, 30], 0.25), [10, 20, 30]);
    }
}
