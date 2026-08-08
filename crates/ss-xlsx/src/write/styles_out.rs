//! Adding to `styles.xml` without rewriting it.
//!
//! Typing a date into an empty cell needs a style that displays as a date, and
//! if the workbook has no such style one has to be created. That is the only
//! reason this exists at C10 — everything else in styles.xml (fonts, fills,
//! borders, alignment, the named-style table, differential formats, table
//! styles) is read by nobody yet, and reprinting the part would erase all of it.
//!
//! So the file's own entries are copied through and ours are appended. Appending
//! is not a stylistic choice: a cell's `s` attribute is an index into `cellXfs`,
//! and inserting anywhere but the end would silently reformat every cell in the
//! workbook after the insertion point.

use quick_xml::events::Event;

use ss_model::style::StyleTable;

use crate::error::Result;
use crate::write::splice::{close, open, prefix_of, raw_attr, retag, Set, Splicer};
use crate::xml::{end_local_name, local_name, parse_u32};

/// What the model has that the file does not.
pub(crate) struct Additions {
    /// `numFmtId` -> format code, for codes the file never declared.
    formats: Vec<(u32, String)>,
    /// The `numFmtId` of each `<xf>` to append to `cellXfs`.
    styles: Vec<u32>,
}

/// Compares the model's style table against the file's.
///
/// The file is read again rather than remembered from open, so that this is a
/// statement about the bytes being written and not about what a reader believed
/// some time earlier.
pub(crate) fn additions(part: &str, data: &[u8], styles: &StyleTable) -> Result<Additions> {
    let file = scan(part, data)?;
    let formats = styles
        .codes()
        .iter()
        .filter(|(id, _)| !file.declared.contains(id))
        .map(|(id, code)| (*id, code.clone()))
        .collect();
    let styles = (file.cell_xfs..styles.len())
        .map(|i| styles.format_id(ss_model::StyleId(i as u32)))
        .collect();
    Ok(Additions { formats, styles })
}

/// Rewrites `styles.xml` with the additions spliced into their two elements.
pub(crate) fn rewrite(part: &str, data: &[u8], add: &Additions) -> Result<Vec<u8>> {
    let file = scan(part, data)?;
    let mut out = Vec::with_capacity(data.len() + 256);
    let mut splicer = Splicer::new(part, data);
    let mut prefix = Vec::new();
    let mut done_formats = false;

    while let Some((event, span)) = splicer.next()? {
        match &event {
            Event::Start(e) | Event::Empty(e) if local_name(e) == b"styleSheet" => {
                prefix = prefix_of(e);
                out.extend_from_slice(splicer.bytes(span));
                // A workbook with no custom formats has no `<numFmts>` at all,
                // and the schema fixes its position: first child, before
                // `<fonts>`. Nothing else can carry it.
                if !file.has_num_fmts && !add.formats.is_empty() {
                    write_num_fmts(&mut out, &prefix, &add.formats, add.formats.len());
                    done_formats = true;
                }
            }

            Event::Start(e) | Event::Empty(e) if local_name(e) == b"numFmts" => {
                let count = file.declared.len() + add.formats.len();
                let sets = [Set::to(b"count", count.to_string())];
                if matches!(event, Event::Empty(_)) {
                    out.extend_from_slice(&retag(e, &sets, add.formats.is_empty()));
                    if !add.formats.is_empty() {
                        push_formats(&mut out, &prefix, &add.formats);
                        close(&mut out, &prefix, b"numFmts");
                    }
                    done_formats = true;
                } else {
                    out.extend_from_slice(&retag(e, &sets, false));
                }
            }
            Event::End(e) if end_local_name(e) == b"numFmts" => {
                if !done_formats {
                    push_formats(&mut out, &prefix, &add.formats);
                    done_formats = true;
                }
                out.extend_from_slice(splicer.bytes(span));
            }

            Event::Start(e) | Event::Empty(e) if local_name(e) == b"cellXfs" => {
                let count = file.cell_xfs + add.styles.len();
                let sets = [Set::to(b"count", count.to_string())];
                if matches!(event, Event::Empty(_)) {
                    out.extend_from_slice(&retag(e, &sets, add.styles.is_empty()));
                    if !add.styles.is_empty() {
                        push_styles(&mut out, &prefix, &add.styles);
                        close(&mut out, &prefix, b"cellXfs");
                    }
                } else {
                    out.extend_from_slice(&retag(e, &sets, false));
                }
            }
            Event::End(e) if end_local_name(e) == b"cellXfs" => {
                push_styles(&mut out, &prefix, &add.styles);
                out.extend_from_slice(splicer.bytes(span));
            }

            _ => out.extend_from_slice(splicer.bytes(span)),
        }
    }

    Ok(out)
}

fn write_num_fmts(out: &mut Vec<u8>, prefix: &[u8], formats: &[(u32, String)], count: usize) {
    open(
        out,
        prefix,
        b"numFmts",
        &[Set::to(b"count", count.to_string())],
        false,
    );
    push_formats(out, prefix, formats);
    close(out, prefix, b"numFmts");
}

fn push_formats(out: &mut Vec<u8>, prefix: &[u8], formats: &[(u32, String)]) {
    for (id, code) in formats {
        open(
            out,
            prefix,
            b"numFmt",
            &[
                Set::to(b"numFmtId", id.to_string()),
                Set::to(b"formatCode", code.clone()),
            ],
            true,
        );
    }
}

