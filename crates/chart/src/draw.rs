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

use crate::{Axis, ChartKind, Grouping, LegendPosition, Paint, Plot};

/// The size a chart sets its own text in, before the caller's zoom.
///
/// Word's default, and the one it uses whenever the part names none — which
/// it does not here: the sample's `chartSpace` carries no `txPr`, no
/// typeface, and its package no theme at all, so Word falls back to its
/// built-in theme. See [`FACE`] in `wp_print::ops` for the face that goes
/// with it.
pub const TEXT: f64 = 10.0;

/// How far the plot area stands clear of whatever is beside it — the value
/// labels on one side, the legend on the other — in [`TEXT`] sizes.
///
/// This and the three below are Word's, measured from charts Word rendered
/// itself: probe documents whose chart area carries a visible border, so that
/// the plot rectangle could be read against the chart's own edges rather than
/// guessed from the page. They hold whatever the chart's width, the labels'
/// width, the legend's text, or the number of series — Office's automatic
/// layout is constants, not fractions of the box. In points, at Word's own
/// ten: 15.7 clear, 10.8 of margin, 24.8 below, and 9.0 between a value
/// label and the axis it belongs to.
const CLEAR: f64 = 1.566;

/// The margin at the top of a chart and down its right edge.
const MARGIN: f64 = 1.08;

/// What is kept below the plot: the category labels, and the margin under
/// them. Re-measured 2026-08-19 against the magenta-border probes: 24.79pt
/// on every one of the eight variants — the 22.0 first written here was a
/// misreading, and it showed as a plot ~3pt too tall on the sample.
const FOOTER: f64 = 2.479;

/// How far a value label stands clear of the axis. Whatever [`CLEAR`] has
/// left over after it is the margin at the chart's left edge.
const LABEL_GAP: f64 = 0.904;

