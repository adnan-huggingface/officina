//! Where a chart sits: `xl/drawings/drawingN.xml`.
//!
//! A worksheet does not point at a chart. It points at a *drawing*, and the
//! drawing anchors a graphic frame whose relationship names the chart. Three
//! parts and two relationship hops for one picture on a sheet, which is why
//! this is a separate file rather than four lines in the worksheet reader.
//!
//! The anchor is a cell plus an offset into it, measured in EMUs. That offset
//! is not pedantry: without it, a chart shifts by up to a whole column when the
//! grid it sits on is redrawn at a different width.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use ss_model::chart::{Anchor, AnchorPoint};

use crate::error::{xml_err, Result};
use crate::xml::{attr_raw, end_local_name, local_name, push_text, strip_prefix};

/// One anchored graphic, and the relationship id of what it holds.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Anchored {
    /// Position among *all* anchors in the part, counted from zero. Charts and
    /// shapes take their place in the count: it is an index into the file, not
    /// into the pictures we happened to understand.
    pub index: usize,
    pub anchor: Anchor,
    /// `r:id` on `<c:chart>`, `<a:blip>`, or whatever the frame carries.
    pub rel_id: String,
    /// `<xdr:cNvPr name>` — "Picture 3", "Chart 1", or whatever it was renamed
    /// to. The only human-readable name a drawing has.
    pub name: String,
}

/// Which corner of an anchor the reader is inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Corner {
    None,
    From,
    To,
}