fn push_styles(out: &mut Vec<u8>, prefix: &[u8], styles: &[u32]) {
    for id in styles {
        // The font, fill, and border of the workbook default. A style we created
        // exists to carry a number format and nothing else; inheriting anything
        // else would need an understanding of the tables C11 has not built yet.
        open(
            out,
            prefix,
            b"xf",
            &[
                Set::to(b"numFmtId", id.to_string()),
                Set::to(b"fontId", "0"),
                Set::to(b"fillId", "0"),
                Set::to(b"borderId", "0"),
                Set::to(b"xfId", "0"),
                Set::to(b"applyNumberFormat", "1"),
            ],
            true,
        );
    }
}

/// What styles.xml already contains.
struct Scan {
    declared: std::collections::BTreeSet<u32>,
    has_num_fmts: bool,
    cell_xfs: usize,
}

fn scan(part: &str, data: &[u8]) -> Result<Scan> {
    let mut splicer = Splicer::new(part, data);
    let mut out = Scan {
        declared: Default::default(),
        has_num_fmts: false,
        cell_xfs: 0,
    };
    let mut in_cell_xfs = false;

    while let Some((event, _)) = splicer.next()? {
        match &event {
            Event::Start(e) | Event::Empty(e) => match local_name(e) {
                b"numFmts" => out.has_num_fmts = true,
                b"numFmt" => {
                    if let Some(id) = raw_attr(e, b"numFmtId").and_then(|a| parse_u32(&a.value)) {
                        out.declared.insert(id);
                    }
                }
                b"cellXfs" => in_cell_xfs = true,
                // `<cellStyleXfs>` holds `<xf>` elements too, and a cell's `s`
                // attribute does not index it.
                b"xf" if in_cell_xfs => out.cell_xfs += 1,
                _ => {}
            },
            Event::End(e) if end_local_name(e) == b"cellXfs" => in_cell_xfs = false,
            _ => {}
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const STYLES: &str = concat!(
        r#"<?xml version="1.0"?><styleSheet xmlns="http://x">"#,
        r#"<fonts count="1"><font><sz val="11"/></font></fonts>"#,
        r#"<cellStyleXfs count="1"><xf numFmtId="0"/></cellStyleXfs>"#,
        r#"<cellXfs count="1"><xf numFmtId="0" fontId="0"/></cellXfs>"#,
        r#"<dxfs count="0"/></styleSheet>"#,
    );

    fn rewritten(styles: &StyleTable) -> String {
        let add = additions("styles.xml", STYLES.as_bytes(), styles).expect("scans");
        let out = rewrite("styles.xml", STYLES.as_bytes(), &add).expect("writes");
        String::from_utf8(out).expect("utf-8")
    }

    #[test]
    fn a_workbook_nobody_edited_comes_back_unchanged() {
        let styles = StyleTable::build(&BTreeMap::new(), &[0]);
        assert_eq!(rewritten(&styles), STYLES);
    }

    #[test]
    fn a_typed_date_adds_a_style_that_points_at_a_builtin() {
        let mut styles = StyleTable::build(&BTreeMap::new(), &[0]);
        styles.style_for_format("mm-dd-yy");
        let out = rewritten(&styles);

        assert!(out.contains(r#"<cellXfs count="2">"#), "{out}");
        assert!(out.contains(r#"<xf numFmtId="14""#), "{out}");
        assert!(
            !out.contains("<numFmts"),
            "a built-in format is never declared: {out}"
        );
        assert!(
            out.contains(r#"<dxfs count="0"/>"#),
            "everything else survives: {out}"
        );
    }

    #[test]
    fn a_format_excel_has_no_id_for_gets_a_numfmts_element() {
        let mut styles = StyleTable::build(&BTreeMap::new(), &[0]);
        styles.style_for_format("0.000");
        let out = rewritten(&styles);

        // The schema fixes `<numFmts>` as the first child, before `<fonts>`.
        let fmts = out.find("<numFmts").expect("declared");
        let fonts = out.find("<fonts").expect("still there");
        assert!(fmts < fonts, "{out}");
        assert!(
            out.contains(r#"<numFmt numFmtId="164" formatCode="0.000"/>"#),
            "{out}"
        );
    }

    #[test]
    fn an_existing_numfmts_element_is_added_to_rather_than_replaced() {
        let file = concat!(
            r#"<styleSheet><numFmts count="1">"#,
            r#"<numFmt numFmtId="164" formatCode="&quot;$&quot;#,##0"/></numFmts>"#,
            r#"<cellXfs count="1"><xf numFmtId="164"/></cellXfs></styleSheet>"#,
        );
        let codes = BTreeMap::from([(164, "\"$\"#,##0".to_string())]);
        let mut styles = StyleTable::build(&codes, &[164]);
        // Not one of Excel's built-ins, so it has to be declared.
        styles.style_for_format("0.0000");

        let add = additions("styles.xml", file.as_bytes(), &styles).expect("scans");
        let out = String::from_utf8(rewrite("styles.xml", file.as_bytes(), &add).expect("writes"))
            .expect("utf-8");

        assert!(
            out.contains(r#"<numFmt numFmtId="164" formatCode="&quot;$&quot;#,##0"/>"#),
            "the file's own escaping survives: {out}"
        );
        assert!(
            out.contains(r#"numFmtId="165" formatCode="0.0000""#),
            "{out}"
        );
        assert!(out.contains(r#"<numFmts count="2">"#), "{out}");
    }
}
