//! Chart parts: `xl/charts/chartN.xml`, `word/charts/chartN.xml`.
//!
//! The same part in both formats — a `<c:chartSpace>` — so it is read once
//! here and the two applications differ only in how they find it and where
//! they put the result.
//!
//! Read for what it takes to draw the thing: the type, the series, where their
//! numbers come from, and the labels. `chartSpace` holds an enormous amount
//! besides, all of which is preserved verbatim and none of which is modeled.
//!
//! Two things about the shape of the file matter to the reader:
//!
//! **A series carries both a reference and a cache.** `<c:f>` says where the
//! numbers live; `<c:numCache>` says what the producing application last
//! computed. Both are kept — the reference is what lets a chart redraw when a
//! cell changes, and the cache is what lets it draw at all when the reference
//! names a workbook we cannot open, which is every chart in a document.
//!
//! **`<c:idx>` and `<c:order>` are not the same as document order**, and
//! `<c:pt idx="3">` may skip indices. A cache read as a flat list puts every
//! point after a gap in the wrong place.

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use crate::{ChartKind, Grouping, LegendPosition, Paint, Plot, Series, Symbol, TickMark};

/// Which reference of a series is being read. `<c:f>` and `<c:pt>` are the same
/// elements under `<c:tx>`, `<c:cat>`, and `<c:val>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    None,
    Name,
    Categories,
    Values,
}

/// Which of the two axes the reader is inside. A category axis may be written
/// as `catAx`, `dateAx` or `serAx` depending on what the categories are; all
/// three run along the bottom of a column chart and are labelled the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    Category,
    Value,
}

/// Whose `<c:spPr>` the reader is inside.
///
/// Nearly everything in a chart part may carry shape properties — a series, a
/// legend, gridlines, a title, a block of data labels — and the elements
/// inside them are indistinguishable from the ones that matter. So the state
/// says *whose*, and everything not named here is nobody's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Props {
    Nobody,
    Area,
    PlotArea,
    Axis,
}

/// Records one paint decision against whatever element's properties it was
/// found in.
fn record(out: &mut Plot, props: Props, axis: Option<Which>, line: bool, paint: Paint) {
    match (props, line) {
        (Props::Area, false) => out.area_fill = paint,
        (Props::Area, true) => out.area_line = paint,
        (Props::PlotArea, false) => out.plot_fill = paint,
        (Props::PlotArea, true) => out.plot_line = paint,
        // An axis's fill paints nothing anyone sees; its line is the axis.
        (Props::Axis, true) => match axis {
            Some(Which::Category) => out.cat_axis.line = paint,
            Some(Which::Value) => out.val_axis.line = paint,
            None => {}
        },
        (Props::Axis, false) | (Props::Nobody, _) => {}
    }
}

