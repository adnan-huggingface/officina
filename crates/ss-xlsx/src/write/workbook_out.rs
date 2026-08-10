//! Rewriting the sheet list in `workbook.xml`.
//!
//! This is the part the writer never had, and its absence was recorded as a
//! watch item for five chunks: everything downstream walks the `<sheet>` entries
//! in this file and rewrites the parts they name, so a sheet the model grew had
//! nowhere to be written and a sheet the model dropped stayed in the file.
//!
//! What is spliced is `<sheets>` and `<definedNames>`, and nothing else. A real
//! `workbook.xml` also carries `<fileVersion>`, `<workbookPr>`, `<bookViews>`,
//! `<calcPr>`, `<pivotCaches>`, `<fileRecoveryPr>` and `extLst`, none of which
//! this crate models. They go back byte for byte.
//!
//! `<sheets>` is rewritten *whole* rather than edited element by element,
//! because the model's list is the answer to all four questions at once — which
//! sheets exist, what they are called, whether they are hidden, and what order
//! they are in — and reconciling four independent edits against one list is how
//! a reorder ends up duplicating a tab.

use quick_xml::events::Event;

use ooxml::PartName;
use ss_model::Workbook;

use crate::error::Result;
use crate::write::splice::{escape_text, escape_value, prefix_of, Splicer};

/// One `<sheet>` as it should be written.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Entry {
    pub name: String,
    /// The relationship id tying this entry to its part.
    pub rel_id: String,
    /// `sheetId`, kept from the file where the sheet came from one so that
    /// anything else in the package referring to it still resolves.
    pub sheet_id: u32,
    pub hidden: bool,
}

/// Rewrites `<sheets>` and `<definedNames>` to match `entries` and `book`.
pub(crate) fn rewrite(
    part: &str,
    data: &[u8],
    entries: &[Entry],
    book: &Workbook,
) -> Result<Vec<u8>> {
    let mut splicer = Splicer::new(part, data);
    let mut out = Vec::with_capacity(data.len() + 128);

    // Depth counters rather than booleans: `<definedNames>` holds elements with
    // text in them, and a boolean would be cleared by the first `</definedName>`.
    let mut skipping: Option<&[u8]> = None;
    let mut depth = 0usize;
    let mut wrote_names = false;
    let names_body = defined_names(book);

    while let Some((event, span)) = splicer.next()? {
        if let Some(tag) = skipping {
            match &event {
                Event::Start(e) if crate::xml::local_name(e) == tag => depth += 1,
                Event::End(e) if crate::xml::end_local_name(e) == tag => {
                    depth -= 1;
                    if depth == 0 {
                        skipping = None;
                    }
                }
                _ => {}
            }
            continue;
        }

        match &event {
            Event::Start(e) | Event::Empty(e) => {
                let local = crate::xml::local_name(e);
                let empty = matches!(event, Event::Empty(_));
                if local == b"sheets" {
                    let prefix = prefix_of(e);
                    out.extend_from_slice(&sheets_element(&prefix, entries));
                    if !empty {
                        skipping = Some(b"sheets");
                        depth = 1;
                    }
                    continue;
                }
                if local == b"definedNames" {
                    let prefix = prefix_of(e);
                    if !names_body.is_empty() {
                        out.extend_from_slice(&names_element(&prefix, &names_body));
                    }
                    wrote_names = true;
                    if !empty {
                        skipping = Some(b"definedNames");
                        depth = 1;
                    }
                    continue;
                }
                out.extend_from_slice(splicer.bytes(span));
            }
            // A workbook that had no `<definedNames>` and now needs one: it
            // belongs immediately after `</sheets>`, which is where the schema
            // puts it and the only position Excel accepts.
            Event::End(e) if crate::xml::end_local_name(e) == b"sheets" => {
                out.extend_from_slice(splicer.bytes(span));
            }
            _ => out.extend_from_slice(splicer.bytes(span)),
        }
    }

    if !wrote_names && !names_body.is_empty() {
        out = insert_after_sheets(out, &names_body);
    }
    Ok(out)
}

