//! Moving, resizing and removing anchored pictures in `xl/drawings/drawingN.xml`.
//!
//! A drawing part carries far more than we model — shapes, text boxes, OLE
//! objects, the whole of DrawingML's formatting vocabulary — so the anchors are
//! spliced in place, exactly as cells are. Everything outside the `<xdr:from>`,
//! `<xdr:to>` and `<xdr:ext>` of an anchor we changed comes back byte for byte,
//! and an anchor we did not change is not touched at all.
//!
//! Anchors are matched **by position**, counted over every `*Anchor` element in
//! the part. Geometry cannot be the identity here: the whole point of the edit
//! is that the geometry is different from what the file says.
//!
//! Watch item: a picture also carries a cached `<a:xfrm>` inside its `spPr`,
//! and this leaves it alone. The anchor is what Excel positions from — it has
//! to be, or a picture would not move when a column is widened — so the cache
//! is stale in the same way it is stale in Excel's own files between edits.
//! Rewriting it would mean converting our column measurements into absolute
//! EMUs and asserting they match Excel's, which is a much bigger claim than
//! this edit needs to make.

use std::collections::BTreeMap;

use quick_xml::events::Event;

use ss_model::chart::{Anchor, AnchorPoint};

use crate::error::Result;
use crate::write::splice::{close, open, prefix_of, Splicer};
use crate::xml::{end_local_name, local_name};

/// What is to become of one anchor: a new geometry, or removal.
pub(crate) type Wanted = BTreeMap<usize, Option<Anchor>>;

/// Rewrites the anchors named in `wanted` and leaves every other byte alone.
pub(crate) fn rewrite(part: &str, data: &[u8], wanted: &Wanted) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() + 64);
    let mut splicer = Splicer::new(part, data);

    // The `xdr:` prefix is learnt from the file rather than assumed: it is a
    // namespace prefix, and nothing obliges a writer to spell it that way.
    let mut prefix: Vec<u8> = b"xdr:".to_vec();
    let mut seen = 0usize;
    // Set while inside an anchor being rewritten. `Some(None)` is a deletion.
    let mut editing: Option<Option<Anchor>> = None;
    // Depth inside a `<from>`, `<to>` or `<ext>` whose contents are replaced.
    let mut suppress = 0usize;
    // How deep inside the anchor the current element sits. Zero means a
    // direct child, and only direct children may be rewritten: a
    // `<xdr:graphicFrame>` — a chart — carries an `<a:ext>` of its own
    // deeper in, a mandatory DrawingML element that must not be clobbered
    // into the drawing namespace just because its local name matches.
    let mut inside = 0usize;

    while let Some((event, span)) = splicer.next()? {
        let is_anchor_tag = |name: &[u8]| {
            matches!(name, b"twoCellAnchor" | b"oneCellAnchor" | b"absoluteAnchor")
        };
        // The depth of *this* element, measured before it opens.
        let at_depth = inside;
        if editing.is_some() {
            match &event {
                Event::Start(e) if !is_anchor_tag(local_name(e)) => inside += 1,
                Event::End(e) if !is_anchor_tag(end_local_name(e)) => {
                    inside = inside.saturating_sub(1);
                }
                _ => {}
            }
        }
        match &event {
            Event::Start(e) if local_name(e) == b"wsDr" => {
                prefix = prefix_of(e);
                out.extend_from_slice(splicer.bytes(span));
            }

            Event::Start(e)
                if matches!(
                    local_name(e),
                    b"twoCellAnchor" | b"oneCellAnchor" | b"absoluteAnchor"
                ) =>
            {
                let index = seen;
                seen += 1;
                editing = wanted.get(&index).cloned();
                inside = 0;
                match &editing {
                    // Deleted: the whole element goes, opening tag included.
                    Some(None) => {}
                    _ => out.extend_from_slice(splicer.bytes(span)),
                }
            }
            Event::End(e)
                if matches!(
                    end_local_name(e),
                    b"twoCellAnchor" | b"oneCellAnchor" | b"absoluteAnchor"
                ) =>
            {
                if !matches!(editing, Some(None)) {
                    out.extend_from_slice(splicer.bytes(span));
                }
                editing = None;
                suppress = 0;
            }

            // Everything inside a deleted anchor goes with it.
            _ if matches!(editing, Some(None)) => {}

            // The three elements that carry an anchor's geometry. Each is
            // rewritten whole, so the values inside are dropped rather than
            // edited one at a time.
            Event::Start(e)
                if matches!(local_name(e), b"from" | b"to")
                    && matches!(editing, Some(Some(_)))
                    && suppress == 0 =>
            {
                suppress = 1;
                let Some(Some(anchor)) = &editing else {
                    unreachable!("guarded above")
                };
                let corner = match local_name(e) {
                    b"from" => from_point(anchor),
                    _ => to_point(anchor),
                };
                match corner {
                    Some(point) => write_corner(&mut out, &prefix, local_name(e), point),
                    // An absolute anchor has no cells; its `<xdr:pos>` is
                    // handled below and `<from>`/`<to>` should not be there.
                    None => out.extend_from_slice(splicer.bytes(span)),
                }
            }
            Event::End(e) if matches!(end_local_name(e), b"from" | b"to") && suppress > 0 => {
                suppress = 0;
            }
            _ if suppress > 0 => {}

            // `<xdr:ext cx cy>` on a one-cell or absolute anchor, and
            // `<xdr:pos x y>` on an absolute one. Both are attribute-only.
            Event::Empty(e) | Event::Start(e)
                if local_name(e) == b"ext"
                    && matches!(editing, Some(Some(_)))
                    && at_depth == 0 =>
            {
                let Some(Some(anchor)) = &editing else {
                    unreachable!("guarded above")
                };
                match extent(anchor) {
                    Some((cx, cy)) => {
                        open(
                            &mut out,
                            &prefix,
                            b"ext",
                            &[
                                crate::write::splice::Set::to(b"cx", cx.to_string()),
                                crate::write::splice::Set::to(b"cy", cy.to_string()),
                            ],
                            true,
                        );
                    }
                    None => out.extend_from_slice(splicer.bytes(span)),
                }
            }
            Event::Empty(e) | Event::Start(e)
                if local_name(e) == b"pos"
                    && matches!(editing, Some(Some(_)))
                    && at_depth == 0 =>
            {
                let Some(Some(anchor)) = &editing else {
                    unreachable!("guarded above")
                };
                match anchor {
                    Anchor::Absolute { x, y, .. } => {
                        open(
                            &mut out,
                            &prefix,
                            b"pos",
                            &[
                                crate::write::splice::Set::to(b"x", x.to_string()),
                                crate::write::splice::Set::to(b"y", y.to_string()),
                            ],
                            true,
                        );
                    }
                    _ => out.extend_from_slice(splicer.bytes(span)),
                }
            }

            _ => out.extend_from_slice(splicer.bytes(span)),
        }
    }

    Ok(out)
}

