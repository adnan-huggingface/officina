//! `workbook.xml` — the sheet list and the defined names.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use ss_model::workbook::DefinedName;

use crate::error::{xml_err, Result};
use crate::xml::{attr_text, end_local_name, local_name, push_text};

/// One `<sheet>` entry, before its part has been read.
#[derive(Debug, Clone)]
pub(crate) struct SheetEntry {
    pub name: String,
    /// The `r:id` tying this entry to a part. Absent only in malformed files.
    pub rel_id: Option<String>,
    pub hidden: bool,
    /// `sheetId`, which is *not* the relationship id and *not* the position.
    ///
    /// Kept because other parts in the package refer to a sheet by it, so a
    /// rewritten list has to give each surviving sheet the number it had.
    pub sheet_id: u32,
}

#[derive(Debug, Default)]
pub(crate) struct WorkbookMeta {
    pub sheets: Vec<SheetEntry>,
    pub defined_names: Vec<DefinedName>,
    /// `<workbookView activeTab="..">` — which tab was showing when it was
    /// saved. An index into the sheet list including the hidden entries.
    pub active_tab: usize,
}

pub(crate) fn parse(part: &str, data: &[u8]) -> Result<WorkbookMeta> {
    let mut reader = Reader::from_reader(data);
    reader.config_mut().check_end_names = true;

    let mut meta = WorkbookMeta::default();
    let mut buf = Vec::new();

    // A defined name's target is element *text*, not an attribute, so it has to
    // be accumulated across events until the closing tag.
    let mut pending: Option<DefinedName> = None;
    let mut refers_to = String::new();

    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| xml_err(part, e))?
        {
            Event::Start(e) => match local_name(&e) {
                b"sheet" => meta.sheets.push(read_sheet(&e)),
                b"workbookView" => {
                    if let Some(tab) = attr_text(&e, b"activeTab") {
                        meta.active_tab = tab.trim().parse().unwrap_or(0);
                    }
                }
                b"definedName" => {
                    refers_to.clear();
                    pending = Some(read_defined_name(&e));
                }
                _ => {}
            },
            // `<definedName/>` with no body is legal: the name exists and points
            // nowhere. Handled here because no End event will ever arrive for it.
            Event::Empty(e) => match local_name(&e) {
                b"sheet" => meta.sheets.push(read_sheet(&e)),
                b"definedName" => meta.defined_names.push(read_defined_name(&e)),
                b"workbookView" => {
                    if let Some(tab) = attr_text(&e, b"activeTab") {
                        meta.active_tab = tab.trim().parse().unwrap_or(0);
                    }
                }
                _ => {}
            },
            Event::End(e) => {
                if end_local_name(&e) == b"definedName" {
                    if let Some(mut d) = pending.take() {
                        d.refers_to = std::mem::take(&mut refers_to);
                        meta.defined_names.push(d);
                    }
                }
            }
            Event::Eof => break,
            ev => {
                if pending.is_some() {
                    push_text(&mut refers_to, &ev)?;
                }
            }
        }
        buf.clear();
    }

    Ok(meta)
}

fn read_sheet(e: &BytesStart<'_>) -> SheetEntry {
    // `state` is absent for visible sheets. `veryHidden` is hidden too — it only
    // means the user cannot unhide it through the UI.
    let hidden = attr_text(e, b"state").is_some_and(|s| s == "hidden" || s == "veryHidden");
    SheetEntry {
        name: attr_text(e, b"name").unwrap_or_default(),
        // `r:id` after the prefix is stripped. `sheetId` is a different attribute
        // and an unrelated numbering; it is deliberately not used to find parts.
        rel_id: attr_text(e, b"id"),
        hidden,
        sheet_id: attr_text(e, b"sheetId")
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0),
    }
}