/// Puts an authored `<definedNames>` in after `</sheets>`.
///
/// Searched for in the output rather than tracked during the walk because the
/// element may have been the empty form, in which case there is no end event to
/// hang it off. The prefix is taken from whatever `</sheets…>` was written.
fn insert_after_sheets(out: Vec<u8>, names_body: &str) -> Vec<u8> {
    let Some(close) = find(&out, b"</").into_iter().find_map(|at| {
        let rest = &out[at + 2..];
        let end = rest.iter().position(|&b| b == b'>')?;
        let tag = &rest[..end];
        let local = match tag.iter().position(|&b| b == b':') {
            Some(i) => &tag[i + 1..],
            None => tag,
        };
        (local == b"sheets").then_some(at + 2 + end + 1)
    }) else {
        return out;
    };
    let mut merged = Vec::with_capacity(out.len() + names_body.len() + 32);
    merged.extend_from_slice(&out[..close]);
    merged.extend_from_slice(&names_element(b"", names_body));
    merged.extend_from_slice(&out[close..]);
    merged
}

fn find(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    haystack
        .windows(needle.len())
        .enumerate()
        .filter(|(_, w)| *w == needle)
        .map(|(i, _)| i)
        .collect()
}

fn sheets_element(prefix: &[u8], entries: &[Entry]) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(b'<');
    out.extend_from_slice(prefix);
    out.extend_from_slice(b"sheets>");
    for entry in entries {
        out.push(b'<');
        out.extend_from_slice(prefix);
        out.extend_from_slice(b"sheet name=\"");
        out.extend_from_slice(escape_value(&entry.name).as_bytes());
        out.extend_from_slice(format!("\" sheetId=\"{}\"", entry.sheet_id).as_bytes());
        if entry.hidden {
            out.extend_from_slice(b" state=\"hidden\"");
        }
        // `r:id` keeps the `r` prefix whatever the element prefix is: it is a
        // different namespace, and the file's own declaration of it is one of
        // the bytes being preserved.
        out.extend_from_slice(format!(" r:id=\"{}\"/>", escape_value(&entry.rel_id)).as_bytes());
    }
    out.extend_from_slice(b"</");
    out.extend_from_slice(prefix);
    out.extend_from_slice(b"sheets>");
    out
}

fn names_element(prefix: &[u8], body: &str) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(b'<');
    out.extend_from_slice(prefix);
    out.extend_from_slice(b"definedNames>");
    out.extend_from_slice(body.as_bytes());
    out.extend_from_slice(b"</");
    out.extend_from_slice(prefix);
    out.extend_from_slice(b"definedNames>");
    out
}

fn defined_names(book: &Workbook) -> String {
    let mut out = String::new();
    for name in &book.defined_names {
        let scope = match name.scope {
            Some(index) => format!(" localSheetId=\"{index}\""),
            None => String::new(),
        };
        out.push_str(&format!(
            "<definedName name=\"{}\"{scope}>{}</definedName>",
            escape_value(&name.name),
            escape_text(&name.refers_to),
        ));
    }
    out
}

/// The body of a worksheet part with nothing in it.
pub(crate) fn empty_worksheet() -> Vec<u8> {
    const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n";
    const MAIN_NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    format!("{DECL}<worksheet xmlns=\"{MAIN_NS}\"><sheetData/></worksheet>").into_bytes()
}