/// Reads one `<c:chartSpace>`.
///
/// `None` when the part names no plot type this understands — which is also
/// what a part that is not a chart at all looks like. A caller draws an empty
/// frame for those rather than axes around nothing, and a workbook keeps them
/// out of the model entirely so that an anchor with nothing behind it is not
/// mistaken for a deleted chart.
///
/// A part that will not parse is not an error worth refusing a file over: the
/// bytes are preserved either way, and a chart drawn as an empty frame beats a
/// document that will not open. So the plot comes back as far as it was read.
pub fn plot(data: &[u8]) -> Option<Plot> {
    let mut reader = Reader::from_reader(data);
    reader.config_mut().check_end_names = false;

    let mut out = Plot::default();
    let mut kind: Option<ChartKind> = None;
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
    // The chartSpace's own `<c:spPr>` is the one *outside* `<c:chart>` — the
    // chart area's fill and border. `in_ln` tells fill from line within it.
    let mut in_chart = false;
    let mut in_ln = false;
    // The plotArea's own `<c:spPr>`, and each axis's, are *direct* children
    // among a crowd carrying the same element: gridlines, a title, a label
    // block, the axis text. Depth alone cannot tell them apart — the
    // exclusions can, and `props` is what they decide.
    let mut in_plot_area = false;
    let mut in_axis: Option<Which> = None;
    let mut in_gridlines = false;
    let mut in_text_props = false;
    let mut in_dlbls = false;
    // A single point's own properties (`<c:dPt>`): Excel writes one per
    // slice of a pie, and the first slice's colour is not the series'.
    let mut in_dpt = false;
    let mut props = Props::Nobody;

    while let Ok(ev) = reader.read_event_into(&mut buf) {
        let empty = matches!(ev, Event::Empty(_));

        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => match local_name(e) {
                b"chart" => in_chart = in_chart || !empty,
                b"plotArea" => in_plot_area = in_plot_area || !empty,
                b"catAx" | b"dateAx" | b"serAx" if !empty => in_axis = Some(Which::Category),
                b"valAx" if !empty => in_axis = Some(Which::Value),
                b"majorGridlines" | b"minorGridlines" => {
                    in_gridlines = in_gridlines || !empty;
                    if local_name(e) == b"majorGridlines" {
                        match in_axis {
                            Some(Which::Category) => out.cat_axis.gridlines = true,
                            Some(Which::Value) => out.val_axis.gridlines = true,
                            None => {}
                        }
                    }
                }
                b"txPr" => in_text_props = in_text_props || !empty,
                b"dLbls" => in_dlbls = in_dlbls || !empty,
                b"dPt" => in_dpt = in_dpt || !empty,
                b"majorTickMark" => {
                    let mark = attr_text(e, b"val")
                        .map_or(TickMark::Cross, |v| TickMark::from_val(v.trim()));
                    match in_axis {
                        Some(Which::Category) => out.cat_axis.major_tick = mark,
                        Some(Which::Value) => out.val_axis.major_tick = mark,
                        None => {}
                    }
                }
                b"delete" => {
                    // `val` defaults to true: the element is there to say the
                    // axis is gone, and Office writes `val="0"` to say it is
                    // not.
                    let gone = matches!(attr_text(e, b"val").as_deref(), None | Some("1" | "true"));
                    match in_axis {
                        Some(Which::Category) => out.cat_axis.deleted = gone,
                        Some(Which::Value) => out.val_axis.deleted = gone,
                        None => {}
                    }
                }
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
                    out.horizontal = attr_text(e, b"val").as_deref() == Some("bar");
                }
                b"scatterStyle" => {
                    // Every joined style names its line: `lineMarker`, `line`,
                    // `smooth`, `smoothMarker`. Only `marker` and `none` do not.
                    let style = attr_text(e, b"val").unwrap_or_default();
                    out.scatter_lines = style.contains("ine") || style.contains("mooth");
                    out.scatter_smooth = style.contains("mooth");
                    out.markers = style.contains("arker");
                }
                b"radarStyle" => {
                    out.markers = attr_text(e, b"val").as_deref() == Some("marker");
                }
                // The chart's own `<c:marker val>` says whether a line marks
                // its points; a series' `<c:marker>` holds the symbol instead.
                b"marker" if !in_series => {
                    out.markers = !matches!(attr_text(e, b"val").as_deref(), Some("0" | "false"));
                }
                b"symbol" if in_series => {
                    series.symbol =
                        attr_text(e, b"val").map_or(Symbol::Auto, |v| Symbol::from_val(v.trim()));
                }
                b"smooth" if in_series => {
                    series.smooth = Some(!matches!(
                        attr_text(e, b"val").as_deref(),
                        Some("0" | "false")
                    ));
                }
                b"varyColors" => {
                    out.vary_colors =
                        !matches!(attr_text(e, b"val").as_deref(), Some("0" | "false"));
                }
                b"holeSize" => {
                    if let Some(v) = attr_text(e, b"val").and_then(|v| v.trim().parse().ok()) {
                        out.hole = v;
                    }
                }
                b"crossBetween" => {
                    out.between = Some(attr_text(e, b"val").as_deref() != Some("midCat"));
                }
                b"gapWidth" => {
                    if let Some(v) = attr_text(e, b"val").and_then(|v| v.trim().parse().ok()) {
                        out.gap = v;
                    }
                }
                b"overlap" => {
                    if let Some(v) = attr_text(e, b"val").and_then(|v| v.trim().parse().ok()) {
                        out.overlap = v;
                    }
                }
                b"spPr" if in_series => depth_in_series_props += 1,
                b"spPr" if !in_chart => props = Props::Area,
                b"spPr" if in_axis.is_some() && !in_gridlines && !in_text_props && !in_title => {
                    props = Props::Axis
                }
                b"spPr" if in_plot_area && in_axis.is_none() && !in_title && !in_dlbls => {
                    props = Props::PlotArea
                }
                b"ln" if props != Props::Nobody => in_ln = true,
                b"noFill" if props != Props::Nobody => {
                    record(&mut out, props, in_axis, in_ln, Paint::None);
                }
                b"srgbClr" if props != Props::Nobody => {
                    if let Some(rgb) = attr_text(e, b"val").as_deref().and_then(rgb) {
                        record(&mut out, props, in_axis, in_ln, Paint::Rgb(rgb));
                    }
                }
                b"srgbClr"
                    if depth_in_series_props > 0
                        && !in_dlbls
                        && !in_dpt
                        && series.color.is_none() =>
                {
                    if let Some(hex) = attr_text(e, b"val") {
                        series.color = rgb(&hex);
                    }
                }
                other => {
                    // The plot-type element is what names the chart. The first
                    // one wins: a combination chart has several, and drawing it
                    // as its first type is closer than drawing nothing.
                    if kind.is_none() {
                        kind = ChartKind::from_element(&String::from_utf8_lossy(other));
                    }
                    if empty {
                        continue;
                    }
                }
            },

            Event::End(ref e) => match end_local_name(e) {
                b"chart" => in_chart = false,
                b"plotArea" => in_plot_area = false,
                b"catAx" | b"valAx" | b"dateAx" | b"serAx" => in_axis = None,
                b"majorGridlines" | b"minorGridlines" => in_gridlines = false,
                b"txPr" => in_text_props = false,
                b"dLbls" => in_dlbls = false,
                b"dPt" => in_dpt = false,
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
                b"spPr" => {
                    depth_in_series_props = depth_in_series_props.saturating_sub(1);
                    props = Props::Nobody;
                    in_ln = false;
                }
                b"ln" => in_ln = false,
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
            ref other if sink.is_some() => push_text(&mut text, other),
            _ => {}
        }
        buf.clear();
    }

    // A title whose text comes from a cell has a `<c:f>` under `<c:tx>` outside
    // any series, which the series reader will not have claimed.
    if out.title.is_none() {
        out.title_ref = None;
    }
    out.kind = kind?;
    Some(out)
}

