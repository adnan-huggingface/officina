//! `xl/tables/tableN.xml` — a table's range, its style name, and its emphases.
//!
//! Small, and worth reading precisely because it is small: nine attributes here
//! decide the whole appearance of a region whose cells may carry no style at
//! all. See `ss_model::table` for what is done with them.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use ss_model::{CellRange, CellRef, TableStyle};

use crate::error::{xml_err, Result};
use crate::xml::{attr_text, local_name};

/// Everything the reader takes from one table part.
pub(crate) struct Parsed {
    pub name: String,
    pub range: CellRange,
    pub header_rows: u32,
    pub totals_rows: u32,
    pub style: TableStyle,
    pub header_dxf: Option<u32>,
    pub data_dxf: Option<u32>,
    pub totals_dxf: Option<u32>,
}

pub(crate) fn parse(part: &str, data: &[u8]) -> Result<Option<Parsed>> {
    let mut reader = Reader::from_reader(data);
    reader.config_mut().check_end_names = false;
    let mut buf = Vec::new();
    let mut found: Option<Parsed> = None;

    loop {
        let ev = reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err(part, e))?;
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => match local_name(e) {
                b"table" => {
                    let Some(range) = attr_text(e, b"ref").and_then(|r| range_of(&r)) else {
                        // A table with no range is a table with no cells. Not
                        // an error: the part is preserved either way.
                        return Ok(None);
                    };
                    found = Some(Parsed {
                        name: attr_text(e, b"displayName")
                            .or_else(|| attr_text(e, b"name"))
                            .unwrap_or_default(),
                        range,
                        // Absent means one header row, not none. A table
                        // without headers says `headerRowCount="0"`.
                        header_rows: number(e, b"headerRowCount").unwrap_or(1),
                        totals_rows: number(e, b"totalsRowCount").unwrap_or(0),
                        style: TableStyle::default(),
                        header_dxf: number(e, b"headerRowDxfId"),
                        data_dxf: number(e, b"dataDxfId"),
                        totals_dxf: number(e, b"totalsRowDxfId"),
                    });
                }
                b"tableStyleInfo" => {
                    if let Some(table) = found.as_mut() {
                        table.style = TableStyle {
                            name: attr_text(e, b"name").filter(|n| !n.is_empty()),
                            row_stripes: flag(e, b"showRowStripes"),
                            column_stripes: flag(e, b"showColumnStripes"),
                            first_column: flag(e, b"showFirstColumn"),
                            last_column: flag(e, b"showLastColumn"),
                        };
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(found)
}

/// `E7:M8`, or a single cell for a one-cell table.
fn range_of(text: &str) -> Option<CellRange> {
    let (a, b) = match text.split_once(':') {
        Some((a, b)) => (a, b),
        None => (text, text),
    };
    Some(CellRange::new(CellRef::from_a1(a)?, CellRef::from_a1(b)?))
}

fn number(e: &BytesStart<'_>, name: &[u8]) -> Option<u32> {
    attr_text(e, name)?.trim().parse().ok()
}

/// `1` and `true` both mean on; absent means off.
fn flag(e: &BytesStart<'_>, name: &[u8]) -> bool {
    matches!(attr_text(e, name).as_deref(), Some("1") | Some("true"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" id="1"
       name="Table1" displayName="Table1" ref="E7:M8" totalsRowShown="0" headerRowDxfId="1">
  <tableColumns count="2">
    <tableColumn id="1" name="Fix Nibble"/>
    <tableColumn id="2" name="B0" dataDxfId="0"/>
  </tableColumns>
  <tableStyleInfo name="TableStyleMedium15" showFirstColumn="0" showLastColumn="0"
                  showRowStripes="1" showColumnStripes="0"/>
</table>"#;

    #[test]
    fn the_range_the_style_and_the_emphases_are_read() {
        let table = parse("table1.xml", TABLE.as_bytes())
            .expect("parses")
            .expect("a table");
        assert_eq!(table.name, "Table1");
        assert_eq!(table.range.start, CellRef::from_a1("E7").expect("E7"));
        assert_eq!(table.range.end, CellRef::from_a1("M8").expect("M8"));
        assert_eq!(table.style.name.as_deref(), Some("TableStyleMedium15"));
        assert!(table.style.row_stripes);
        assert!(!table.style.column_stripes);
        assert!(!table.style.first_column);
        assert_eq!(table.header_dxf, Some(1));
    }

    #[test]
    fn a_table_with_no_header_row_count_still_has_one() {
        // Absent is not zero. Read as zero, the headings become a striped body
        // row and the first row of data becomes the heading.
        let table = parse("table1.xml", TABLE.as_bytes())
            .expect("parses")
            .expect("a table");
        assert_eq!(table.header_rows, 1);
        assert_eq!(table.totals_rows, 0);
    }

    #[test]
    fn a_table_that_says_it_has_no_headers_is_believed() {
        let xml = TABLE.replace(r#"ref="E7:M8""#, r#"ref="E7:M8" headerRowCount="0""#);
        let table = parse("table1.xml", xml.as_bytes())
            .expect("parses")
            .expect("a table");
        assert_eq!(table.header_rows, 0);
    }

    #[test]
    fn a_part_that_is_not_a_table_is_silence_rather_than_an_error() {
        let out = parse("table1.xml", b"<something/>").expect("parses");
        assert!(out.is_none());
    }
}
