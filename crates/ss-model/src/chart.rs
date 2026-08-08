//! Charts, and where they sit on a sheet.
//!
//! A chart is three parts in the package, not one: the *drawing* says where on
//! the sheet it goes, the *chart* says what it plots, and the worksheet points
//! at the drawing. Nothing here is authored from scratch — this is a view over
//! parts that are preserved verbatim, so that a workbook whose chart we render
//! imperfectly still saves back exactly as it opened.
//!
//! What is modeled is what a *reader* needs: the shape, the series and where
//! their numbers come from, the labels, and the rectangle. Everything else in
//! `chartSpace` — gradients, 3-D rotation, trendlines, the several dozen
//! elements of axis formatting — is not, and is carried through untouched.

use crate::color::Color;

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
    pub color: Option<Color>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chart {
    /// The chart part's name, so a writer can find the bytes again.
    pub part: String,
    pub anchor: Anchor,
    pub kind: ChartKind,
    pub grouping: Grouping,
    /// `<c:barDir val="bar"/>`: bars lie down, columns stand up.
    pub horizontal: bool,
    pub title: Option<String>,
    /// The formula behind the title, when it is a cell rather than typed text.
    pub title_ref: Option<String>,
    pub legend: Option<LegendPosition>,
    pub series: Vec<Series>,
}

impl Chart {
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

    /// The most points any one series has.
    pub fn points(&self) -> usize {
        self.series
            .iter()
            .map(|s| s.values.len())
            .max()
            .unwrap_or(0)
    }
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
        let chart = Chart {
            part: "/xl/charts/chart1.xml".to_string(),
            anchor: Anchor::TwoCell {
                from: AnchorPoint::default(),
                to: AnchorPoint::default(),
            },
            kind: ChartKind::Bar,
            grouping: Grouping::Clustered,
            horizontal: false,
            title: None,
            title_ref: None,
            legend: None,
            series: vec![
                Series::default(),
                Series {
                    categories: vec!["Q1".into(), "Q2".into()],
                    values: vec![Some(1.0), Some(2.0), Some(3.0)],
                    ..Default::default()
                },
            ],
        };
        assert_eq!(chart.categories(), ["Q1", "Q2"]);
        assert_eq!(chart.points(), 3);
    }
}
