//! Putting a chart or a picture into a package that has never held one.
//!
//! Everywhere else this crate edits parts it was given. Here it authors them,
//! which is the one thing `DESIGN.md` §1 allows only under its own terms: what
//! is written is written whole, by us, and is small enough to be read and
//! checked by a person. A chart part authored here holds a plot, its series,
//! and nothing else — no gradients, no 3-D, none of the several dozen elements
//! Excel writes and this crate carries through untouched on files it did not
//! author.
//!
//! Four things have to line up for one object to exist:
//!
//! 1. The **object part** — a chart's `chartN.xml`, or a picture's image bytes
//!    in `media/`.
//! 2. The **drawing part**, which holds the anchor saying where on the sheet
//!    it goes, and a relationship from that drawing to the object part.
//! 3. A relationship from the **worksheet** to the drawing, and a `<drawing>`
//!    element in the worksheet pointing at it by id.
//! 4. A **content type** for every part introduced.
//!
//! Anything half-done here is a workbook Excel refuses to open, so the work is
//! ordered to leave the package consistent at every step: nothing is written
//! until every name is settled.

use ooxml::{Package, PartName, Relationship, TargetMode};
use ss_model::chart::{Anchor, AnchorPoint};
use ss_model::{Chart, Picture, Workbook};

use crate::error::Result;
use crate::write::REL_BASE;

const DRAWING_TYPE: &str = "application/vnd.openxmlformats-officedocument.drawing+xml";
const COMMENTS_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml";
const VML_TYPE: &str = "application/vnd.openxmlformats-officedocument.vmlDrawing";
const CHART_TYPE: &str = "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";
const RELS_TYPE: &str = "application/vnd.openxmlformats-package.relationships+xml";

const XDR_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const A_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const C_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";

const DECL: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;

/// A sheet's objects that are in the model and not yet in the package.
///
/// `part` empty is what marks one: every object read from a file names the part
/// it came from, so an empty name can only be something the application made.
pub(crate) fn pending(sheet: &ss_model::Sheet) -> bool {
    sheet.charts.iter().any(|c| c.part.is_empty())
        || sheet.pictures.iter().any(|p| p.part.is_empty())
}