/// Places points at the indices they claim, filling gaps.
///
/// `<c:pt idx="3">` after `idx="1"` means index 2 has no value. Pushing them in
/// document order shifts every point after a gap onto the wrong category.
fn flatten(points: Vec<(u32, String)>) -> Vec<String> {
    let size = points
        .iter()
        .map(|(i, _)| *i as usize + 1)
        .max()
        .unwrap_or(0);
    let mut out = vec![String::new(); size];
    for (index, value) in points {
        out[index as usize] = value;
    }
    out
}

/// `RRGGBB`, or `AARRGGBB` with the alpha dropped — a chart draws opaque.
fn rgb(hex: &str) -> Option<[u8; 3]> {
    let hex = hex.trim();
    let bytes: Vec<u8> = (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16))
        .collect::<Result<_, _>>()
        .ok()?;
    match bytes.len() {
        4 => Some([bytes[1], bytes[2], bytes[3]]),
        3 => Some([bytes[0], bytes[1], bytes[2]]),
        _ => None,
    }
}

fn strip_prefix(name: &[u8]) -> &[u8] {
    match name.iter().position(|b| *b == b':') {
        Some(at) => &name[at + 1..],
        None => name,
    }
}

fn local_name<'a>(e: &'a BytesStart<'a>) -> &'a [u8] {
    strip_prefix(e.name().into_inner())
}

