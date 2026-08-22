//! Charts, and where they sit.
//!
//! A chart is more than one part in the package: the *chart* part says what it
//! plots, and something else says where it goes — a drawing anchored to cells
//! in a workbook, an inline or anchored `<w:drawing>` in a document. Nothing
//! here is authored from scratch. This is a view over parts that are preserved
//! verbatim, so that a file whose chart we render imperfectly still saves back
//! exactly as it opened.
//!
//! What is modeled is what a *reader* needs: the shape, the series and where
//! their numbers come from, the labels, and the rectangle. Everything else in
//! `chartSpace` — gradients, 3-D rotation, trendlines, the several dozen
//! elements of axis formatting — is not, and is carried through untouched.

/// EMUs (English Metric Units) per point. Office measures drawings in these:
/// 914,400 to the inch and 12,700 to the point, chosen so that both inches and
/// centimetres divide exactly.
pub const EMU_PER_POINT: f64 = 12_700.0;

/// One corner of an anchor: a cell, plus an offset into it.
///
/// The offset is why a chart does not jump when a column is widened by a pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AnchorPoint {
    pub col: u32,
    pub col_offset: i64,
    pub row: u32,
    pub row_offset: i64,
}

/// How a drawing is pinned to the grid.
#[derive(Debug, Clone, PartialEq)]
pub enum Anchor {
    /// `<xdr:twoCellAnchor>`: corner to corner, so it resizes with the cells.
    TwoCell { from: AnchorPoint, to: AnchorPoint },
    /// `<xdr:oneCellAnchor>`: pinned at one corner with a fixed size in EMUs.
    OneCell {
        from: AnchorPoint,
        width: i64,
        height: i64,
    },
    /// `<xdr:absoluteAnchor>`: a position on the sheet that ignores the grid.
    Absolute {
        x: i64,
        y: i64,
        width: i64,
        height: i64,
    },
}