/// Writes every new chart and picture on `sheet` into the package.
///
/// Returns the relationship id of the sheet's drawing when the worksheet has to
/// be told about it — that is, when the drawing was authored here.
pub(crate) fn materialize(
    package: &mut Package,
    workbook: &mut Workbook,
    sheet_index: usize,
    sheet_part: &PartName,
) -> Result<Option<String>> {
    let Some(sheet) = workbook.sheet(sheet_index) else {
        return Ok(None);
    };
    if !pending(sheet) {
        return Ok(None);
    }

    // The drawing this sheet's objects go in: its own if it has one, otherwise
    // a new one, which is also the only case where the worksheet itself has to
    // change.
    let (drawing, new_rel) = match crate::drawing_of(package, sheet_part) {
        Some(name) => (name, None),
        None => {
            let name = free_name(package, "xl/drawings", "drawing", "xml")?;
            package.put_part(name.clone(), DRAWING_TYPE, empty_drawing());
            let mut rels = package.relationships(sheet_part)?;
            let id = rels.next_id();
            rels.insert(Relationship {
                id: id.clone(),
                rel_type: format!("{REL_BASE}/drawing"),
                target: crate::write::relative_to(sheet_part, &name),
                mode: TargetMode::Internal,
            });
            package.put_part(sheet_part.rels_part(), RELS_TYPE, rels.to_xml());
            (name, Some(id))
        }
    };

    // Every anchor already in the drawing, so that the ones appended here are
    // numbered from the right place — `anchor_index` is a count over the file's
    // own order and the model has to agree with it.
    let mut next_anchor = crate::object_anchors(package, &drawing)?
        .keys()
        .max()
        .map_or(0, |n| n + 1);
    let mut rels = package.relationships(&drawing)?;
    let mut anchors: Vec<u8> = Vec::new();
    // `id` on `<xdr:cNvPr>` has to be unique within the drawing; Excel counts
    // from two and nothing depends on the number beyond that.
    let mut next_id = 2 + next_anchor as u32;

    let charts: Vec<usize> = workbook.sheets[sheet_index]
        .charts
        .iter()
        .enumerate()
        .filter(|(_, c)| c.part.is_empty())
        .map(|(i, _)| i)
        .collect();
    for index in charts {
        let name = free_name(package, "xl/charts", "chart", "xml")?;
        let chart = workbook.sheets[sheet_index].charts[index].clone();
        package.put_part(name.clone(), CHART_TYPE, chart_part(&chart));

        let rel_id = rels.next_id();
        rels.insert(Relationship {
            id: rel_id.clone(),
            rel_type: format!("{REL_BASE}/chart"),
            target: crate::write::relative_to(&drawing, &name),
            mode: TargetMode::Internal,
        });
        anchors.extend_from_slice(&graphic_frame(&chart.anchor, &rel_id, next_id, index));

        let stored = &mut workbook.sheets[sheet_index].charts[index];
        stored.part = name.as_str().to_string();
        stored.drawing_part = drawing.as_str().to_string();
        stored.anchor_index = next_anchor;
        next_anchor += 1;
        next_id += 1;
    }

    let pictures: Vec<usize> = workbook.sheets[sheet_index]
        .pictures
        .iter()
        .enumerate()
        .filter(|(_, p)| p.part.is_empty())
        .map(|(i, _)| i)
        .collect();
    for index in pictures {
        let picture = workbook.sheets[sheet_index].pictures[index].clone();
        let extension = extension_for(&picture.content_type);
        let name = free_name(package, "xl/media", "image", extension)?;
        package.put_part(name.clone(), &picture.content_type, picture.data.to_vec());

        let rel_id = rels.next_id();
        rels.insert(Relationship {
            id: rel_id.clone(),
            rel_type: format!("{REL_BASE}/image"),
            target: crate::write::relative_to(&drawing, &name),
            mode: TargetMode::Internal,
        });
        anchors.extend_from_slice(&pic_anchor(&picture, &rel_id, next_id));

        let stored = &mut workbook.sheets[sheet_index].pictures[index];
        stored.part = name.as_str().to_string();
        stored.drawing_part = drawing.as_str().to_string();
        stored.anchor_index = next_anchor;
        next_anchor += 1;
        next_id += 1;
    }

    if !anchors.is_empty() {
        let Some(part) = package.part(&drawing) else {
            return Ok(new_rel);
        };
        let content_type = part.content_type.clone();
        let data = append_anchors(part.data(), &anchors);
        package.put_part(drawing.clone(), &content_type, data);
        package.put_part(drawing.rels_part(), RELS_TYPE, rels.to_xml());
    }
    Ok(new_rel)
}

/// Brings a sheet's notes in line with the model.
///
/// Returns the relationship id of a VML drawing authored here, which the
/// worksheet then has to name with `<legacyDrawing>` — the same shape as a
/// drawing, and for the same reason: a note Excel cannot draw a box for is a
/// file Excel offers to repair.
pub(crate) fn comments(
    package: &mut Package,
    workbook: &ss_model::Workbook,
    sheet_index: usize,
    sheet_part: &PartName,
) -> Result<Option<String>> {
    let Some(sheet) = workbook.sheet(sheet_index) else {
        return Ok(None);
    };
    let existing = crate::comments_part(package, sheet_part);
    let was = match existing.as_ref().and_then(|name| package.part(name)) {
        Some(part) => crate::comments::parse(sheet_part.as_str(), part.data())?,
        None => Vec::new(),
    };
    if was == sheet.comments {
        return Ok(None);
    }

    // A sheet whose notes have all been deleted keeps its part, emptied. The
    // alternative — removing the part, its relationship, the VML, and the
    // `<legacyDrawing>` naming it — is four edits to undo one, and an empty
    // comment list is what Excel itself leaves behind.
    let name = match existing {
        Some(name) => name,
        None => free_name(package, "xl", "comments", "xml")?,
    };
    package.put_part(
        name.clone(),
        COMMENTS_TYPE,
        crate::comments::write(&sheet.comments),
    );

    let mut rels = package.relationships(sheet_part)?;
    let mut authored: Option<String> = None;
    if !rels.iter().any(|r| r.rel_type.ends_with("/comments")) {
        let id = rels.next_id();
        rels.insert(Relationship {
            id,
            rel_type: format!("{REL_BASE}/comments"),
            target: crate::write::relative_to(sheet_part, &name),
            mode: TargetMode::Internal,
        });
    }
    // The VML: authored when the sheet has none, rewritten when it has one,
    // because a shape per note is what says which cells have boxes.
    let vml_existing = rels
        .iter()
        .find(|r| r.rel_type.ends_with("/vmlDrawing"))
        .and_then(|r| r.resolve(sheet_part))
        .and_then(|r| r.ok());
    let vml_name = match vml_existing {
        Some(name) => name,
        None => {
            let name = free_name(package, "xl/drawings", "vmlDrawing", "vml")?;
            let id = rels.next_id();
            rels.insert(Relationship {
                id: id.clone(),
                rel_type: format!("{REL_BASE}/vmlDrawing"),
                target: crate::write::relative_to(sheet_part, &name),
                mode: TargetMode::Internal,
            });
            authored = Some(id);
            name
        }
    };
    package.put_part(vml_name, VML_TYPE, crate::comments::vml(&sheet.comments));
    package.put_part(sheet_part.rels_part(), RELS_TYPE, rels.to_xml());
    Ok(authored)
}

