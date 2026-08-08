//! The style table (`styles.xml`), read only for number formats.
//!
//! Two things are needed and nothing else: the custom format codes in
//! `<numFmts>`, and the `numFmtId` on each `<xf>` inside `<cellXfs>`. A cell's
//! `s` attribute indexes `cellXfs`, and that entry names the format.
//!
//! The trap is that most formats are never written down. `numFmtId="14"` is a
//! date and the file says nothing more; the code lives in a table baked into
//! every implementation of the format. A reader that only honours the codes it
//! can see shows every date in every document as a five-digit serial.
//!
//! `<cellStyleXfs>` is the *named-style* table and is a different list that also
//! contains `<xf>` elements. Reading both into one produces indices that are
//! plausible and wrong, so the two are kept apart here deliberately.

use std::collections::BTreeMap;

use quick_xml::events::Event;
use quick_xml::Reader;

use ss_model::style::StyleTable;

use crate::error::{xml_err, Result};
use crate::xml::{attr_text, attr_u32, end_local_name, local_name};

pub(crate) fn parse(part: &str, data: &[u8]) -> Result<StyleTable> {
    let mut reader = Reader::from_reader(data);
    reader.config_mut().check_end_names = true;

    let mut codes: BTreeMap<u32, String> = BTreeMap::new();
    let mut cell_formats: Vec<u32> = Vec::new();
    let mut in_cell_xfs = false;
    let mut buf = Vec::new();

    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err(part, e))?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => match local_name(e) {
                b"numFmt" => {
                    if let (Some(id), Some(code)) =
                        (attr_u32(e, b"numFmtId"), attr_text(e, b"formatCode"))
                    {
                        codes.insert(id, code);
                    }
                }
                b"cellXfs" => in_cell_xfs = true,
                // Only the ones inside `<cellXfs>`; `<cellStyleXfs>` is a
                // separate list that a cell's `s` attribute does not index.
                b"xf" if in_cell_xfs => {
                    cell_formats.push(attr_u32(e, b"numFmtId").unwrap_or(0));
                }
                _ => {}
            },
            Event::End(ref e) => {
                if end_local_name(e) == b"cellXfs" {
                    in_cell_xfs = false;
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(StyleTable::build(&codes, &cell_formats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ss_model::numfmt::FormatValue;
    use ss_model::StyleId;

    const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <numFmts count="1"><numFmt numFmtId="164" formatCode="&quot;$&quot;#,##0.00"/></numFmts>
  <cellStyleXfs count="1"><xf numFmtId="9" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
  <cellXfs count="3">
    <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
    <xf numFmtId="14" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
    <xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
  </cellXfs>
</styleSheet>"#;

    fn shown(table: &StyleTable, style: u32, value: f64) -> String {
        table
            .number_format(StyleId(style))
            .format(FormatValue::Number(value))
            .text
    }

    #[test]
    fn cell_formats_resolve_through_builtins_and_custom_codes() {
        let table = parse("styles.xml", STYLES.as_bytes()).expect("parses");
        assert_eq!(table.len(), 3, "only the cellXfs entries");
        assert_eq!(shown(&table, 0, 45352.0), "45352");
        assert_eq!(shown(&table, 1, 45352.0), "03-01-24");
        assert_eq!(shown(&table, 2, 1234.5), "$1,234.50");
    }

    #[test]
    fn the_named_style_table_is_not_the_cell_style_table() {
        // `<cellStyleXfs>` also holds `<xf>` elements. Counting them here would
        // shift every cell's style index by one and format the whole sheet
        // wrongly — plausibly, since the values would still be numbers.
        let table = parse("styles.xml", STYLES.as_bytes()).expect("parses");
        assert_eq!(
            shown(&table, 0, 0.5),
            "0.5",
            "style 0 is General, not the 0% from cellStyleXfs"
        );
    }

    #[test]
    fn a_workbook_with_no_styles_part_still_formats() {
        let table = parse("styles.xml", b"<styleSheet/>").expect("parses");
        assert!(table.is_empty());
        assert_eq!(shown(&table, 0, 1.5), "1.5");
    }
}