fn read_defined_name(e: &BytesStart<'_>) -> DefinedName {
    DefinedName {
        name: attr_text(e, b"name").unwrap_or_default(),
        refers_to: String::new(),
        // Absent means workbook scope. Present means an index into the sheet list
        // — which is why chart sheets must keep their slot in that list.
        scope: attr_text(e, b"localSheetId").and_then(|v| v.trim().parse::<usize>().ok()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(xml: &str) -> WorkbookMeta {
        parse("workbook.xml", xml.as_bytes()).expect("parses")
    }

    #[test]
    fn sheets_read_in_document_order_with_their_rel_ids() {
        let m = meta(
            r#"<workbook><sheets>
                 <sheet name="Data" sheetId="1" r:id="rId1"/>
                 <sheet name="Summary" sheetId="4" r:id="rId2"/>
               </sheets></workbook>"#,
        );
        assert_eq!(m.sheets.len(), 2);
        assert_eq!(m.sheets[0].name, "Data");
        assert_eq!(m.sheets[0].rel_id.as_deref(), Some("rId1"));
        assert_eq!(m.sheets[1].name, "Summary");
        assert_eq!(m.sheets[1].rel_id.as_deref(), Some("rId2"));
    }

    #[test]
    fn sheet_id_is_not_mistaken_for_the_relationship_id() {
        // sheetId and r:id number independently; using one for the other silently
        // loads the wrong sheet's contents.
        let m = meta(
            r#"<workbook><sheets><sheet name="S" sheetId="7" r:id="rId3"/></sheets></workbook>"#,
        );
        assert_eq!(m.sheets[0].rel_id.as_deref(), Some("rId3"));
        assert_eq!(m.sheets[0].sheet_id, 7, "and both are kept");
    }

    #[test]
    fn both_flavours_of_hidden_count_as_hidden() {
        let m = meta(
            r#"<workbook><sheets>
                 <sheet name="A" r:id="rId1"/>
                 <sheet name="B" state="hidden" r:id="rId2"/>
                 <sheet name="C" state="veryHidden" r:id="rId3"/>
               </sheets></workbook>"#,
        );
        assert!(!m.sheets[0].hidden);
        assert!(m.sheets[1].hidden);
        assert!(m.sheets[2].hidden, "veryHidden is still hidden");
    }

    #[test]
    fn sheet_names_with_entities_decode() {
        let m = meta(
            r#"<workbook><sheets><sheet name="R&amp;D &lt;2026&gt;" r:id="rId1"/></sheets></workbook>"#,
        );
        assert_eq!(m.sheets[0].name, "R&D <2026>");
    }

    #[test]
    fn defined_names_carry_scope_and_target() {
        let m = meta(
            r#"<workbook><definedNames>
                 <definedName name="Rate">0.15</definedName>
                 <definedName name="Local" localSheetId="2">Sheet3!$A$1</definedName>
               </definedNames></workbook>"#,
        );
        assert_eq!(m.defined_names.len(), 2);
        assert_eq!(m.defined_names[0].name, "Rate");
        assert_eq!(m.defined_names[0].refers_to, "0.15");
        assert_eq!(m.defined_names[0].scope, None, "workbook scope");
        assert_eq!(m.defined_names[1].scope, Some(2));
        assert_eq!(m.defined_names[1].refers_to, "Sheet3!$A$1");
    }

    #[test]
    fn an_empty_defined_name_element_still_yields_a_name() {
        // No End event ever arrives for these, so a Start-only reader loses them
        // and leaks its pending state into whatever element comes next.
        let m = meta(
            r#"<workbook><definedNames>
                 <definedName name="Empty"/>
                 <definedName name="After">Sheet1!$B$2</definedName>
               </definedNames></workbook>"#,
        );
        assert_eq!(m.defined_names.len(), 2);
        assert_eq!(m.defined_names[0].name, "Empty");
        assert_eq!(m.defined_names[0].refers_to, "");
        assert_eq!(m.defined_names[1].name, "After");
        assert_eq!(m.defined_names[1].refers_to, "Sheet1!$B$2");
    }

    #[test]
    fn defined_name_targets_keep_their_entities() {
        let m = meta(
            r#"<workbook><definedNames>
                 <definedName name="Q">'Q1 &amp; Q2'!$A$1:$A$9</definedName>
               </definedNames></workbook>"#,
        );
        assert_eq!(m.defined_names[0].refers_to, "'Q1 & Q2'!$A$1:$A$9");
    }

    #[test]
    fn prefixed_documents_parse_the_same() {
        let m = meta(
            r#"<x:workbook xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
                 <x:sheets><x:sheet name="P" r:id="rId1"/></x:sheets>
               </x:workbook>"#,
        );
        assert_eq!(m.sheets[0].name, "P");
    }
}
