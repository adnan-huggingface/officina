//! Chart parts: `xl/charts/chartN.xml`.
//!
//! Read for what it takes to draw the thing — the type, the series, where their
//! numbers come from, and the labels. `chartSpace` holds an enormous amount
//! besides, all of which is preserved verbatim and none of which is modeled.
//!
//! Two things about the shape of the file matter to the reader:
//!
//! **A series carries both a reference and a cache.** `<c:f>` says where the
//! numbers live; `<c:numCache>` says what the producing application last
//! computed. Both are kept — the reference is what lets a chart redraw when a
//! cell changes, and the cache is what lets it draw at all when the reference
//! names a workbook we cannot open.
//!
//! **`<c:idx>` and `<c:order>` are not the same as document order**, and
//! `<c:pt idx="3">` may skip indices. A cache read as a flat list puts every
//! point after a gap in the wrong place.

use quick_xml::events::Event;
use quick_xml::Reader;

use ss_model::chart::{ChartKind, Grouping, LegendPosition, Series};
use ss_model::Color;

use crate::error::{xml_err, Result};
use crate::xml::{attr_raw, attr_text, attr_u32, end_local_name, local_name, push_text};

/// Everything read out of one chart part. The anchor comes from the drawing.
#[derive(Debug, Clone, Default)]
pub(crate) struct ChartBody {
    pub kind: Option<ChartKind>,
    pub grouping: Grouping,
    pub horizontal: bool,
    pub title: Option<String>,
    pub title_ref: Option<String>,
    pub legend: Option<LegendPosition>,
    pub series: Vec<Series>,
}

/// Which reference of a series is being read. `<c:f>` and `<c:pt>` are the same
/// elements under `<c:tx>`, `<c:cat>`, and `<c:val>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    None,
    Name,
    Categories,
    Values,
}

pub(crate) fn parse(part: &str, data: &[u8]) -> Result<ChartBody> {
    let mut reader = Reader::from_reader(data);
    reader.config_mut().check_end_names = false;

    let mut out = ChartBody::default();
    let mut buf = Vec::new();

    let mut series = Series::default();
    let mut in_series = false;
    let mut slot = Slot::None;
    let mut in_title = false;
    let mut title = String::new();
    // `<c:pt idx="..">` may skip, so points are placed by index rather than
    // pushed.
    let mut points: Vec<(u32, String)> = Vec::new();
    let mut point_index = 0u32;
    let mut sink: Option<&'static str> = None;
    let mut text = String::new();
    // A `<c:srgbClr>` inside a series' shape properties is its colour; the same
    // element inside an axis is not.
    let mut depth_in_series_props = 0usize;

    loop {
        let ev = reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err(part, e))?;
        let empty = matches!(ev, Event::Empty(_));

        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => match local_name(e) {
                b"ser" => {
                    series = Series::default();
                    in_series = true;
                }
                b"tx" if in_series => slot = Slot::Name,
                b"cat" | b"xVal" => slot = Slot::Categories,
                b"val" | b"yVal" if in_series => slot = Slot::Values,
                b"f" if slot != Slot::None => {
                    text.clear();
                    sink = Some("f");
                }
                b"pt" => {
                    point_index = attr_u32(e, b"idx").unwrap_or(point_index);
                }
                b"v" => {
                    text.clear();
                    sink = Some("v");
                }
                b"title" => {
                    in_title = true;
                    title.clear();
                }
                b"t" if in_title => {
                    text.clear();
                    sink = Some("t");
                }
                b"legendPos" => {
                    out.legend = attr_text(e, b"val").map(|v| LegendPosition::from_val(&v));
                }
                b"grouping" => {
                    if let Some(v) = attr_text(e, b"val") {
                        out.grouping = Grouping::from_val(&v);
                    }
                }
                b"barDir" => {
                    out.horizontal = attr_raw(e, b"val").as_deref() == Some(b"bar");
                }
                b"spPr" if in_series => depth_in_series_props += 1,
                b"srgbClr" if depth_in_series_props > 0 && series.color.is_none() => {
                    if let Some(hex) = attr_text(e, b"val") {
                        series.color = Color::from_hex(&hex);
                    }
                }
                other => {
                    // The plot-type element is what names the chart. The first
                    // one wins: a combination chart has several, and drawing it
                    // as its first type is closer than drawing nothing.
                    if out.kind.is_none() {
                        if let Some(kind) = ChartKind::from_element(&String::from_utf8_lossy(other))
                        {
                            out.kind = Some(kind);
                        }
                    }
                    if empty {
                        continue;
                    }
                }
            },

            Event::End(ref e) => match end_local_name(e) {
                b"f" | b"v" | b"t" => {
                    let value = std::mem::take(&mut text);
                    match (sink, slot) {
                        (Some("f"), Slot::Name) => series.name_ref = Some(value),
                        (Some("f"), Slot::Categories) => series.categories_ref = Some(value),
                        (Some("f"), Slot::Values) => series.values_ref = Some(value),
                        (Some("v"), _) if slot != Slot::None => {
                            points.push((point_index, value));
                            point_index += 1;
                        }
                        (Some("t"), _) if in_title => {
                            title.push_str(&value);
                        }
                        _ => {}
                    }
                    sink = None;
                }
                b"tx" | b"cat" | b"xVal" | b"val" | b"yVal" => {
                    let collected = std::mem::take(&mut points);
                    match slot {
                        Slot::Name => {
                            // A name cache is one point, and an empty one means
                            // the series is unnamed rather than named "".
                            series.name = collected
                                .into_iter()
                                .next()
                                .map(|(_, v)| v)
                                .filter(|v| !v.is_empty());
                        }
                        Slot::Categories => series.categories = flatten(collected),
                        Slot::Values => {
                            series.values = flatten(collected)
                                .into_iter()
                                .map(|v| v.trim().parse::<f64>().ok())
                                .collect();
                        }
                        Slot::None => {}
                    }
                    slot = Slot::None;
                    point_index = 0;
                }
                b"ser" => {
                    if in_series {
                        out.series.push(std::mem::take(&mut series));
                    }
                    in_series = false;
                    depth_in_series_props = 0;
                }
                b"spPr" => depth_in_series_props = depth_in_series_props.saturating_sub(1),
                b"title" => {
                    in_title = false;
                    let text = title.trim().to_string();
                    if !text.is_empty() {
                        out.title = Some(text);
                    }
                }
                _ => {}
            },

            Event::Eof => break,
            ref other if sink.is_some() => {
                push_text(&mut text, other)?;
            }
            _ => {}
        }
        buf.clear();
    }

    // A title whose text comes from a cell has a `<c:f>` under `<c:tx>` outside
    // any series, which the series reader will not have claimed.
    if out.title.is_none() {
        out.title_ref = None;
    }
    Ok(out)
}

