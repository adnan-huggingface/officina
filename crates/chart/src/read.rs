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

use crate::{ChartKind, Grouping, LegendPosition, Plot, Series};

/// Which reference of a series is being read. `<c:f>` and `<c:pt>` are the same
/// elements under `<c:tx>`, `<c:cat>`, and `<c:val>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    None,
    Name,
    Categories,
    Values,
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

    while let Ok(ev) = reader.read_event_into(&mut buf) {
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
                    out.horizontal = attr_text(e, b"val").as_deref() == Some("bar");
                }
                b"spPr" if in_series => depth_in_series_props += 1,
                b"srgbClr" if depth_in_series_props > 0 && series.color.is_none() => {
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