fn end_local_name<'a>(e: &'a BytesEnd<'a>) -> &'a [u8] {
    strip_prefix(e.name().into_inner())
}

fn attr_text(e: &BytesStart<'_>, want: &[u8]) -> Option<String> {
    let mut attrs = e.attributes();
    attrs.with_checks(false);
    for a in attrs.flatten() {
        if strip_prefix(a.key.as_ref()) == want {
            return a.normalized_value(XML_VERSION).ok().map(|v| v.into_owned());
        }
    }
    None
}

fn attr_u32(e: &BytesStart<'_>, want: &[u8]) -> Option<u32> {
    attr_text(e, want)?.trim().parse().ok()
}

/// Appends one text event, entity references resolved.
///
/// A title reading `Sales & Costs` arrives as three events — text, a general
/// reference, text — and a reader that takes the raw bytes of each drops the
/// ampersand on the floor.
fn push_text(out: &mut String, ev: &Event<'_>) {
    match ev {
        Event::Text(t) => {
            if let Ok(s) = t.xml_content(XML_VERSION) {
                out.push_str(&s);
            }
        }
        Event::CData(c) => {
            if let Ok(s) = c.xml_content(XML_VERSION) {
                out.push_str(&s);
            }
        }
        Event::GeneralRef(r) => {
            if let Ok(Some(ch)) = r.resolve_char_ref() {
                out.push(ch);
            } else if let Ok(name) = r.decode() {
                match quick_xml::escape::resolve_predefined_entity(&name) {
                    Some(text) => out.push_str(text),
                    // An entity we cannot resolve is written back as it came
                    // in. Losing it outright would corrupt the text; guessing
                    // would corrupt it differently.
                    None => {
                        out.push('&');
                        out.push_str(&name);
                        out.push(';');
                    }
                }
            }
        }
        _ => {}
    }
}