fn from_point(anchor: &Anchor) -> Option<AnchorPoint> {
    match anchor {
        Anchor::TwoCell { from, .. } | Anchor::OneCell { from, .. } => Some(*from),
        Anchor::Absolute { .. } => None,
    }
}

fn to_point(anchor: &Anchor) -> Option<AnchorPoint> {
    match anchor {
        Anchor::TwoCell { to, .. } => Some(*to),
        _ => None,
    }
}

fn extent(anchor: &Anchor) -> Option<(i64, i64)> {
    match anchor {
        Anchor::OneCell { width, height, .. } | Anchor::Absolute { width, height, .. } => {
            Some((*width, *height))
        }
        Anchor::TwoCell { .. } => None,
    }
}

/// `<xdr:from><xdr:col>3</xdr:col>…`, in the order the schema fixes.
fn write_corner(out: &mut Vec<u8>, prefix: &[u8], tag: &[u8], point: AnchorPoint) {
    open(out, prefix, tag, &[], false);
    for (name, value) in [
        (&b"col"[..], point.col as i64),
        (b"colOff", point.col_offset),
        (b"row", point.row as i64),
        (b"rowOff", point.row_offset),
    ] {
        open(out, prefix, name, &[], false);
        out.extend_from_slice(value.to_string().as_bytes());
        close(out, prefix, name);
    }
    close(out, prefix, tag);
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRAWING: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><xdr:twoCellAnchor editAs="oneCell"><xdr:from><xdr:col>3</xdr:col><xdr:colOff>110612</xdr:colOff><xdr:row>0</xdr:row><xdr:rowOff>159775</xdr:rowOff></xdr:from><xdr:to><xdr:col>3</xdr:col><xdr:colOff>1926712</xdr:colOff><xdr:row>3</xdr:row><xdr:rowOff>75381</xdr:rowOff></xdr:to><xdr:pic><xdr:nvPicPr><xdr:cNvPr id="4" name="Picture 3"/><xdr:cNvPicPr/></xdr:nvPicPr><xdr:blipFill><a:blip r:embed="rId1" cstate="print"/><a:stretch><a:fillRect/></a:stretch></xdr:blipFill><xdr:spPr><a:xfrm><a:off x="2494935" y="159775"/><a:ext cx="1816100" cy="431800"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></xdr:spPr></xdr:pic><xdr:clientData/></xdr:twoCellAnchor><xdr:oneCellAnchor><xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>20</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:ext cx="2857500" cy="1714500"/><xdr:pic><xdr:nvPicPr><xdr:cNvPr id="5" name="Picture 4"/></xdr:nvPicPr><xdr:blipFill><a:blip r:embed="rId2"/></xdr:blipFill></xdr:pic><xdr:clientData/></xdr:oneCellAnchor></xdr:wsDr>"#;

    fn point(col: u32, col_offset: i64, row: u32, row_offset: i64) -> AnchorPoint {
        AnchorPoint {
            col,
            col_offset,
            row,
            row_offset,
        }
    }

    #[test]
    fn nothing_asked_for_means_nothing_changed() {
        let out = rewrite("drawing1.xml", DRAWING.as_bytes(), &Wanted::new()).expect("rewrites");
        assert_eq!(out, DRAWING.as_bytes());
    }

    #[test]
    fn a_moved_two_cell_anchor_gets_both_corners_rewritten() {
        let mut wanted = Wanted::new();
        wanted.insert(
            0,
            Some(Anchor::TwoCell {
                from: point(1, 5000, 4, 6000),
                to: point(2, 7000, 8, 8000),
            }),
        );
        let out = rewrite("drawing1.xml", DRAWING.as_bytes(), &wanted).expect("rewrites");
        let text = String::from_utf8(out).expect("utf-8");

        assert!(text.contains(
            "<xdr:from><xdr:col>1</xdr:col><xdr:colOff>5000</xdr:colOff>\
             <xdr:row>4</xdr:row><xdr:rowOff>6000</xdr:rowOff></xdr:from>"
        ));
        assert!(text.contains(
            "<xdr:to><xdr:col>2</xdr:col><xdr:colOff>7000</xdr:colOff>\
             <xdr:row>8</xdr:row><xdr:rowOff>8000</xdr:rowOff></xdr:to>"
        ));
        // Everything that is not the anchor's own geometry survives.
        assert!(text.contains(r#"<xdr:cNvPr id="4" name="Picture 3"/>"#));
        assert!(text.contains(r#"<a:blip r:embed="rId1" cstate="print"/>"#));
        assert!(text.contains(r#"<a:prstGeom prst="rect">"#));
        // And the second anchor is untouched.
        assert!(text.contains(r#"<xdr:ext cx="2857500" cy="1714500"/>"#));
    }

    #[test]
    fn a_resized_one_cell_anchor_gets_a_new_extent() {
        let mut wanted = Wanted::new();
        wanted.insert(
            1,
            Some(Anchor::OneCell {
                from: point(1, 0, 20, 0),
                width: 100,
                height: 200,
            }),
        );
        let out = rewrite("drawing1.xml", DRAWING.as_bytes(), &wanted).expect("rewrites");
        let text = String::from_utf8(out).expect("utf-8");
        assert!(text.contains(r#"<xdr:ext cx="100" cy="200"/>"#));
        // The first anchor's `<a:ext>` lives inside DrawingML, not the anchor,
        // and must not be caught by the same rule.
        assert!(text.contains(r#"<a:ext cx="1816100" cy="431800"/>"#));
    }

    #[test]
    fn a_deleted_anchor_takes_its_whole_element_with_it() {
        let mut wanted = Wanted::new();
        wanted.insert(0, None);
        let out = rewrite("drawing1.xml", DRAWING.as_bytes(), &wanted).expect("rewrites");
        let text = String::from_utf8(out).expect("utf-8");

        assert!(!text.contains("twoCellAnchor"));
        assert!(!text.contains("Picture 3"));
        assert!(!text.contains("rId1"));
        // The other one is still there, whole.
        assert!(text.contains("Picture 4"));
        assert!(text.contains(r#"<xdr:ext cx="2857500" cy="1714500"/>"#));
        assert!(text.ends_with("</xdr:wsDr>"));
    }

    #[test]
    fn deleting_every_anchor_leaves_a_valid_empty_drawing() {
        let mut wanted = Wanted::new();
        wanted.insert(0, None);
        wanted.insert(1, None);
        let out = rewrite("drawing1.xml", DRAWING.as_bytes(), &wanted).expect("rewrites");
        let text = String::from_utf8(out).expect("utf-8");
        assert!(text.contains("<xdr:wsDr"));
        assert!(text.ends_with("</xdr:wsDr>"));
        assert!(!text.contains("<xdr:pic>"));
    }
}