/// A worksheet part name nothing in `taken` is using.
///
/// Numbered rather than named after the sheet: a part name is a path, sheet
/// names contain spaces and non-ASCII, and Excel's own numbering bears no
/// relation to the tab order anyway.
pub(crate) fn free_part_name(taken: &[PartName], directory: &str) -> Result<PartName> {
    for n in 1.. {
        let candidate = format!("{directory}/sheet{n}.xml");
        if taken.iter().all(|p| p.as_str() != candidate) {
            return PartName::new(&candidate).map_err(crate::Error::Package);
        }
    }
    unreachable!("the loop is unbounded")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ss_model::DefinedName;

    const BOOK: &str = concat!(
        r#"<?xml version="1.0"?>"#,
        r#"<workbook xmlns:r="http://x"><fileVersion appName="xl"/>"#,
        r#"<sheets><sheet name="One" sheetId="1" r:id="rId1"/>"#,
        r#"<sheet name="Two" sheetId="2" r:id="rId2"/></sheets>"#,
        r#"<calcPr calcId="191029"/></workbook>"#,
    );

    fn entry(name: &str, rel: &str, id: u32, hidden: bool) -> Entry {
        Entry {
            name: name.into(),
            rel_id: rel.into(),
            sheet_id: id,
            hidden,
        }
    }

    fn out(xml: &str, entries: &[Entry], book: &Workbook) -> String {
        String::from_utf8(rewrite("workbook.xml", xml.as_bytes(), entries, book).expect("rewrites"))
            .expect("utf-8")
    }

    #[test]
    fn everything_outside_the_sheet_list_is_the_producers_bytes() {
        let book = Workbook::new();
        let same = [
            entry("One", "rId1", 1, false),
            entry("Two", "rId2", 2, false),
        ];
        let result = out(BOOK, &same, &book);
        assert!(result.starts_with(r#"<?xml version="1.0"?>"#));
        assert!(result.contains(r#"<fileVersion appName="xl"/>"#));
        assert!(
            result.contains(r#"<calcPr calcId="191029"/>"#),
            "calcPr is not ours to touch: {result}"
        );
    }

    #[test]
    fn a_sheet_added_removed_renamed_and_reordered_all_come_out_of_one_list() {
        let book = Workbook::new();
        let entries = [
            entry("Three", "rId7", 3, false),
            entry("Renamed", "rId1", 1, true),
        ];
        let result = out(BOOK, &entries, &book);
        assert!(result.contains(
            r#"<sheets><sheet name="Three" sheetId="3" r:id="rId7"/><sheet name="Renamed" sheetId="1" state="hidden" r:id="rId1"/></sheets>"#
        ), "{result}");
        assert!(!result.contains("rId2"), "the removed sheet is gone");
    }

    #[test]
    fn a_sheet_name_with_markup_in_it_is_escaped() {
        let book = Workbook::new();
        let entries = [entry("R&D <2026>", "rId1", 1, false)];
        let result = out(BOOK, &entries, &book);
        assert!(
            result.contains(r#"name="R&amp;D &lt;2026&gt;""#),
            "{result}"
        );
    }

    #[test]
    fn defined_names_are_replaced_where_they_were() {
        let xml = BOOK.replace(
            "<calcPr",
            r#"<definedNames><definedName name="Old">A1</definedName></definedNames><calcPr"#,
        );
        let mut book = Workbook::new();
        book.defined_names.push(DefinedName {
            name: "New".into(),
            refers_to: "Sheet1!$A$1:$A$9".into(),
            scope: Some(1),
        });
        let entries = [entry("One", "rId1", 1, false)];
        let result = out(&xml, &entries, &book);
        assert!(!result.contains("Old"));
        assert!(result.contains(
            r#"<definedNames><definedName name="New" localSheetId="1">Sheet1!$A$1:$A$9</definedName></definedNames>"#
        ), "{result}");
    }

    #[test]
    fn a_workbook_with_no_defined_names_gains_the_element_after_the_sheet_list() {
        // The schema fixes the order of workbook children, and Excel repairs a
        // file that puts definedNames anywhere else.
        let mut book = Workbook::new();
        book.defined_names.push(DefinedName {
            name: "Rate".into(),
            refers_to: "0.15".into(),
            scope: None,
        });
        let entries = [entry("One", "rId1", 1, false)];
        let result = out(BOOK, &entries, &book);
        let names_at = result.find("<definedNames>").expect("written");
        let sheets_end = result.find("</sheets>").expect("present");
        assert_eq!(names_at, sheets_end + "</sheets>".len());
    }

    #[test]
    fn removing_the_last_defined_name_removes_the_element_too() {
        let xml = BOOK.replace(
            "<calcPr",
            r#"<definedNames><definedName name="Old">A1</definedName></definedNames><calcPr"#,
        );
        let book = Workbook::new();
        let entries = [entry("One", "rId1", 1, false)];
        let result = out(&xml, &entries, &book);
        assert!(!result.contains("definedName"), "{result}");
    }

    #[test]
    fn a_free_part_name_skips_the_ones_in_use() {
        let taken: Vec<PartName> = ["/xl/worksheets/sheet1.xml", "/xl/worksheets/sheet3.xml"]
            .iter()
            .map(|n| PartName::new(n).expect("valid"))
            .collect();
        let found = free_part_name(&taken, "/xl/worksheets").expect("names one");
        assert_eq!(found.as_str(), "/xl/worksheets/sheet2.xml");
    }

    #[test]
    fn a_prefixed_workbook_keeps_its_prefix() {
        let xml = r#"<x:workbook xmlns:x="http://m"><x:sheets><x:sheet name="One" r:id="rId1"/></x:sheets></x:workbook>"#;
        let book = Workbook::new();
        let result = out(xml, &[entry("One", "rId1", 1, false)], &book);
        assert!(
            result.contains("<x:sheets><x:sheet name=\"One\""),
            "{result}"
        );
        assert!(result.contains("</x:sheets>"), "{result}");
    }
}