impl Anchor {
    pub fn from(&self) -> AnchorPoint {
        match self {
            Anchor::TwoCell { from, .. } | Anchor::OneCell { from, .. } => *from,
            Anchor::Absolute { .. } => AnchorPoint::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ChartKind {
    #[default]
    Bar,
    Line,
    Pie,
    Doughnut,
    Area,
    Scatter,
    Radar,
    /// A type we can name but do not draw. Rendered as a placeholder rather
    /// than as nothing, so the user can see that something is there.
    Other(String),
}

impl ChartKind {
    /// The element name a chart type is written as, without the `c:` prefix.
    pub fn from_element(name: &str) -> Option<ChartKind> {
        Some(match name {
            "barChart" | "bar3DChart" => ChartKind::Bar,
            "lineChart" | "line3DChart" => ChartKind::Line,
            "pieChart" | "pie3DChart" | "ofPieChart" => ChartKind::Pie,
            "doughnutChart" => ChartKind::Doughnut,
            "areaChart" | "area3DChart" => ChartKind::Area,
            "scatterChart" | "bubbleChart" => ChartKind::Scatter,
            "radarChart" => ChartKind::Radar,
            "stockChart" | "surfaceChart" | "surface3DChart" => ChartKind::Other(name.to_string()),
            _ => return None,
        })
    }

    /// True when the categories run along the bottom and the values up the side.
    pub fn has_axes(&self) -> bool {
        !matches!(self, ChartKind::Pie | ChartKind::Doughnut)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Grouping {
    #[default]
    Clustered,
    Stacked,
    PercentStacked,
    /// `standard` on a line or area chart: series drawn over each other.
    Standard,
}

impl Grouping {
    pub fn from_val(text: &str) -> Grouping {
        match text {
            "stacked" => Grouping::Stacked,
            "percentStacked" => Grouping::PercentStacked,
            "clustered" => Grouping::Clustered,
            _ => Grouping::Standard,
        }
    }

    pub fn stacked(self) -> bool {
        matches!(self, Grouping::Stacked | Grouping::PercentStacked)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegendPosition {
    Top,
    Bottom,
    Left,
    Right,
    TopRight,
}

impl LegendPosition {
    pub fn from_val(text: &str) -> LegendPosition {
        match text {
            "t" => LegendPosition::Top,
            "b" => LegendPosition::Bottom,
            "l" => LegendPosition::Left,
            "tr" => LegendPosition::TopRight,
            _ => LegendPosition::Right,
        }
    }
}

/// One plotted series.
///
/// Both the reference *and* the cached numbers are kept. The reference is what
/// makes a chart redraw when a cell changes; the cache is what lets it draw at
/// all when the reference names a sheet we could not resolve — a chart whose
/// data lives in another workbook, most often.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Series {
    /// `<c:tx>` resolved to text, when the file cached it.
    pub name: Option<String>,
    /// The formula behind the name, e.g. `Sheet1!$B$1`.
    pub name_ref: Option<String>,
    pub values_ref: Option<String>,
    /// What the producing application last computed, index by index.
    pub values: Vec<Option<f64>>,
    pub categories_ref: Option<String>,
    pub categories: Vec<String>,
    /// `<a:srgbClr>` on the series' shape properties, as plain sRGB.
    ///
    /// A triple rather than a workbook `Color` because that is all a chart
    /// part ever states here — the theme-coloured case writes `<a:schemeClr>`,
    /// which is not read — and because a document has no workbook theme to
    /// resolve one against.
    pub color: Option<[u8; 3]>,
    /// `<c:marker><c:symbol val>`: the shape this series marks its points
    /// with, when the part names one.
    pub symbol: Symbol,
    /// `<c:smooth>` on a line or scatter series. `None` when the part says
    /// nothing, which Excel reads as *smoothed* for a line chart — the one
    /// boolean in the part whose silence means yes (measured 2026-08-21).
    pub smooth: Option<bool>,
}

/// The shape a series marks its points with: `<c:symbol val>`.
///
/// `Auto` is silence, and Office fills it from its own rotation — diamond,
/// square, triangle, x, star, circle, plus, dash, dot — by the series'
/// index, or by the point's when the chart varies colours by point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Symbol {
    #[default]
    Auto,
    None,
    Circle,
    Square,
    Diamond,
    Triangle,
    X,
    Star,
    Plus,
    Dash,
    Dot,
}

impl Symbol {
    pub fn from_val(text: &str) -> Symbol {
        match text {
            "none" => Symbol::None,
            "circle" => Symbol::Circle,
            "square" | "picture" => Symbol::Square,
            "diamond" => Symbol::Diamond,
            "triangle" => Symbol::Triangle,
            "x" => Symbol::X,
            "star" => Symbol::Star,
            "plus" => Symbol::Plus,
            "dash" => Symbol::Dash,
            "dot" => Symbol::Dot,
            _ => Symbol::Auto,
        }
    }

    /// Office's automatic rotation, which is what `Auto` comes to for the
    /// n-th series or point.
    pub fn automatic(index: usize) -> Symbol {
        const ROTATION: [Symbol; 9] = [
            Symbol::Diamond,
            Symbol::Square,
            Symbol::Triangle,
            Symbol::X,
            Symbol::Star,
            Symbol::Circle,
            Symbol::Plus,
            Symbol::Dash,
            Symbol::Dot,
        ];
        ROTATION[index % ROTATION.len()]
    }
}

/// One paint decision the chart part may state: the chart area's fill, or its
/// border.
///
/// Three states because the file has three: an explicit colour, an explicit
/// *nothing* (`<a:noFill/>`, which LibreOffice writes for every chart it puts
/// in a document), and silence — for which Office supplies its own default,
/// and so does the painter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Paint {
    /// The part says nothing; the painter uses its default.
    #[default]
    Auto,
    /// `<a:noFill/>`: deliberately not painted.
    None,
    /// `<a:solidFill><a:srgbClr/></a:solidFill>`.
    Rgb([u8; 3]),
}

/// Where an axis puts the stubs that mark its major units.
///
/// `<c:majorTickMark val>`. The schema's own default is `cross`, but every
/// producer writes the element out on every axis, so silence here means a part
/// nobody stated ticks for — and, like the plot area's border, it draws
/// nothing rather than ink of our invention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TickMark {
    #[default]
    None,
    /// Outside the plot area, which is what Word writes for both of the
    /// sample's axes and what makes the small stubs beyond its frame.
    Out,
    In,
    /// Both ways from the axis line.
    Cross,
}

impl TickMark {
    pub fn from_val(text: &str) -> TickMark {
        match text {
            "out" => TickMark::Out,
            "in" => TickMark::In,
            "cross" => TickMark::Cross,
            _ => TickMark::None,
        }
    }

    /// How far a stub of this kind reaches into the plot area, and how far out
    /// of it.
    pub fn reach(self, length: f64) -> (f64, f64) {
        match self {
            TickMark::None => (0.0, 0.0),
            TickMark::In => (length, 0.0),
            TickMark::Out => (0.0, length),
            TickMark::Cross => (length, length),
        }
    }
}

/// One axis's own look.
///
/// Not its scale: what to plot over is worked out from the data, because a
/// chart in a document carries no cells to recompute a stated maximum against
/// and a stated one that disagrees with the cache draws bars off the top.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Axis {
    /// `<c:spPr><a:ln>`: the axis line, and the ink its ticks are drawn in.
    pub line: Paint,
    pub major_tick: TickMark,
    /// `<c:delete val="1"/>`: an axis the author took away. Its ticks are not
    /// drawn. Its labels still are — a separate gap, and a wider one, since
    /// the gutter they stand in is what the plot is measured against.
    pub deleted: bool,
    /// `<c:majorGridlines>`: a line across the plot at every major unit.
    /// Drawn only when stated — Office draws none for an axis that has none,
    /// and a chart of our invention with lines Excel would not show is a
    /// chart that looks different in every application that opens it.
    pub gridlines: bool,
}

/// What a chart part plots, which is everything a painter needs.
///
/// Separate from where it sits, because where it sits is the one thing the two
/// formats do not share: a workbook anchors a chart to cells, a document puts
/// it in a line of text or on the page. This half is identical in both, and is
/// what the reader produces.
#[derive(Debug, Clone, PartialEq)]
pub struct Plot {
    pub kind: ChartKind,
    pub grouping: Grouping,
    /// `<c:barDir val="bar"/>`: bars lie down, columns stand up.
    pub horizontal: bool,
    pub title: Option<String>,
    /// The formula behind the title, when it is a cell rather than typed text.
    pub title_ref: Option<String>,
    pub legend: Option<LegendPosition>,
    /// The chart area's own background, from the `chartSpace` shape properties.
    pub area_fill: Paint,
    /// The border around the whole chart, from the same place.
    pub area_line: Paint,
    /// The plot area's own background, from the `plotArea` shape properties.
    pub plot_fill: Paint,
    /// The box around the plot area — the frame Word draws around the bars
    /// when the part states one. Office's modern default is none, so `Auto`
    /// draws nothing here, unlike the chart area's.
    pub plot_line: Paint,
    /// The category axis: the one along the bottom of a column chart, whose
    /// ticks stand at the boundaries between categories rather than under
    /// them.
    pub cat_axis: Axis,
    /// The value axis, up the side, whose ticks stand at the gridlines.
    pub val_axis: Axis,
    /// `<c:gapWidth>`: the space between category groups, as a percentage of
    /// one bar's width. Office's default is 150 — the gap is one and a half
    /// bars wide — and it is what sets how fat the bars are.
    pub gap: f64,
    /// `<c:scatterStyle>`: whether a scatter's points are joined by lines
    /// (`lineMarker`, `smoothMarker`) or stand alone (`marker`), which is what
    /// Excel writes for the plain scatter it inserts.
    pub scatter_lines: bool,
    /// `<c:scatterStyle>` again: whether those lines bend (`smooth`,
    /// `smoothMarker`).
    pub scatter_smooth: bool,
    /// Whether a line, scatter or radar marks its points at all: the chart's
    /// own `<c:marker val>`, a scatter style that names markers, or a radar
    /// style that does. Each series may still say `none` for itself.
    pub markers: bool,
    /// `<c:varyColors>`: every point in its own colour. The part's silence
    /// means yes — Excel gives a one-series chart that says nothing a
    /// different colour per bar, and per marker on a scatter. With more than
    /// one series it means nothing; the series keep their colours.
    pub vary_colors: bool,
    /// `<c:holeSize>`: a doughnut's hole as a percentage of its radius. Excel
    /// inserts 75 and draws a part that states none with *no* hole — the
    /// schema's ten is not what it does (measured 2026-08-21).
    pub hole: f64,
    /// `<c:crossBetween val="between"/>` on the value axis: categories stand
    /// in the middle of their slots rather than at the tick marks. `None`
    /// when unstated, for which Excel chooses by chart type — an area runs
    /// edge to edge, everything else stands between.
    pub between: Option<bool>,
    pub series: Vec<Series>,
}

impl Default for Plot {
    fn default() -> Plot {
        Plot {
            kind: ChartKind::default(),
            grouping: Grouping::default(),
            horizontal: false,
            title: None,
            title_ref: None,
            legend: None,
            area_fill: Paint::Auto,
            area_line: Paint::Auto,
            plot_fill: Paint::Auto,
            plot_line: Paint::Auto,
            cat_axis: Axis::default(),
            val_axis: Axis::default(),
            gap: 150.0,
            scatter_lines: false,
            scatter_smooth: false,
            markers: true,
            vary_colors: true,
            hole: 0.0,
            between: None,
            series: Vec::new(),
        }
    }
}

impl Plot {
    /// The categories to label the axis with: the first series that has any.
    ///
    /// Excel writes the same category reference on every series and draws one
    /// axis from it. Taking the first non-empty one matches what it shows.
    pub fn categories(&self) -> &[String] {
        self.series
            .iter()
            .map(|s| s.categories.as_slice())
            .find(|c| !c.is_empty())
            .unwrap_or(&[])
    }

    /// Whether the categories stand in the middle of their slots or at the
    /// ticks: the part's word, or Excel's choice for the kind when it has
    /// none.
    pub fn categories_between(&self) -> bool {
        self.between.unwrap_or(self.kind != ChartKind::Area)
    }

    /// The most points any one series has.
    pub fn points(&self) -> usize {
        self.series
            .iter()
            .map(|s| s.values.len())
            .max()
            .unwrap_or(0)
    }
}

/// Sets on a plot built from nothing what Excel's own Insert states for the
/// kind: gridlines up the value axis, no colour varied except a pie's, a
/// line's points unmarked and its segments straight, a doughnut's hole at
/// three quarters. What the reader finds in an Excel-made part, in other
/// words — so a chart Calx draws the moment it is inserted is the chart Excel
/// draws from the file it saves.
pub fn excel_defaults(plot: &mut Plot) {
    let pie = matches!(plot.kind, ChartKind::Pie | ChartKind::Doughnut);
    plot.vary_colors = pie;
    plot.markers = true;
    plot.val_axis.gridlines = plot.kind.has_axes();
    plot.hole = if plot.kind == ChartKind::Doughnut {
        75.0
    } else {
        0.0
    };
    for series in &mut plot.series {
        if matches!(plot.kind, ChartKind::Line | ChartKind::Radar) {
            series.symbol = Symbol::None;
        }
        if matches!(plot.kind, ChartKind::Line | ChartKind::Scatter) {
            series.smooth = Some(false);
        }
    }
}

/// A chart on a sheet: what it plots, and which cells it is pinned to.
#[derive(Debug, Clone, PartialEq)]
pub struct Chart {
    /// The chart part's name, so a writer can find the bytes again.
    pub part: String,
    /// The drawing part whose anchor places this chart, and which anchor it
    /// is, counted over *every* anchor in that drawing in document order —
    /// the same identity contract pictures carry, and for the same reason:
    /// it is what survives a save when the anchor itself has changed.
    pub drawing_part: String,
    pub anchor_index: usize,
    pub anchor: Anchor,
    pub plot: Plot,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_dimensional_variants_plot_the_same_way_as_their_flat_twins() {
        // A 3-D bar chart is a bar chart drawn with perspective. Treating it as
        // an unknown type would leave a placeholder where a readable chart could
        // have been.
        assert_eq!(ChartKind::from_element("bar3DChart"), Some(ChartKind::Bar));
        assert_eq!(ChartKind::from_element("pieChart"), Some(ChartKind::Pie));
        assert_eq!(ChartKind::from_element("valAx"), None);
    }

    #[test]
    fn a_pie_has_no_axes_to_draw() {
        assert!(!ChartKind::Pie.has_axes());
        assert!(ChartKind::Bar.has_axes());
    }

    #[test]
    fn the_axis_labels_come_from_whichever_series_has_them() {
        let plot = Plot {
            kind: ChartKind::Bar,
            series: vec![
                Series::default(),
                Series {
                    categories: vec!["Q1".into(), "Q2".into()],
                    values: vec![Some(1.0), Some(2.0), Some(3.0)],
                    ..Default::default()
                },
            ],
            ..Plot::default()
        };
        assert_eq!(plot.categories(), ["Q1", "Q2"]);
        assert_eq!(plot.points(), 3);
    }
}