/// The stub an axis marks a major unit with.
const TICK: f64 = 0.31;

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
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Plotted {
    pub name: String,
    pub values: Vec<Option<f64>>,
    /// The X each point was given, which only a scatter plots. For every other
    /// kind the X is a category, and this holds whatever of the category cache
    /// happened to be numeric — which the renderers for those kinds never read.
    pub xs: Vec<Option<f64>>,
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
            // A scatter's `<c:xVal>` cache lands in `categories`: the reader
            // keeps one slot for "the axis a point stands on", whatever its
            // element name. Parsed here so the painter plots numbers.
            xs: series
                .categories
                .iter()
                .map(|c| c.trim().parse::<f64>().ok())
                .collect(),
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

    let small = TEXT * style.zoom;
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
    // never draws underneath it. A pie's legend names its slices, not its one
    // series — the categories are what the colours mean there.
    let slices;
    let named = if matches!(plot.kind, ChartKind::Pie | ChartKind::Doughnut)
        && !plot.categories().is_empty()
    {
        slices = plot
            .categories()
            .iter()
            .enumerate()
            .map(|(i, name)| Plotted {
                name: name.clone(),
                rgb: SERIES_COLORS[i % SERIES_COLORS.len()],
                ..Plotted::default()
            })
            .collect::<Vec<_>>();
        slices.as_slice()
    } else {
        series
    };
    if let Some(position) = plot.legend {
        area = legend(&mut out, area, named, style, small, position, measure);
    }
    if area.width < 16.0 || area.height < 16.0 {
        return out;
    }

    match plot.kind {
        ChartKind::Pie | ChartKind::Doughnut => {
            pie(&mut out, area, series, plot.kind == ChartKind::Doughnut);
        }
        ChartKind::Radar => radar(&mut out, area, plot, series, style, small, measure),
        ChartKind::Scatter => scatter(&mut out, area, plot, series, style, small, measure),
        ChartKind::Bar if plot.horizontal => {
            sideways(&mut out, area, plot, series, style, small, measure);
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
    // Word's swatch is a square just over half its text, and stands a quarter
    // clear of it. Its entries are set on a pitch of nearly two — noticeably
    // airier than a line of type needs, and what makes a legend of three read
    // as three separate things rather than a paragraph.
    let swatch = 0.54 * size;
    let gap = 0.26 * size;
    let line = 1.81 * size;
    let mut left = area;
    // Both the swatch and the name stand at the middle of that pitch, which is
    // where Word has them: at this spacing, hanging either from the top of the
    // line puts it visibly above the other.
    let middle = |y: f64, height: f64| y + (line - height) / 2.0;
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
                    rect: Rect::new(x, middle(strip, swatch), swatch, swatch),
                    rgb: entry.rgb,
                    round: 1.0,
                });
                x += swatch + gap;
                let (width, height) = measure.size(&entry.name, size);
                out.push(Prim::Text {
                    at: (x, middle(strip, height)),
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
                    rect: Rect::new(strip.left(), middle(y, swatch), swatch, swatch),
                    rgb: entry.rgb,
                    round: 1.0,
                });
                let height = measure.size(&entry.name, size).1;
                out.push(Prim::Text {
                    at: (strip.left() + swatch + gap, middle(y, height)),
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
    let percent = plot.grouping == Grouping::PercentStacked;
    // A 100% stack plots each point's share, not its size; the shares are
    // computed here so everything downstream sees a chart of fractions.
    let shares;
    let series = if percent {
        shares = percented(series);
        shares.as_slice()
    } else {
        series
    };
    let (data_low, data_high) = bounds(series, stacked);
    // Excel pins an all-positive 100% stack at exactly 100%, in steps of ten;
    // padding it like any other axis would run the scale to 120%.
    let (low, high, major) = if percent && data_low >= 0.0 {
        (0.0, 1.0, 0.1)
    } else {
        axis(data_low, data_high)
    };
    if !(high - low).is_finite() || high <= low {
        return;
    }

    // Every tick's label, measured before anything is placed: the gutter is as
    // wide as the widest of them, not a guess that pushes the plot around.
    let ticks = ((high - low) / major).round() as usize;
    let labels: Vec<(f64, String)> = (0..=ticks)
        .map(|step| {
            let value = low + major * step as f64;
            if percent {
                (value, format!("{}%", (style.label)(value * 100.0)))
            } else {
                (value, (style.label)(value))
            }
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
    let stub = TICK * size;
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
            at: (area.left() - LABEL_GAP * size - width, y - height / 2.0),
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
        ChartKind::Line => {
            // A stacked line plots each series on top of the running total; a
            // blank contributes nothing to the stack and draws no marker.
            let mut totals = vec![0.0f64; points];
            for entry in series {
                let path: Vec<(f64, f64)> = entry
                    .values
                    .iter()
                    .enumerate()
                    .filter_map(|(i, v)| {
                        let x = area.left() + slot * (i as f64 + 0.5);
                        if stacked {
                            totals[i] += v.unwrap_or(0.0);
                            v.map(|_| (x, y_of(totals[i])))
                        } else {
                            v.map(|v| (x, y_of(v)))
                        }
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
            if stacked {
                // Each series is the band between the running total below it
                // and the total with it added: forward along its own top, back
                // along the series beneath. Bands cannot overlap, so they are
                // drawn opaque in the series' own colour, which is what Office
                // does.
                let mut below = vec![0.0f64; points];
                for entry in series {
                    let mut above = below.clone();
                    for (i, v) in entry.values.iter().enumerate() {
                        above[i] += v.unwrap_or(0.0);
                    }
                    let x_at = |i: usize| area.left() + slot * (i as f64 + 0.5);
                    let mut path: Vec<(f64, f64)> =
                        (0..points).map(|i| (x_at(i), y_of(above[i]))).collect();
                    path.extend((0..points).rev().map(|i| (x_at(i), y_of(below[i]))));
                    if path.len() > 3 {
                        out.push(Prim::Poly {
                            points: path,
                            rgb: entry.rgb,
                            edge: None,
                        });
                    }
                    below = above;
                }
            } else {
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
                // The label's em box centred in the footer band, which is
                // where Word sets them — measured ink margins of 9.3 and 9.1
                // in the 24.8pt band. The em rather than the shaper's line,
                // whose leading would over-centre by half of itself.
                area.bottom() + (FOOTER - 1.0) * size / 2.0,
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

/// Each value as its share of its point's total — what a 100% stack plots.
///
/// The shares are of the absolute total, sign kept, which is how Excel fills
/// a percent stack that has a negative in it. A point whose total is zero has
/// no shares to plot.
fn percented(series: &[Plotted]) -> Vec<Plotted> {
    let points = series.iter().map(|s| s.values.len()).max().unwrap_or(0);
    let totals: Vec<f64> = (0..points)
        .map(|i| {
            series
                .iter()
                .filter_map(|s| s.values.get(i).copied().flatten())
                .map(f64::abs)
                .sum()
        })
        .collect();
    series
        .iter()
        .map(|entry| Plotted {
            values: entry
                .values
                .iter()
                .enumerate()
                .map(|(i, v)| match v {
                    Some(v) if totals[i] > 0.0 => Some(v / totals[i]),
                    _ => None,
                })
                .collect(),
            ..entry.clone()
        })
        .collect()
}

/// Bars that lie down: `<c:barDir val="bar"/>`.
///
/// The same chart as the standing one with its axes traded — categories run
/// up the left edge, values along the bottom — so the layout is the standing
/// chart's transposed: the gutter holds category names instead of numbers,
/// the footer holds numbers instead of names, and the first category sits at
/// the *bottom*, because Excel lays a column chart on its side by turning it
/// anticlockwise.
#[allow(clippy::too_many_arguments)]
fn sideways(
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
    let percent = plot.grouping == Grouping::PercentStacked;
    let shares;
    let series = if percent {
        shares = percented(series);
        shares.as_slice()
    } else {
        series
    };
    let (data_low, data_high) = bounds(series, stacked);
    let (low, high, major) = if percent && data_low >= 0.0 {
        (0.0, 1.0, 0.1)
    } else {
        axis(data_low, data_high)
    };
    if !(high - low).is_finite() || high <= low {
        return;
    }

    let ticks = ((high - low) / major).round() as usize;
    let labels: Vec<(f64, String)> = (0..=ticks)
        .map(|step| {
            let value = low + major * step as f64;
            if percent {
                (value, format!("{}%", (style.label)(value * 100.0)))
            } else {
                (value, (style.label)(value))
            }
        })
        .collect();
    // The gutter holds the category names, and is as wide as the widest.
    let widest = plot
        .categories()
        .iter()
        .map(|text| measure.size(text, size).0)
        .fold(0.0f64, f64::max);
    let gutter = widest + CLEAR * size;
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

    let x_of = |value: f64| area.left() + (value - low) / (high - low) * area.width;

    if let Paint::Rgb(rgb) = plot.plot_fill {
        out.push(Prim::Fill {
            rect: area,
            rgb,
            round: 0.0,
        });
    }

    let stub = TICK * size;
    let val_tick = tick_marks(&plot.val_axis, stub, style.grid);
    let cat_tick = tick_marks(&plot.cat_axis, stub, style.grid);

    // A gridline stands at every major unit, its label centred underneath.
    for (value, text) in labels {
        let x = x_of(value);
        out.push(Prim::Line {
            points: vec![(x, area.top()), (x, area.bottom())],
            thickness: 1.0,
            rgb: blend(style.grid, style.background, 0.5),
        });
        if let Some((inside, outside, rgb)) = val_tick {
            out.push(Prim::Line {
                points: vec![(x, area.bottom() - inside), (x, area.bottom() + outside)],
                thickness: 1.0,
                rgb,
            });
        }
        let (width, height) = measure.size(&text, size);
        out.push(Prim::Text {
            at: (x - width / 2.0, area.bottom() + (footer - height) / 2.0),
            size,
            text,
            rgb: style.text,
        });
    }
    // The category axis is the vertical one here, standing at zero.
    let base = x_of(0.0f64.clamp(low, high));
    if let Some(rgb) = ink(plot.cat_axis.line, style.grid) {
        out.push(Prim::Line {
            points: vec![(base, area.top()), (base, area.bottom())],
            thickness: 1.0,
            rgb,
        });
    }
    if let Paint::Rgb(rgb) = plot.plot_line {
        out.push(Prim::Frame {
            rect: area,
            rgb,
            thickness: 1.0,
            round: 0.0,
        });
    }

    let slot = area.height / points as f64;
    if let Some((inside, outside, rgb)) = cat_tick {
        for step in 0..=points {
            let y = area.bottom() - slot * step as f64;
            out.push(Prim::Line {
                points: vec![(area.left() - outside, y), (area.left() + inside, y)],
                thickness: 1.0,
                rgb,
            });
        }
    }

    // The first category at the bottom, and within a group the first series
    // nearest the axis — both ends of the anticlockwise turn.
    let lanes = if stacked { 1 } else { series.len().max(1) };
    let bar = (slot / (lanes as f64 + plot.gap.max(0.0) / 100.0)).max(1.0);
    let mut totals = vec![0.0f64; points];
    for (index, entry) in series.iter().enumerate() {
        for (i, value) in entry.values.iter().enumerate() {
            let Some(value) = value else { continue };
            let centre = area.bottom() - slot * (i as f64 + 0.5);
            let y = if stacked {
                centre + bar / 2.0
            } else {
                centre + bar * lanes as f64 / 2.0 - bar * index as f64
            };
            let (from, to) = if stacked {
                let start = totals[i];
                totals[i] += value;
                (start, totals[i])
            } else {
                (0.0f64.clamp(low, high), *value)
            };
            out.push(Prim::Fill {
                rect: Rect::from_corners((x_of(from), y), (x_of(to), y - bar)),
                rgb: entry.rgb,
                round: 0.0,
            });
        }
    }

    // Category names in the gutter, right-aligned against the axis and
    // centred on their slot, thinned when the slots are shorter than a line.
    let step = ((measure.size("M", size).1 / slot).ceil() as usize).max(1);
    for (i, label) in plot.categories().iter().enumerate().step_by(step) {
        if label.is_empty() {
            continue;
        }
        let (width, height) = measure.size(label, size);
        out.push(Prim::Text {
            at: (
                area.left() - LABEL_GAP * size - width,
                area.bottom() - slot * (i as f64 + 0.5) - height / 2.0,
            ),
            size,
            text: label.clone(),
            rgb: style.text,
        });
    }
}

/// A radar: one spoke per category, each series a closed line over them.
#[allow(clippy::too_many_arguments)]
fn radar(
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
    let (data_low, data_high) = bounds(series, false);
    let (low, high, major) = axis(data_low, data_high);
    if !(high - low).is_finite() || high <= low {
        return;
    }

    let centre = area.center();
    // Room around the web for one line of category labels.
    let radius = (area.width.min(area.height) / 2.0 - 1.8 * size).max(4.0);
    // The first category at the top, the rest clockwise: Excel's order.
    let spoke = |k: usize, value: f64| {
        let angle = -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * k as f64 / points as f64;
        let r = (value - low) / (high - low) * radius;
        (centre.0 + angle.cos() * r, centre.1 + angle.sin() * r)
    };

    // The web: a ring at every major unit, and a spoke to every category.
    let faint = blend(style.grid, style.background, 0.5);
    let rings = ((high - low) / major).round() as usize;
    for ring in 1..=rings {
        let value = low + major * ring as f64;
        let mut path: Vec<(f64, f64)> = (0..points).map(|k| spoke(k, value)).collect();
        path.push(path[0]);
        out.push(Prim::Line {
            points: path,
            thickness: 1.0,
            rgb: faint,
        });
    }
    for k in 0..points {
        out.push(Prim::Line {
            points: vec![centre, spoke(k, high)],
            thickness: 1.0,
            rgb: faint,
        });
    }

    // The scale runs up the first spoke, its labels beside it, thinned to
    // taste like any axis's: only as many as the radius has room for.
    if !plot.val_axis.deleted {
        let step = ((measure.size("0", size).1 * rings as f64 / radius).ceil() as usize).max(1);
        for ring in (0..=rings).step_by(step) {
            let value = low + major * ring as f64;
            let text = (style.label)(value);
            let (width, height) = measure.size(&text, size);
            let (x, y) = spoke(0, value);
            out.push(Prim::Text {
                at: (x - width - 0.3 * size, y - height / 2.0),
                size,
                text,
                rgb: style.text,
            });
        }
    }

    // Category names just past their spokes, pulled toward whichever side of
    // the web they stand on so they never lie across it.
    for (k, label) in plot.categories().iter().enumerate().take(points) {
        if label.is_empty() {
            continue;
        }
        let (width, height) = measure.size(label, size);
        let angle = -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * k as f64 / points as f64;
        let r = radius + 0.5 * size;
        let x = centre.0 + angle.cos() * r + (angle.cos() - 1.0) / 2.0 * width;
        let y = centre.1 + angle.sin() * r + (angle.sin() - 1.0) / 2.0 * height;
        out.push(Prim::Text {
            at: (x, y),
            size,
            text: label.clone(),
            rgb: style.text,
        });
    }

    for entry in series {
        let path: Vec<(f64, f64)> = entry
            .values
            .iter()
            .enumerate()
            .filter_map(|(k, v)| v.map(|v| spoke(k, v)))
            .collect();
        if path.len() > 1 {
            let mut closed = path.clone();
            closed.push(path[0]);
            out.push(Prim::Line {
                points: closed,
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

/// The axis Office draws over a scatter's values, which unlike a bar's need
/// not include zero.
///
/// The rule is Excel's own (KB 211119): zero joins the axis only when the
/// data's spread is more than a sixth of its magnitude — years plot as years,
/// not as bars from nought — and otherwise the axis starts half a spread
/// below the data. The ends land on multiples of the major unit.
fn scatter_axis(low: f64, high: f64) -> (f64, f64, f64) {
    let (mut low, mut high) = (low.min(high), high.max(low));
    if low == high {
        low -= 0.5;
        high += 0.5;
    }
    let spread = high - low;
    if low > 0.0 {
        if spread > high / 6.0 {
            low = 0.0;
        } else {
            low -= spread / 2.0;
        }
    } else if high < 0.0 {
        if spread > -low / 6.0 {
            high = 0.0;
        } else {
            high += spread / 2.0;
        }
    }
    let (_, _, major) = axis(low, high);
    (
        (low / major).floor() * major,
        (high / major).ceil() * major,
        major,
    )
}

/// An X-Y scatter: every point plotted where its own two numbers put it.
#[allow(clippy::too_many_arguments)]
fn scatter(
    out: &mut Vec<Prim>,
    area: Rect,
    plot: &Plot,
    series: &[Plotted],
    style: &Style,
    size: f64,
    measure: &mut dyn Measure,
) {
    // A point missing its X gets counted instead: Excel plots a series with
    // no `<c:xVal>` against 1, 2, 3…
    let x_at =
        |entry: &Plotted, i: usize| entry.xs.get(i).copied().flatten().unwrap_or((i + 1) as f64);
    let dots: Vec<Vec<(f64, f64)>> = series
        .iter()
        .map(|entry| {
            entry
                .values
                .iter()
                .enumerate()
                .filter_map(|(i, v)| v.map(|v| (x_at(entry, i), v)))
                .collect()
        })
        .collect();
    let flat: Vec<(f64, f64)> = dots.iter().flatten().copied().collect();
    if flat.is_empty() {
        return;
    }
    let fold = |pick: fn((f64, f64)) -> f64, seed: f64, better: fn(f64, f64) -> f64| {
        flat.iter().fold(seed, |acc, p| better(acc, pick(*p)))
    };
    let (x_low, x_high, x_major) = scatter_axis(
        fold(|p| p.0, f64::INFINITY, f64::min),
        fold(|p| p.0, f64::NEG_INFINITY, f64::max),
    );
    let (y_low, y_high, y_major) = scatter_axis(
        fold(|p| p.1, f64::INFINITY, f64::min),
        fold(|p| p.1, f64::NEG_INFINITY, f64::max),
    );
    if !(x_high - x_low).is_finite() || !(y_high - y_low).is_finite() {
        return;
    }

    let y_ticks = ((y_high - y_low) / y_major).round() as usize;
    let y_labels: Vec<(f64, String)> = (0..=y_ticks)
        .map(|step| {
            let value = y_low + y_major * step as f64;
            (value, (style.label)(value))
        })
        .collect();
    let widest = y_labels
        .iter()
        .map(|(_, text)| measure.size(text, size).0)
        .fold(0.0f64, f64::max);
    let gutter = widest + CLEAR * size;
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

    let x_of = |value: f64| area.left() + (value - x_low) / (x_high - x_low) * area.width;
    let y_of = |value: f64| area.bottom() - (value - y_low) / (y_high - y_low) * area.height;

    if let Paint::Rgb(rgb) = plot.plot_fill {
        out.push(Prim::Fill {
            rect: area,
            rgb,
            round: 0.0,
        });
    }

    // Horizontal gridlines and their labels, as on any value axis. Both of a
    // scatter's axes are value axes; the one whose look the model keeps
    // stands for the pair.
    let stub = TICK * size;
    let val_tick = tick_marks(&plot.val_axis, stub, style.grid);
    for (value, text) in y_labels {
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
        let (width, height) = measure.size(&text, size);
        out.push(Prim::Text {
            at: (area.left() - LABEL_GAP * size - width, y - height / 2.0),
            size,
            text,
            rgb: style.text,
        });
    }
    // The bottom axis at Y's floor, with a label at every X major.
    let base = y_of(0.0f64.clamp(y_low, y_high));
    if let Some(rgb) = ink(plot.val_axis.line, style.grid) {
        out.push(Prim::Line {
            points: vec![(area.left(), base), (area.right(), base)],
            thickness: 1.0,
            rgb,
        });
    }
    if let Paint::Rgb(rgb) = plot.plot_line {
        out.push(Prim::Frame {
            rect: area,
            rgb,
            thickness: 1.0,
            round: 0.0,
        });
    }
    let x_ticks = ((x_high - x_low) / x_major).round() as usize;
    for step in 0..=x_ticks {
        let value = x_low + x_major * step as f64;
        let text = (style.label)(value);
        let (width, height) = measure.size(&text, size);
        out.push(Prim::Text {
            at: (
                x_of(value) - width / 2.0,
                area.bottom() + (footer - height) / 2.0,
            ),
            size,
            text,
            rgb: style.text,
        });
        if let Some((inside, outside, rgb)) = val_tick {
            let x = x_of(value);
            out.push(Prim::Line {
                points: vec![(x, area.bottom() - inside), (x, area.bottom() + outside)],
                thickness: 1.0,
                rgb,
            });
        }
    }

    for (entry, path) in series.iter().zip(&dots) {
        let path: Vec<(f64, f64)> = path.iter().map(|&(x, y)| (x_of(x), y_of(y))).collect();
        if plot.scatter_lines && path.len() > 1 {
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
                ..Default::default()
            },
            Plotted {
                name: "b".into(),
                values: vec![Some(30.0), Some(40.0)],
                rgb: [0, 0, 255],
                ..Default::default()
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
        // Word's own length, and reaching out of the plot, not into it.
        for stub in &value_stubs {
            assert!(
                (stub[1].0 - stub[0].0 - TICK * TEXT).abs() < 1e-9,
                "{stub:?}"
            );
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
        // Swatch + gap + "Column 1" at the test measurer's half-size,
        // measured from the chart's right margin — not 28% of the chart.
        let area_right = 400.0 - MARGIN * TEXT;
        let block = 0.54 * TEXT + 0.26 * TEXT;
        let width = block + 8.0 * 0.5 * TEXT;
        assert!(
            (entry.0 - (area_right - width + block)).abs() < 0.001,
            "the column starts at its measured width: {}",
            entry.0
        );
        // One entry on Word's pitch, the block centred in the area between
        // the top and bottom margins, and the name centred again within its
        // own line — not hung from the top of the chart.
        let inner = 300.0 - MARGIN * TEXT * 2.0;
        let line = 1.81 * TEXT;
        assert!(
            (entry.1 - (MARGIN * TEXT + (inner - line) / 2.0 + (line - TEXT) / 2.0)).abs() < 0.001,
            "centred vertically: {}",
            entry.1
        );

        // And the swatch stands at the same middle, so the two read as one
        // entry rather than as a square with a caption beside it.
        let swatch = prims
            .iter()
            .find_map(|p| match p {
                Prim::Fill { rect, rgb, .. } if *rgb == SERIES_COLORS[0] && rect.width < 20.0 => {
                    Some(*rect)
                }
                _ => None,
            })
            .expect("the legend swatch");
        assert!(
            (swatch.center().1 - (entry.1 + TEXT / 2.0)).abs() < 0.001,
            "swatch {} against name {}",
            swatch.center().1,
            entry.1 + TEXT / 2.0
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
        // wide: half the size, at the test measurer's half-size.
        let size = TEXT;
        let widest = 0.5 * size;
        let close = |a: f64, b: f64, what: &str| {
            assert!((a - b).abs() < 0.001, "{what}: {a} wanted {b}");
        };
        close(plot_rect.left(), widest + CLEAR * size, "left");
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
        let column = 0.54 * size + 0.26 * size + 8.0 * widest;
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

    fn dots(prims: &[Prim], rgb: [u8; 3]) -> Vec<(f64, f64)> {
        prims
            .iter()
            .filter_map(|prim| match prim {
                Prim::Dot { at, rgb: ink, .. } if *ink == rgb => Some(*at),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_bar_that_lies_down_grows_sideways_from_one_baseline() {
        // `barDir val="bar"`: for as long as bars have been drawn, a chart
        // that asked for them lying down got them standing up.
        let mut flat = plot(ChartKind::Bar);
        flat.horizontal = true;
        flat.series[0].values = vec![Some(1.0), Some(2.0), Some(3.0)];
        let series = cached_series(&flat);
        let prims = primitives(
            Rect::new(0.0, 0.0, 400.0, 300.0),
            &flat,
            &series,
            &style(),
            &mut Half,
        );
        let bars = bars(&prims, SERIES_COLORS[0]);
        assert_eq!(bars.len(), 3);
        assert!(
            bars.windows(2).all(|w| w[0].left() == w[1].left()),
            "every bar grows from the same axis"
        );
        assert!(
            bars[0].width < bars[1].width && bars[1].width < bars[2].width,
            "longer values, longer bars"
        );
        assert!(
            bars.windows(2)
                .all(|w| (w[0].height - w[1].height).abs() < 1e-6),
            "and all of them equally fat"
        );
        // The first category at the bottom: a column chart laid on its side
        // by an anticlockwise turn, which is the way Excel lays it.
        assert!(bars[0].center().1 > bars[2].center().1);
    }

    #[test]
    fn a_percent_stack_plots_shares_and_labels_the_axis_in_percent() {
        let mut plot = plot(ChartKind::Bar);
        plot.grouping = Grouping::PercentStacked;
        plot.series[0].values = vec![Some(10.0)];
        plot.series.push(Series {
            values: vec![Some(90.0)],
            ..Series::default()
        });
        let series = cached_series(&plot);
        let prims = primitives(
            Rect::new(0.0, 0.0, 400.0, 300.0),
            &plot,
            &series,
            &style(),
            &mut Half,
        );
        let small = bars(&prims, SERIES_COLORS[0]);
        let large = bars(&prims, SERIES_COLORS[1]);
        assert_eq!((small.len(), large.len()), (1, 1));
        assert!(
            (large[0].height / small[0].height - 9.0).abs() < 1e-6,
            "10 and 90 plot as a tenth and nine tenths, not as sizes"
        );
        assert!(
            prims
                .iter()
                .any(|p| matches!(p, Prim::Text { text, .. } if text == "100%")),
            "the axis reads in percent"
        );
    }

    #[test]
    fn a_stacked_area_stands_each_band_on_the_one_below() {
        let mut plot = plot(ChartKind::Area);
        plot.grouping = Grouping::Stacked;
        plot.series[0].values = vec![Some(1.0), Some(1.0)];
        plot.series.push(Series {
            values: vec![Some(1.0), Some(1.0)],
            ..Series::default()
        });
        let series = cached_series(&plot);
        let prims = primitives(
            Rect::new(0.0, 0.0, 400.0, 300.0),
            &plot,
            &series,
            &style(),
            &mut Half,
        );
        // Stacked bands are opaque in the series' own colour — they cannot
        // overlap — where the overlapping standard areas are blended.
        let band_top = |rgb: [u8; 3]| {
            prims
                .iter()
                .find_map(|p| match p {
                    Prim::Poly {
                        points, rgb: ink, ..
                    } if *ink == rgb => points.iter().map(|p| p.1).min_by(f64::total_cmp),
                    _ => None,
                })
                .expect("a band")
        };
        assert!(
            band_top(SERIES_COLORS[1]) < band_top(SERIES_COLORS[0]),
            "the second band rides on the first"
        );
    }

    #[test]
    fn a_radar_closes_each_series_into_a_ring_over_its_spokes() {
        // Radar fell through to the bar renderer, which drew a radar file as
        // a bar chart with the wrong everything.
        let mut plot = plot(ChartKind::Radar);
        plot.series[0].values = vec![Some(1.0), Some(2.0), Some(3.0)];
        let series = cached_series(&plot);
        let prims = primitives(
            Rect::new(0.0, 0.0, 400.0, 300.0),
            &plot,
            &series,
            &style(),
            &mut Half,
        );
        let ring = prims
            .iter()
            .find_map(|p| match p {
                Prim::Line { points, rgb, .. } if *rgb == SERIES_COLORS[0] => Some(points.clone()),
                _ => None,
            })
            .expect("the series line");
        assert_eq!(ring.len(), 4, "three points, closed");
        assert_eq!(ring.first(), ring.last(), "closed");
        assert_eq!(dots(&prims, SERIES_COLORS[0]).len(), 3);
        assert!(bars(&prims, SERIES_COLORS[0]).is_empty(), "no bars");
    }

    #[test]
    fn a_scatter_puts_each_point_at_its_own_x() {
        // Scatter shared the line renderer, which spaced the points evenly
        // along the categories and threw their X values away.
        let mut plot = plot(ChartKind::Scatter);
        plot.series[0].categories = vec!["1".into(), "2".into(), "4".into()];
        plot.series[0].values = vec![Some(5.0), Some(5.0), Some(5.0)];
        let series = cached_series(&plot);
        let prims = primitives(
            Rect::new(0.0, 0.0, 400.0, 300.0),
            &plot,
            &series,
            &style(),
            &mut Half,
        );
        let dots = dots(&prims, SERIES_COLORS[0]);
        assert_eq!(dots.len(), 3);
        assert!(
            dots.windows(2).all(|w| (w[0].1 - w[1].1).abs() < 1e-6),
            "equal values, one height"
        );
        let (a, b) = (dots[1].0 - dots[0].0, dots[2].0 - dots[1].0);
        assert!(
            (b - 2.0 * a).abs() < 1e-6,
            "the gap from 2 to 4 is twice the gap from 1 to 2: {a} then {b}"
        );
        assert!(
            !prims
                .iter()
                .any(|p| matches!(p, Prim::Line { rgb, .. } if *rgb == SERIES_COLORS[0])),
            "plain scatter draws markers, not a line"
        );

        // `<c:scatterStyle val="lineMarker"/>` is what joins them.
        plot.scatter_lines = true;
        let joined = primitives(
            Rect::new(0.0, 0.0, 400.0, 300.0),
            &plot,
            &series,
            &style(),
            &mut Half,
        );
        assert!(joined
            .iter()
            .any(|p| matches!(p, Prim::Line { rgb, points, .. }
                if *rgb == SERIES_COLORS[0] && points.len() == 3)));
    }

    #[test]
    fn a_pies_legend_names_its_slices_and_not_its_one_series() {
        let mut pie = plot(ChartKind::Pie);
        pie.legend = Some(LegendPosition::Right);
        pie.series[0].name = Some("Sales".to_string());
        pie.series[0].values = vec![Some(1.0), Some(2.0)];
        pie.series[0].categories = vec!["Q1".into(), "Q2".into()];
        let series = cached_series(&pie);
        let prims = primitives(
            Rect::new(0.0, 0.0, 400.0, 300.0),
            &pie,
            &series,
            &style(),
            &mut Half,
        );
        let texts: Vec<&String> = prims
            .iter()
            .filter_map(|p| match p {
                Prim::Text { text, .. } => Some(text),
                _ => None,
            })
            .collect();
        assert!(texts.iter().any(|t| *t == "Q1"), "{texts:?}");
        assert!(texts.iter().any(|t| *t == "Q2"), "{texts:?}");
        assert!(
            !texts.iter().any(|t| *t == "Sales"),
            "the one series' name means nothing beside a pie: {texts:?}"
        );
    }

    #[test]
    fn a_scatter_axis_leaves_zero_out_when_the_data_stands_far_from_it() {
        // Excel's own rule: years plot as years, not as bars from nought.
        let (low, _, _) = scatter_axis(2020.0, 2026.0);
        assert!(low > 2000.0, "not dragged to zero: {low}");
        let (low, high, _) = scatter_axis(3.0, 9.0);
        assert_eq!(low, 0.0, "a spread wider than a sixth reaches back to it");
        assert!(high >= 9.0);
        let (low, high, _) = scatter_axis(5.0, 5.0);
        assert!(low < 5.0 && high > 5.0, "one value still gets a range");
    }
}