/// Puts new anchors in just before `</xdr:wsDr>`.
///
/// Text rather than a parse: the closing tag of the root is the last thing in
/// the part, everything before it is the producer's own bytes, and appending
/// there is the one edit that cannot disturb them.
fn append_anchors(data: &[u8], anchors: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(data);
    if let Some(close) = text.rfind("</xdr:wsDr>").or_else(|| text.rfind("</wsDr>")) {
        let mut out = Vec::with_capacity(data.len() + anchors.len());
        out.extend_from_slice(&data[..close]);
        out.extend_from_slice(anchors);
        out.extend_from_slice(&data[close..]);
        return out;
    }
    // A drawing with no objects in it is written `<xdr:wsDr .../>`, which has
    // to grow a body before anything can go in one.
    let Some(end) = text.rfind("/>") else {
        return data.to_vec();
    };
    let name = text[..end]
        .rfind('<')
        .map(|open| {
            text[open + 1..end]
                .split_whitespace()
                .next()
                .unwrap_or("xdr:wsDr")
        })
        .unwrap_or("xdr:wsDr");
    let mut out = Vec::with_capacity(data.len() + anchors.len() + 16);
    out.extend_from_slice(&data[..end]);
    out.push(b'>');
    out.extend_from_slice(anchors);
    out.extend_from_slice(format!("</{name}>").as_bytes());
    out
}

fn empty_drawing() -> Vec<u8> {
    format!(r#"{DECL}<xdr:wsDr xmlns:xdr="{XDR_NS}" xmlns:a="{A_NS}" xmlns:r="{R_NS}"/>"#)
        .into_bytes()
}

/// The `<xdr:from>`/`<xdr:to>` pair, or the `<xdr:ext>` a one-cell anchor uses.
fn anchor_body(anchor: &Anchor) -> String {
    let corner = |tag: &str, p: &AnchorPoint| {
        format!(
            "<xdr:{tag}><xdr:col>{}</xdr:col><xdr:colOff>{}</xdr:colOff>\
             <xdr:row>{}</xdr:row><xdr:rowOff>{}</xdr:rowOff></xdr:{tag}>",
            p.col, p.col_offset, p.row, p.row_offset
        )
    };
    match anchor {
        Anchor::TwoCell { from, to } => format!("{}{}", corner("from", from), corner("to", to)),
        Anchor::OneCell {
            from,
            width,
            height,
        } => format!(
            r#"{}<xdr:ext cx="{width}" cy="{height}"/>"#,
            corner("from", from)
        ),
        // Authored as a one-cell anchor at the sheet's corner: an absolute
        // anchor is a position in the page rather than in the grid, and nothing
        // here has a reason to make one.
        Anchor::Absolute { width, height, .. } => format!(
            r#"{}<xdr:ext cx="{width}" cy="{height}"/>"#,
            corner("from", &AnchorPoint::default())
        ),
    }
}

fn anchor_tag(anchor: &Anchor) -> &'static str {
    match anchor {
        Anchor::TwoCell { .. } => "twoCellAnchor",
        _ => "oneCellAnchor",
    }
}