const XML_VERSION: XmlVersion = XmlVersion::Explicit1_0;

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

    fn chart() -> Plot {
        plot(CHART.as_bytes()).expect("parses")
    }

    #[test]
    fn the_type_the_title_and_the_legend_come_back() {
        let chart = chart();
        assert_eq!(chart.kind, ChartKind::Bar);
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
        let chart = chart();
        assert_eq!(chart.series.len(), 1);
        let series = &chart.series[0];
        assert_eq!(series.name.as_deref(), Some("Widgets"));
        assert_eq!(series.values_ref.as_deref(), Some("Sheet1!$B$2:$B$5"));
        assert_eq!(series.categories, ["Q1", "Q2", "Q3", "Q4"]);
    }

    #[test]
    fn a_scatter_reads_its_xs_into_the_category_slot_and_its_style_into_lines() {
        let scatter = plot(
            br#"<?xml version="1.0"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
 <c:chart><c:plotArea>
  <c:scatterChart>
   <c:scatterStyle val="lineMarker"/>
   <c:ser>
    <c:idx val="0"/><c:order val="0"/>
    <c:xVal><c:numRef><c:f>Sheet1!$A$2:$A$4</c:f><c:numCache><c:ptCount val="3"/>
      <c:pt idx="0"><c:v>1.5</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt>
      <c:pt idx="2"><c:v>4</c:v></c:pt></c:numCache></c:numRef></c:xVal>
    <c:yVal><c:numRef><c:f>Sheet1!$B$2:$B$4</c:f><c:numCache><c:ptCount val="3"/>
      <c:pt idx="0"><c:v>10</c:v></c:pt><c:pt idx="1"><c:v>20</c:v></c:pt>
      <c:pt idx="2"><c:v>40</c:v></c:pt></c:numCache></c:numRef></c:yVal>
   </c:ser>
  </c:scatterChart>
 </c:plotArea></c:chart>
</c:chartSpace>"#,
        )
        .expect("parses");
        assert_eq!(scatter.kind, ChartKind::Scatter);
        assert!(scatter.scatter_lines, "lineMarker joins the points");
        assert_eq!(scatter.series[0].categories, ["1.5", "2", "4"]);
        assert_eq!(
            scatter.series[0].values,
            [Some(10.0), Some(20.0), Some(40.0)]
        );
        assert_eq!(
            scatter.series[0].categories_ref.as_deref(),
            Some("Sheet1!$A$2:$A$4"),
            "the X reference rides in the category slot"
        );
    }

    #[test]
    fn what_a_line_marks_smooths_and_rules_is_read_and_silence_is_excels_yes() {
        let line = plot(
            br#"<?xml version="1.0"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
 <c:chart><c:plotArea>
  <c:lineChart>
   <c:grouping val="standard"/><c:varyColors val="0"/>
   <c:ser><c:idx val="0"/><c:order val="0"/>
    <c:marker><c:symbol val="none"/></c:marker>
    <c:val><c:numRef><c:f>Sheet1!$B$2:$B$3</c:f></c:numRef></c:val>
    <c:smooth val="0"/>
   </c:ser>
   <c:ser><c:idx val="1"/><c:order val="1"/>
    <c:marker><c:symbol val="triangle"/></c:marker>
    <c:val><c:numRef><c:f>Sheet1!$C$2:$C$3</c:f></c:numRef></c:val>
   </c:ser>
   <c:marker val="1"/>
   <c:axId val="1"/><c:axId val="2"/>
  </c:lineChart>
  <c:catAx><c:axId val="1"/><c:crossAx val="2"/></c:catAx>
  <c:valAx><c:axId val="2"/><c:majorGridlines/><c:crossAx val="1"/>
   <c:crossBetween val="midCat"/></c:valAx>
 </c:plotArea></c:chart>
</c:chartSpace>"#,
        )
        .expect("parses");
        assert!(line.markers);
        assert!(!line.vary_colors);
        assert_eq!(line.series[0].symbol, Symbol::None);
        assert_eq!(line.series[0].smooth, Some(false));
        assert_eq!(line.series[1].symbol, Symbol::Triangle);
        assert_eq!(line.series[1].smooth, None, "unstated, which Excel smooths");
        assert!(line.val_axis.gridlines && !line.cat_axis.gridlines);
        assert_eq!(line.between, Some(false));
        assert!(!line.categories_between());

        // And the part that states none of it: Excel's defaults, not the schema's.
        let bare = chart();
        assert!(bare.markers && bare.vary_colors);
        assert!(!bare.val_axis.gridlines);
        assert_eq!(bare.hole, 0.0);
        assert!(bare.categories_between());
    }

    #[test]
    fn a_gap_in_the_cache_stays_a_gap() {
        // The cache skips idx 2. Pushing points in document order would put 40
        // against Q3 and leave Q4 empty — a chart that is wrong rather than
        // incomplete.
        let chart = chart();
        assert_eq!(
            chart.series[0].values,
            [Some(10.0), Some(20.0), None, Some(40.0)]
        );
    }

    #[test]
    fn a_series_colour_is_its_own_and_not_the_axiss() {
        // `<a:srgbClr>` appears under the value axis too, later in the file.
        let chart = chart();
        assert_eq!(
            chart.series[0].color,
            Some([0x44, 0x72, 0xC4]),
            "the axis's red must not have overwritten it"
        );
    }

    #[test]
    fn a_part_that_names_no_plot_type_is_not_a_chart() {
        // Both a `chartSpace` of a kind nobody draws and a part that is not a
        // chart at all: the caller draws a frame rather than empty axes.
        assert!(plot(b"<html><body>not a chart</body></html>").is_none());
        assert!(
            plot(br#"<c:chartSpace><c:chart><c:plotArea/></c:chart></c:chartSpace>"#).is_none()
        );
    }

    #[test]
    fn the_chart_areas_own_paint_is_read_and_a_seriess_is_not_mistaken_for_it() {
        // LibreOffice writes the chartSpace's spPr *after* </c:chart>: noFill
        // for both the area and its border. The same elements inside a series
        // or a legend mean something else and must not leak out.
        let part = br#"<c:chartSpace xmlns:c="x" xmlns:a="y"><c:chart><c:plotArea><c:barChart>
            <c:barDir val="col"/><c:gapWidth val="100"/>
            <c:ser><c:idx val="0"/>
              <c:spPr><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></c:spPr>
              <c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:numRef></c:val>
            </c:ser></c:barChart></c:plotArea>
            <c:legend><c:legendPos val="r"/><c:spPr><a:noFill/><a:ln><a:noFill/></a:ln></c:spPr></c:legend>
            </c:chart>
            <c:spPr><a:noFill/><a:ln><a:noFill/></a:ln></c:spPr>
            </c:chartSpace>"#;
        let plot = plot(part).expect("parses");
        assert_eq!(plot.area_fill, Paint::None);
        assert_eq!(plot.area_line, Paint::None);
        assert_eq!(plot.gap, 100.0);
        assert_eq!(plot.series[0].color, Some([0xFF, 0, 0]));

        // And a part that says nothing leaves the painter its defaults.
        let silent = chart();
        assert_eq!(silent.area_fill, Paint::Auto);
        assert_eq!(silent.area_line, Paint::Auto);
        assert_eq!(silent.gap, 150.0, "Office's own default");
    }

    #[test]
    fn the_plot_areas_own_paint_is_read_and_an_axiss_is_not_mistaken_for_it() {
        // The sample writes the plotArea's spPr *after* the axes, each of
        // which carries its own — and the value axis carries gridlines with a
        // third. Only the direct child names the box Word draws around the
        // bars.
        let part = br#"<c:chartSpace xmlns:c="x" xmlns:a="y"><c:chart><c:plotArea><c:barChart>
            <c:barDir val="col"/>
            <c:ser><c:idx val="0"/>
              <c:spPr><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></c:spPr>
              <c:dLbls><c:spPr><a:ln><a:solidFill><a:srgbClr val="00FF00"/></a:solidFill></a:ln></c:spPr></c:dLbls>
              <c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:numRef></c:val>
            </c:ser></c:barChart>
            <c:catAx><c:spPr><a:ln><a:solidFill><a:srgbClr val="111111"/></a:solidFill></a:ln></c:spPr></c:catAx>
            <c:valAx><c:majorGridlines><c:spPr><a:ln><a:solidFill><a:srgbClr val="222222"/></a:solidFill></a:ln></c:spPr></c:majorGridlines>
              <c:spPr><a:ln><a:solidFill><a:srgbClr val="333333"/></a:solidFill></a:ln></c:spPr></c:valAx>
            <c:spPr><a:noFill/><a:ln><a:solidFill><a:srgbClr val="B3B3B3"/></a:solidFill></a:ln></c:spPr>
            </c:plotArea></c:chart></c:chartSpace>"#;
        let plot = plot(part).expect("parses");
        assert_eq!(plot.plot_fill, Paint::None);
        assert_eq!(plot.plot_line, Paint::Rgb([0xB3, 0xB3, 0xB3]));
        assert_eq!(plot.series[0].color, Some([0xFF, 0, 0]));
        // The chart area stays untouched by any of it.
        assert_eq!(plot.area_fill, Paint::Auto);
        assert_eq!(plot.area_line, Paint::Auto);

        // A part whose plotArea says nothing leaves the painter its default,
        // which draws no box.
        let silent = chart();
        assert_eq!(silent.plot_fill, Paint::Auto);
        assert_eq!(silent.plot_line, Paint::Auto);
    }

    #[test]
    fn each_axis_keeps_its_own_ticks_and_its_own_line() {
        // The sample states `majorTickMark="out"` and a b3b3b3 line on both
        // axes. The value axis's gridlines carry a line of their own, and the
        // axis text carries a colour — neither is the axis.
        let part = br#"<c:chartSpace xmlns:c="x" xmlns:a="y"><c:chart><c:plotArea><c:barChart>
            <c:barDir val="col"/>
            <c:ser><c:idx val="0"/>
              <c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:numRef></c:val>
            </c:ser></c:barChart>
            <c:catAx><c:delete val="0"/><c:majorTickMark val="out"/><c:minorTickMark val="none"/>
              <c:spPr><a:ln><a:solidFill><a:srgbClr val="B3B3B3"/></a:solidFill></a:ln></c:spPr></c:catAx>
            <c:valAx><c:delete val="0"/>
              <c:majorGridlines><c:spPr><a:ln><a:solidFill><a:srgbClr val="222222"/></a:solidFill></a:ln></c:spPr></c:majorGridlines>
              <c:majorTickMark val="cross"/>
              <c:txPr><a:p><a:pPr><a:defRPr><a:solidFill><a:srgbClr val="444444"/></a:solidFill></a:defRPr></a:pPr></a:p></c:txPr>
              <c:spPr><a:ln><a:solidFill><a:srgbClr val="999999"/></a:solidFill></a:ln></c:spPr></c:valAx>
            </c:plotArea></c:chart></c:chartSpace>"#;
        let both = plot(part).expect("parses");
        assert_eq!(both.cat_axis.major_tick, TickMark::Out);
        assert_eq!(both.cat_axis.line, Paint::Rgb([0xB3, 0xB3, 0xB3]));
        assert_eq!(both.val_axis.major_tick, TickMark::Cross);
        assert_eq!(
            both.val_axis.line,
            Paint::Rgb([0x99, 0x99, 0x99]),
            "not the gridlines' colour and not the text's"
        );
        assert!(!both.cat_axis.deleted && !both.val_axis.deleted);
        // Neither axis is the plot area, whose own spPr this part omits.
        assert_eq!(both.plot_line, Paint::Auto);

        // And an axis the author removed says so.
        let gone = br#"<c:chartSpace xmlns:c="x" xmlns:a="y"><c:chart><c:plotArea><c:barChart>
            <c:barDir val="col"/><c:ser><c:idx val="0"/>
              <c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:numRef></c:val>
            </c:ser></c:barChart>
            <c:catAx><c:delete val="1"/><c:majorTickMark val="out"/></c:catAx>
            </c:plotArea></c:chart></c:chartSpace>"#;
        let removed = plot(gone).expect("parses");
        assert!(removed.cat_axis.deleted);

        // A part that names no ticks draws none, rather than the schema's
        // nominal default.
        let silent = chart();
        assert_eq!(silent.cat_axis.major_tick, TickMark::None);
        assert_eq!(silent.val_axis.major_tick, TickMark::None);
    }

    #[test]
    fn a_document_chart_reads_the_same_as_a_workbook_one() {
        // Word writes the reference as a bare label rather than a cell range,
        // and the cache is all a document ever has to draw from.
        let word = br#"<c:chartSpace xmlns:c="x" xmlns:a="y"><c:chart><c:plotArea><c:barChart>
            <c:barDir val="col"/><c:grouping val="clustered"/>
            <c:ser><c:idx val="0"/>
              <c:tx><c:strRef><c:f>label 0</c:f><c:strCache><c:pt idx="0"><c:v>Column 1</c:v></c:pt></c:strCache></c:strRef></c:tx>
              <c:cat><c:strRef><c:f>categories</c:f><c:strCache>
                <c:pt idx="0"><c:v>Row 1</c:v></c:pt><c:pt idx="1"><c:v>Row 2</c:v></c:pt></c:strCache></c:strRef></c:cat>
              <c:val><c:numRef><c:f>0</c:f><c:numCache>
                <c:pt idx="0"><c:v>9.1</c:v></c:pt><c:pt idx="1"><c:v>2.4</c:v></c:pt></c:numCache></c:numRef></c:val>
            </c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
        let plot = plot(word).expect("parses");
        assert_eq!(plot.series[0].values, [Some(9.1), Some(2.4)]);
        assert_eq!(plot.categories(), ["Row 1", "Row 2"]);
    }
}