pub(crate) fn parse(part: &str, data: &[u8]) -> Result<Vec<Anchored>> {
    let mut reader = Reader::from_reader(data);
    reader.config_mut().check_end_names = false;

    let mut out = Vec::new();
    let mut buf = Vec::new();

    let mut kind: Option<&'static str> = None;
    let mut from = AnchorPoint::default();
    let mut to = AnchorPoint::default();
    let mut extent = (0i64, 0i64);
    let mut origin = (0i64, 0i64);
    let mut rel_id: Option<String> = None;
    let mut name = String::new();
    let mut corner = Corner::None;
    let mut seen = 0usize;
    let mut index = 0usize;
    let mut field: Option<&'static str> = None;
    let mut text = String::new();

    loop {
        let ev = reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err(part, e))?;
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => match local_name(e) {
                b"twoCellAnchor" | b"oneCellAnchor" | b"absoluteAnchor" => {
                    kind = Some(match local_name(e) {
                        b"twoCellAnchor" => "two",
                        b"oneCellAnchor" => "one",
                        _ => "absolute",
                    });
                    from = AnchorPoint::default();
                    to = AnchorPoint::default();
                    extent = (0, 0);
                    origin = (0, 0);
                    rel_id = None;
                    name.clear();
                    index = seen;
                    seen += 1;
                }
                b"cNvPr" => {
                    if name.is_empty() {
                        if let Some(raw) = attr_raw(e, b"name") {
                            name = String::from_utf8_lossy(&raw).into_owned();
                        }
                    }
                }
                b"from" => corner = Corner::From,
                b"to" => corner = Corner::To,
                b"col" => field = Some("col"),
                b"colOff" => field = Some("colOff"),
                b"row" => field = Some("row"),
                b"rowOff" => field = Some("rowOff"),
                b"ext" => {
                    extent = (
                        attr_i64(e, b"cx").unwrap_or(0),
                        attr_i64(e, b"cy").unwrap_or(0),
                    );
                }
                b"pos" => {
                    origin = (
                        attr_i64(e, b"x").unwrap_or(0),
                        attr_i64(e, b"y").unwrap_or(0),
                    );
                }
                // The relationship id lives on whatever the frame holds — the
                // chart reference, a picture's blip, an OLE object. Any of them
                // will do: this only records *what* is anchored here.
                _ => {
                    // `r:id` on a chart reference, `r:embed` on a picture's
                    // blip. Both are relationships and either identifies what
                    // is anchored here.
                    if let Some(id) = attr_raw(e, b"id").or_else(|| attr_raw(e, b"embed")) {
                        if rel_id.is_none() && looks_like_rel_id(&id) {
                            rel_id = Some(String::from_utf8_lossy(&id).into_owned());
                        }
                    }
                }
            },
            Event::End(ref e) => match end_local_name(e) {
                b"from" | b"to" => corner = Corner::None,
                b"col" | b"colOff" | b"row" | b"rowOff" => {
                    if let (Some(which), Ok(value)) = (field, text.trim().parse::<i64>()) {
                        let point = match corner {
                            Corner::From => &mut from,
                            Corner::To => &mut to,
                            Corner::None => {
                                text.clear();
                                field = None;
                                continue;
                            }
                        };
                        match which {
                            "col" => point.col = value.max(0) as u32,
                            "colOff" => point.col_offset = value,
                            "row" => point.row = value.max(0) as u32,
                            _ => point.row_offset = value,
                        }
                    }
                    text.clear();
                    field = None;
                }
                b"twoCellAnchor" | b"oneCellAnchor" | b"absoluteAnchor" => {
                    if let (Some(shape), Some(id)) = (kind, rel_id.take()) {
                        let anchor = match shape {
                            "two" => Anchor::TwoCell { from, to },
                            "one" => Anchor::OneCell {
                                from,
                                width: extent.0,
                                height: extent.1,
                            },
                            _ => Anchor::Absolute {
                                x: origin.0,
                                y: origin.1,
                                width: extent.0,
                                height: extent.1,
                            },
                        };
                        out.push(Anchored {
                            index,
                            anchor,
                            rel_id: id,
                            name: std::mem::take(&mut name),
                        });
                    }
                    kind = None;
                }
                _ => {}
            },
            Event::Eof => break,
            ref other if field.is_some() => {
                push_text(&mut text, other)?;
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(out)
}

/// `rId3` and nothing else. A `<xdr:cNvPr id="2">` is a shape id, not a
/// relationship — reading it as one would point every chart at part two.
fn looks_like_rel_id(raw: &[u8]) -> bool {
    raw.starts_with(b"rId") && raw.len() > 3
}

fn attr_i64(e: &BytesStart<'_>, want: &[u8]) -> Option<i64> {
    for a in crate::xml::attributes(e) {
        if strip_prefix(a.key.as_ref()) == want {
            return String::from_utf8_lossy(&a.value).trim().parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRAWING: &str = r#"<?xml version="1.0"?>
<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <xdr:twoCellAnchor>
    <xdr:from><xdr:col>4</xdr:col><xdr:colOff>76200</xdr:colOff>
              <xdr:row>2</xdr:row><xdr:rowOff>12700</xdr:rowOff></xdr:from>
    <xdr:to><xdr:col>12</xdr:col><xdr:colOff>0</xdr:colOff>
            <xdr:row>17</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
    <xdr:graphicFrame><xdr:nvGraphicFramePr><xdr:cNvPr id="2" name="Chart 1"/></xdr:nvGraphicFramePr>
      <a:graphic><a:graphicData><c:chart r:id="rId1"/></a:graphicData></a:graphic>
    </xdr:graphicFrame>
    <xdr:clientData/>
  </xdr:twoCellAnchor>
  <xdr:oneCellAnchor>
    <xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff>
              <xdr:row>20</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
    <xdr:ext cx="2857500" cy="1714500"/>
    <xdr:pic><a:blip r:embed="rId2"/></xdr:pic>
  </xdr:oneCellAnchor>
</xdr:wsDr>"#;

    #[test]
    fn a_two_cell_anchor_keeps_both_corners_and_their_offsets() {
        let found = parse("drawing1.xml", DRAWING.as_bytes()).expect("parses");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].rel_id, "rId1");
        match found[0].anchor {
            Anchor::TwoCell { from, to } => {
                assert_eq!(from.col, 4);
                assert_eq!(from.col_offset, 76_200);
                assert_eq!(from.row, 2);
                assert_eq!(to.col, 12);
                assert_eq!(to.row, 17);
            }
            ref other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_shape_id_is_not_mistaken_for_a_relationship() {
        // `<xdr:cNvPr id="2">` comes *before* the chart's `r:id` in document
        // order. Taking the first `id` attribute would point every chart at
        // part two.
        let found = parse("drawing1.xml", DRAWING.as_bytes()).expect("parses");
        assert_ne!(found[0].rel_id, "2");
    }

    #[test]
    fn a_drawing_keeps_the_name_it_was_given() {
        let found = parse("drawing1.xml", DRAWING.as_bytes()).expect("parses");
        assert_eq!(found[0].name, "Chart 1");
    }

    #[test]
    fn anchors_are_numbered_by_their_place_in_the_file() {
        let found = parse("drawing1.xml", DRAWING.as_bytes()).expect("parses");
        assert_eq!(found[0].index, 0);
        assert_eq!(found[1].index, 1);
    }

    #[test]
    fn a_one_cell_anchor_carries_its_own_size() {
        let found = parse("drawing1.xml", DRAWING.as_bytes()).expect("parses");
        match found[1].anchor {
            Anchor::OneCell {
                from,
                width,
                height,
            } => {
                assert_eq!(from.row, 20);
                assert_eq!(width, 2_857_500);
                assert_eq!(height, 1_714_500);
            }
            ref other => panic!("{other:?}"),
        }
    }
}
