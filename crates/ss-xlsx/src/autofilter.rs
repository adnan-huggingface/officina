//! `<autoFilter>`, read on its own.
//!
//! Kept out of the worksheet's main streaming pass and given its own scan for
//! one reason: the *writer* has to ask the same question the reader does, and it
//! has to ask it about a file it is halfway through copying. If a save always
//! replaced the element, every filter detail this crate does not model —
//! `<top10>`, `<dynamicFilter>`, `<colorFilter>`, `<iconFilter>`,
//! `hiddenButton`, date groupings — would be deleted by the act of saving a
//! sheet nobody had filtered. So the writer parses the element out of the
//! original bytes, compares it with the model, and puts the *original bytes*
//! back when the two agree. Two implementations of that comparison would drift;
//! this is the one.
//!
//! The scan is guarded by [`present`], a plain substring test, so a worksheet
//! with no filter costs one pass over the bytes and no XML parsing at all.

use quick_xml::events::Event;
use quick_xml::Reader;

use ss_model::{AutoFilter, Compare, Criterion, FilterKind};

use crate::error::{xml_err, Result};
use crate::xml::{attr_text, end_local_name, local_name, parse_bool};

/// Whether the part could contain an autofilter at all.
pub(crate) fn present(data: &[u8]) -> bool {
    const TAG: &[u8] = b"autoFilter";
    data.windows(TAG.len()).any(|w| w == TAG)
}

/// The first `<autoFilter>` in `data`, or `None`.
///
/// The *first*: a worksheet has at most one, but a `<table>` part embedded in
/// the same stream would have its own, and reading past the sheet's would
/// replace it with a table's.
pub(crate) fn parse(part: &str, data: &[u8]) -> Result<Option<AutoFilter>> {
    if !present(data) {
        return Ok(None);
    }
    let mut reader = Reader::from_reader(data);
    reader.config_mut().check_end_names = true;
    let mut buf = Vec::new();

    let mut filter: Option<AutoFilter> = None;
    let mut col: Option<u32> = None;
    let mut values: std::collections::BTreeSet<String> = Default::default();
    let mut blanks = false;
    let mut custom: Vec<Criterion> = Vec::new();
    let mut and = false;

    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err(part, e))?;
        match &event {
            Event::Start(e) | Event::Empty(e) => match local_name(e) {
                b"autoFilter" => {
                    if filter.is_some() {
                        break;
                    }
                    let Some(range) = attr_text(e, b"ref")
                        .as_deref()
                        .and_then(crate::sheet::parse_range_str)
                    else {
                        // A filter with no range covers nothing, and Excel does
                        // not write one; treating it as absent is the only
                        // reading that cannot mis-hide a row.
                        break;
                    };
                    filter = Some(AutoFilter::over(range));
                    if matches!(event, Event::Empty(_)) {
                        break;
                    }
                }
                b"filterColumn" if filter.is_some() => {
                    col = attr_text(e, b"colId").and_then(|v| v.trim().parse().ok());
                    values.clear();
                    blanks = false;
                    custom.clear();
                    and = false;
                }
                b"filters" => {
                    blanks = attr_text(e, b"blank")
                        .and_then(|v| parse_bool(v.as_bytes()))
                        .unwrap_or(false);
                }
                b"filter" => {
                    if let Some(v) = attr_text(e, b"val") {
                        values.insert(v);
                    }
                }
                b"customFilters" => {
                    and = attr_text(e, b"and")
                        .and_then(|v| parse_bool(v.as_bytes()))
                        .unwrap_or(false);
                }
                b"customFilter" => {
                    custom.push(Criterion {
                        op: attr_text(e, b"operator")
                            .and_then(|o| Compare::from_code(&o))
                            .unwrap_or(Compare::Equal),
                        value: attr_text(e, b"val").unwrap_or_default(),
                    });
                }
                _ => {}
            },
            Event::End(e) => match end_local_name(e) {
                b"filterColumn" => {
                    // A column with neither a value list nor a comparison is
                    // constraining nothing. Excel writes those, and carrying one
                    // would show a filtered column that filters nothing.
                    let kind = if !values.is_empty() || blanks {
                        Some(FilterKind::Values {
                            values: std::mem::take(&mut values),
                            blanks,
                        })
                    } else {
                        let mut criteria = std::mem::take(&mut custom).into_iter();
                        criteria.next().map(|first| FilterKind::Custom {
                            first,
                            second: criteria.next().map(|second| (and, second)),
                        })
                    };
                    if let (Some(f), Some(c), Some(kind)) = (filter.as_mut(), col.take(), kind) {
                        f.set(c, Some(kind));
                    }
                }
                b"autoFilter" => break,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(filter)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(xml: &str) -> Option<AutoFilter> {
        parse("sheet1.xml", xml.as_bytes()).expect("parses")
    }

    #[test]
    fn a_filter_with_no_criteria_is_the_arrows_and_nothing_else() {
        let f = filter(r#"<worksheet><autoFilter ref="A1:C9"/></worksheet>"#).expect("read");
        assert_eq!(f.range.start.to_a1(), "A1");
        assert_eq!(f.range.end.to_a1(), "C9");
        assert!(!f.is_filtering());
    }

    #[test]
    fn a_checkbox_list_comes_back_as_the_values_that_were_ticked() {
        let f = filter(
            r#"<worksheet><autoFilter ref="A1:C9"><filterColumn colId="1">
                 <filters blank="1"><filter val="North"/><filter val="South"/></filters>
               </filterColumn></autoFilter></worksheet>"#,
        )
        .expect("read");
        let column = f.column(1).expect("column 1 is constrained");
        match &column.kind {
            FilterKind::Values { values, blanks } => {
                assert!(values.contains("North") && values.contains("South"));
                assert!(*blanks);
            }
            other => panic!("expected a value list, got {other:?}"),
        }
    }

    #[test]
    fn a_custom_filter_keeps_both_halves_and_how_they_are_joined() {
        let f = filter(
            r#"<worksheet><autoFilter ref="A1:A9"><filterColumn colId="0">
                 <customFilters and="1">
                   <customFilter operator="greaterThan" val="10"/>
                   <customFilter operator="lessThanOrEqual" val="99"/>
                 </customFilters>
               </filterColumn></autoFilter></worksheet>"#,
        )
        .expect("read");
        match &f.column(0).expect("constrained").kind {
            FilterKind::Custom { first, second } => {
                assert_eq!(first.op, Compare::Greater);
                assert_eq!(first.value, "10");
                let (and, second) = second.as_ref().expect("two halves");
                assert!(*and);
                assert_eq!(second.op, Compare::LessEqual);
            }
            other => panic!("expected a custom filter, got {other:?}"),
        }
    }

    #[test]
    fn a_column_whose_filter_was_cleared_is_not_carried() {
        let f = filter(
            r#"<worksheet><autoFilter ref="A1:C9"><filterColumn colId="0"/></autoFilter></worksheet>"#,
        )
        .expect("read");
        assert!(!f.is_filtering());
    }

    #[test]
    fn a_sheet_with_no_filter_costs_nothing_and_reads_as_none() {
        assert!(filter(r#"<worksheet><sheetData/></worksheet>"#).is_none());
        assert!(!present(b"<worksheet><sheetData/></worksheet>"));
    }

    #[test]
    fn a_tables_own_filter_further_down_the_stream_is_not_the_sheets() {
        let f = filter(
            r#"<worksheet><autoFilter ref="A1:B2"/></worksheet><table><autoFilter ref="Z1:Z9"/></table>"#,
        )
        .expect("read");
        assert_eq!(f.range.end.to_a1(), "B2");
    }
}