/// The anchor holding a chart: a graphic frame naming the chart part by
/// relationship id.
///
/// Pushed element by element rather than written as one long string, because
/// the schema fixes this order and a sequence down the page can be checked
/// against the schema line by line.
fn graphic_frame(anchor: &Anchor, rel_id: &str, id: u32, number: usize) -> Vec<u8> {
    let tag = anchor_tag(anchor);
    let mut out = String::new();
    out.push_str(&format!("<xdr:{tag}>"));
    out.push_str(&anchor_body(anchor));
    out.push_str(r#"<xdr:graphicFrame macro="">"#);
    out.push_str("<xdr:nvGraphicFramePr>");
    out.push_str(&format!(
        r#"<xdr:cNvPr id="{id}" name="Chart {}"/>"#,
        number + 1
    ));
    out.push_str("<xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr>");
    // The frame's own transform, zeroed: the anchor above already says where
    // the chart goes, and Excel writes the same when it has just made one.
    out.push_str(r#"<xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm>"#);
    out.push_str(&format!(r#"<a:graphic><a:graphicData uri="{C_NS}">"#));
    out.push_str(&format!(
        r#"<c:chart xmlns:c="{C_NS}" xmlns:r="{R_NS}" r:id="{rel_id}"/>"#
    ));
    out.push_str("</a:graphicData></a:graphic></xdr:graphicFrame>");
    out.push_str(&format!("<xdr:clientData/></xdr:{tag}>"));
    out.into_bytes()
}

/// The anchor holding a picture.
fn pic_anchor(picture: &Picture, rel_id: &str, id: u32) -> Vec<u8> {
    let tag = anchor_tag(&picture.anchor);
    let mut out = String::new();
    out.push_str(&format!("<xdr:{tag}>"));
    out.push_str(&anchor_body(&picture.anchor));
    out.push_str("<xdr:pic><xdr:nvPicPr>");
    out.push_str(&format!(
        r#"<xdr:cNvPr id="{id}" name="{}"/>"#,
        xml_escape(&picture.name)
    ));
    out.push_str("<xdr:cNvPicPr/></xdr:nvPicPr><xdr:blipFill>");
    out.push_str(&format!(r#"<a:blip xmlns:r="{R_NS}" r:embed="{rel_id}"/>"#));
    out.push_str("<a:stretch><a:fillRect/></a:stretch></xdr:blipFill>");
    out.push_str(r#"<xdr:spPr><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></xdr:spPr>"#);
    out.push_str(&format!("</xdr:pic><xdr:clientData/></xdr:{tag}>"));
    out.into_bytes()
}

/// A whole `chartSpace` part for one chart.
///
/// Cached values are written beside every reference, as Excel does: a reader
/// that cannot resolve the reference — because the sheet name has a space in
/// it, or the workbook is being read by something simpler than Excel — still
/// has the numbers to draw.
fn chart_part(chart: &Chart) -> Vec<u8> {
    let mut series = String::new();
    for (index, s) in chart.plot.series.iter().enumerate() {
        series.push_str(&format!(
            r#"<c:ser><c:idx val="{index}"/><c:order val="{index}"/>"#
        ));
        if let Some(name_ref) = &s.name_ref {
            series.push_str(&format!(
                r#"<c:tx><c:strRef><c:f>{}</c:f>"#,
                xml_escape(name_ref)
            ));
            series.push_str(&format!(
                r#"<c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>{}</c:v></c:pt></c:strCache>"#,
                xml_escape(s.name.as_deref().unwrap_or(""))
            ));
            series.push_str("</c:strRef></c:tx>");
        }
        if let Some(cat_ref) = &s.categories_ref {
            series.push_str(&format!(
                r#"<c:cat><c:strRef><c:f>{}</c:f><c:strCache><c:ptCount val="{}"/>"#,
                xml_escape(cat_ref),
                s.categories.len(),
            ));
            for (i, c) in s.categories.iter().enumerate() {
                series.push_str(&format!(
                    r#"<c:pt idx="{i}"><c:v>{}</c:v></c:pt>"#,
                    xml_escape(c)
                ));
            }
            series.push_str("</c:strCache></c:strRef></c:cat>");
        }
        if let Some(values_ref) = &s.values_ref {
            series.push_str(&format!(
                r#"<c:val><c:numRef><c:f>{}</c:f><c:numCache>"#,
                xml_escape(values_ref)
            ));
            series.push_str(&format!(
                r#"<c:formatCode>General</c:formatCode><c:ptCount val="{}"/>"#,
                s.values.len()
            ));
            // A blank in the middle of a series is written as no point at all
            // rather than as a zero: that is the difference between a gap in a
            // line and a line that dives to the axis.
            for (i, v) in s.values.iter().enumerate() {
                if let Some(v) = v {
                    series.push_str(&format!(
                        r#"<c:pt idx="{i}"><c:v>{}</c:v></c:pt>"#,
                        ss_model::format_general(*v)
                    ));
                }
            }
            series.push_str("</c:numCache></c:numRef></c:val>");
        }
        series.push_str("</c:ser>");
    }

    let (element, shape) = match chart.plot.kind {
        ss_model::ChartKind::Line => ("lineChart", String::new()),
        ss_model::ChartKind::Pie => ("pieChart", String::new()),
        ss_model::ChartKind::Doughnut => ("doughnutChart", String::new()),
        ss_model::ChartKind::Area => ("areaChart", String::new()),
        _ => (
            "barChart",
            format!(
                r#"<c:barDir val="{}"/>"#,
                if chart.plot.horizontal { "bar" } else { "col" }
            ),
        ),
    };
    let grouping = match (element, chart.plot.grouping) {
        ("pieChart" | "doughnutChart", _) => String::new(),
        (_, ss_model::chart::Grouping::Stacked) => r#"<c:grouping val="stacked"/>"#.to_string(),
        (_, ss_model::chart::Grouping::PercentStacked) => {
            r#"<c:grouping val="percentStacked"/>"#.to_string()
        }
        ("barChart", _) => r#"<c:grouping val="clustered"/>"#.to_string(),
        _ => r#"<c:grouping val="standard"/>"#.to_string(),
    };

    // The axes a plot with axes must declare, and their ids, which the plot
    // itself repeats. Excel picks arbitrary numbers; these are ours.
    let axes = if chart.plot.kind.has_axes() {
        concat!(
            r#"<c:catAx><c:axId val="111111111"/><c:scaling><c:orientation val="minMax"/></c:scaling>"#,
            r#"<c:delete val="0"/><c:axPos val="b"/><c:crossAx val="222222222"/></c:catAx>"#,
            r#"<c:valAx><c:axId val="222222222"/><c:scaling><c:orientation val="minMax"/></c:scaling>"#,
            r#"<c:delete val="0"/><c:axPos val="l"/><c:crossAx val="111111111"/></c:valAx>"#,
        )
    } else {
        ""
    };
    let axis_ids = if chart.plot.kind.has_axes() {
        r#"<c:axId val="111111111"/><c:axId val="222222222"/>"#
    } else {
        ""
    };

    let title = match &chart.plot.title {
        Some(text) => {
            let mut out = String::from("<c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>");
            out.push_str(&xml_escape(text));
            out.push_str("</a:t></a:r></a:p></c:rich></c:tx>");
            out.push_str(r#"<c:overlay val="0"/></c:title><c:autoTitleDeleted val="0"/>"#);
            out
        }
        None => r#"<c:autoTitleDeleted val="1"/>"#.to_string(),
    };
    let legend = match chart.plot.legend {
        Some(position) => format!(
            r#"<c:legend><c:legendPos val="{}"/><c:overlay val="0"/></c:legend>"#,
            match position {
                ss_model::chart::LegendPosition::Top => "t",
                ss_model::chart::LegendPosition::Bottom => "b",
                ss_model::chart::LegendPosition::Left => "l",
                ss_model::chart::LegendPosition::TopRight => "tr",
                ss_model::chart::LegendPosition::Right => "r",
            }
        ),
        None => String::new(),
    };

    let mut out = String::from(DECL);
    out.push_str(&format!(
        r#"<c:chartSpace xmlns:c="{C_NS}" xmlns:a="{A_NS}" xmlns:r="{R_NS}"><c:chart>"#
    ));
    out.push_str(&title);
    out.push_str("<c:plotArea><c:layout/>");
    out.push_str(&format!(
        "<c:{element}>{shape}{grouping}{series}{axis_ids}</c:{element}>"
    ));
    out.push_str(axes);
    out.push_str("</c:plotArea>");
    out.push_str(&legend);
    out.push_str(r#"<c:plotVisOnly val="1"/><c:dispBlanksAs val="gap"/>"#);
    out.push_str("</c:chart></c:chartSpace>");
    out.into_bytes()
}

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// The file extension an image's content type asks for.
fn extension_for(content_type: &str) -> &'static str {
    match content_type {
        "image/jpeg" => "jpeg",
        "image/gif" => "gif",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        _ => "png",
    }
}

/// `<directory>/<stem>N.<extension>` for the lowest `N` nobody has taken.
fn free_name(package: &Package, directory: &str, stem: &str, extension: &str) -> Result<PartName> {
    for n in 1..=10_000u32 {
        let candidate = PartName::new(&format!("/{directory}/{stem}{n}.{extension}"))?;
        if package.part(&candidate).is_none() {
            return Ok(candidate);
        }
    }
    Err(crate::Error::Xml {
        part: format!("/{directory}/{stem}N.{extension}"),
        source: "no free part name left".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ss_model::chart::AnchorPoint;

    fn at(col: u32, row: u32) -> AnchorPoint {
        AnchorPoint {
            col,
            row,
            ..Default::default()
        }
    }

    fn a_chart() -> Chart {
        Chart {
            part: String::new(),
            drawing_part: String::new(),
            anchor_index: 0,
            anchor: Anchor::TwoCell {
                from: at(3, 1),
                to: at(9, 16),
            },
            plot: ss_model::chart::Plot {
                kind: ss_model::ChartKind::Bar,
                grouping: ss_model::chart::Grouping::Clustered,
                horizontal: false,
                title: Some("Sales & Costs".to_string()),
                title_ref: None,
                legend: Some(ss_model::chart::LegendPosition::Right),
                series: vec![ss_model::chart::Series {
                    name: Some("Sales".to_string()),
                    name_ref: Some("Sheet1!$B$1".to_string()),
                    values_ref: Some("Sheet1!$B$2:$B$4".to_string()),
                    values: vec![Some(1.0), Some(2.5), None],
                    categories_ref: Some("Sheet1!$A$2:$A$4".to_string()),
                    categories: vec!["Jan".into(), "Feb".into(), "Mar".into()],
                    color: None,
                }],
                ..ss_model::chart::Plot::default()
            },
        }
    }

    #[test]
    fn an_authored_chart_part_reads_back_as_the_chart_it_was() {
        let bytes = chart_part(&a_chart());
        let read = ss_model::chart::read::plot(&bytes).expect("parses");
        assert_eq!(read.kind, ss_model::ChartKind::Bar);
        assert_eq!(read.title.as_deref(), Some("Sales & Costs"));
        assert_eq!(read.series.len(), 1);
        assert_eq!(read.series[0].name.as_deref(), Some("Sales"));
        // The blank third point is written as a count of three with two points
        // in it, which is how a gap is spelled; the reader keeps the points it
        // finds rather than padding to the count.
        assert_eq!(read.series[0].values, [Some(1.0), Some(2.5)]);
        let text = String::from_utf8(chart_part(&a_chart())).expect("utf-8");
        assert!(text.contains(r#"<c:ptCount val="3"/>"#), "{text}");
        assert_eq!(read.series[0].categories, ["Jan", "Feb", "Mar"]);
        assert_eq!(
            read.series[0].values_ref.as_deref(),
            Some("Sheet1!$B$2:$B$4")
        );
    }

    #[test]
    fn an_ampersand_in_a_title_is_escaped_rather_than_written() {
        let bytes = chart_part(&a_chart());
        let text = String::from_utf8(bytes).expect("utf-8");
        assert!(text.contains("Sales &amp; Costs"), "{text}");
    }

    #[test]
    fn a_pie_chart_declares_no_axes_and_a_bar_chart_does() {
        let mut chart = a_chart();
        chart.plot.kind = ss_model::ChartKind::Pie;
        let text = String::from_utf8(chart_part(&chart)).expect("utf-8");
        assert!(!text.contains("catAx"), "{text}");
        assert!(text.contains("<c:pieChart>"), "{text}");

        let text = String::from_utf8(chart_part(&a_chart())).expect("utf-8");
        assert!(text.contains("<c:catAx>"), "{text}");
        assert!(text.contains(r#"<c:barDir val="col"/>"#), "{text}");
    }

    #[test]
    fn an_anchor_is_written_in_the_order_the_schema_fixes() {
        let frame = graphic_frame(
            &Anchor::TwoCell {
                from: at(1, 2),
                to: at(5, 9),
            },
            "rId3",
            2,
            0,
        );
        let text = String::from_utf8(frame).expect("utf-8");
        let from = text.find("<xdr:from>").expect("from");
        let to = text.find("<xdr:to>").expect("to");
        let frame_at = text.find("<xdr:graphicFrame").expect("frame");
        assert!(from < to && to < frame_at, "{text}");
        assert!(text.contains(r#"r:id="rId3""#), "{text}");
    }

    #[test]
    fn new_anchors_go_in_before_the_closing_tag() {
        let out = append_anchors(&empty_drawing(), b"<xdr:twoCellAnchor/>");
        let text = String::from_utf8(out).expect("utf-8");
        assert!(
            text.ends_with("<xdr:twoCellAnchor/></xdr:wsDr>"),
            "an empty root has to grow a body: {text}"
        );
    }
}