/// Places points at the indices they claim, filling gaps.
///
/// `<c:pt idx="3">` after `idx="1"` means index 2 has no value. Pushing them in
/// document order shifts every point after a gap onto the wrong category.
fn flatten(points: Vec<(u32, String)>) -> Vec<String> {
    let Some(highest) = points.iter().map(|(i, _)| *i).max() else {
        return Vec::new();
    };
    let mut out = vec![String::new(); highest as usize + 1];
    for (index, value) in points {
        out[index as usize] = value;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHART: &str = r#"<?xml version="1.0"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"
              xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
 <c:chart>
  <c:title><c:tx><c:rich><a:p><a:r><a:t>Quarterly </a:t></a:r>
    <a:r><a:t>revenue</a:t></a:r></a:p></c:rich></c:tx></c:title>
  <c:plotArea>
   <c:barChart>
    <c:barDir val="col"/><c:grouping val="clustered"/>
    <c:ser>
     <c:idx val="0"/><c:order val="0"/>
     <c:tx><c:strRef><c:f>Sheet1!$B$1</c:f>
       <c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>Widgets</c:v></c:pt></c:strCache></c:strRef></c:tx>
     <c:spPr><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></c:spPr>
     <c:cat><c:strRef><c:f>Sheet1!$A$2:$A$5</c:f><c:strCache><c:ptCount val="4"/>
       <c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt>
       <c:pt idx="2"><c:v>Q3</c:v></c:pt><c:pt idx="3"><c:v>Q4</c:v></c:pt></c:strCache></c:strRef></c:cat>
     <c:val><c:numRef><c:f>Sheet1!$B$2:$B$5</c:f><c:numCache><c:ptCount val="4"/>
       <c:pt idx="0"><c:v>10</c:v></c:pt><c:pt idx="1"><c:v>20</c:v></c:pt>
       <c:pt idx="3"><c:v>40</c:v></c:pt></c:numCache></c:numRef></c:val>
    </c:ser>
   </c:barChart>
   <c:valAx><c:spPr><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></c:spPr></c:valAx>
  </c:plotArea>
  <c:legend><c:legendPos val="b"/></c:legend>
 </c:chart>
</c:chartSpace>"#;

    fn body() -> ChartBody {
        parse("chart1.xml", CHART.as_bytes()).expect("parses")
    }

    #[test]
    fn the_type_the_title_and_the_legend_come_back() {
        let chart = body();
        assert_eq!(chart.kind, Some(ChartKind::Bar));
        assert!(!chart.horizontal, "barDir=col means columns stand up");
        assert_eq!(chart.grouping, Grouping::Clustered);
        assert_eq!(chart.title.as_deref(), Some("Quarterly revenue"));
        assert_eq!(chart.legend, Some(LegendPosition::Bottom));
        assert!(
            chart.title_ref.is_none(),
            "typed text, not a cell reference"
        );
    }

    #[test]
    fn a_series_keeps_both_where_its_numbers_are_and_what_they_were() {
        let chart = body();
        assert_eq!(chart.series.len(), 1);
        let series = &chart.series[0];
        assert_eq!(series.name.as_deref(), Some("Widgets"));
        assert_eq!(series.values_ref.as_deref(), Some("Sheet1!$B$2:$B$5"));
        assert_eq!(series.categories, ["Q1", "Q2", "Q3", "Q4"]);
    }

    #[test]
    fn a_gap_in_the_cache_stays_a_gap() {
        // The cache skips idx 2. Pushing points in document order would put 40
        // against Q3 and leave Q4 empty — a chart that is wrong rather than
        // incomplete.
        let chart = body();
        assert_eq!(
            chart.series[0].values,
            [Some(10.0), Some(20.0), None, Some(40.0)]
        );
    }

    #[test]
    fn a_series_colour_is_its_own_and_not_the_axiss() {
        // `<a:srgbClr>` appears under the value axis too, later in the file.
        let chart = body();
        assert_eq!(
            chart.series[0].color,
            Some(Color::rgb(0x44, 0x72, 0xC4)),
            "the axis's red must not have overwritten it"
        );
    }
}
