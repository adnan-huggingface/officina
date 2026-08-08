//! Reading xlsx workbooks into the [`ss_model`] document model.
//!
//! Reading never disturbs the Preservation Vault. Every part stays classified as
//! it was when the package was opened — the model built here is a *view* over the
//! package, not a replacement for it. That is what lets C4 land before the writer
//! exists: an open-and-save today still produces the original bytes, because the
//! save path has nothing modeled to re-serialize yet.
//!
//! See `DESIGN.md` §3 and §4.

#![forbid(unsafe_code)]

mod error;
mod parts;
mod shared_strings;
mod sheet;
mod workbook_part;
mod xml;

use std::io::{Read, Seek};
use std::path::Path;

use ooxml::Package;
use ss_model::{Sheet, SheetKind, Workbook};

pub use error::{Error, Result};

/// A workbook and the package it came from.
///
/// Both are kept. The model is what the UI and the formula engine edit; the
/// package is what gets written back, carrying every part we did not model.
pub struct XlsxDocument {
    pub workbook: Workbook,
    pub package: Package,
}

impl XlsxDocument {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        Self::read(std::io::BufReader::new(file))
    }

    pub fn read<R: Read + Seek>(reader: R) -> Result<Self> {
        let package = Package::read(reader)?;
        let workbook = build(&package)?;
        Ok(XlsxDocument { workbook, package })
    }
}

fn build(package: &Package) -> Result<Workbook> {
    let located = parts::locate(package)?;

    let workbook_part = package
        .part(&located.workbook)
        .ok_or_else(|| Error::MissingPart {
            referenced_by: "/_rels/.rels".to_owned(),
            rel_id: "officeDocument".to_owned(),
        })?;
    let meta = workbook_part::parse(located.workbook.as_str(), workbook_part.data())?;

    let mut wb = Workbook::new();
    wb.defined_names = meta.defined_names;

    // Shared strings first: the sheets index into it.
    let mut sst = Vec::new();
    if let Some(name) = &located.shared_strings {
        if let Some(part) = package.part(name) {
            sst = shared_strings::parse(name.as_str(), part.data(), &mut wb.strings)?;
        }
    }

    for entry in &meta.sheets {
        let mut sheet = Sheet::new(entry.name.clone());
        sheet.hidden = entry.hidden;

        // Every `<sheet>` gets a slot even when its part is missing or is not a
        // worksheet. `localSheetId` on a defined name indexes this list, so a gap
        // here re-points names at the wrong sheet.
        let target = entry
            .rel_id
            .as_deref()
            .and_then(|id| located.sheet_target(id));

        if let Some((kind, part_name)) = target {
            sheet.kind = match kind {
                "chartsheet" => SheetKind::Chart,
                "dialogsheet" => SheetKind::Dialog,
                "xlMacrosheet" | "macrosheet" => SheetKind::Macro,
                _ => SheetKind::Worksheet,
            };
            if sheet.kind.has_grid() {
                if let Some(part) = package.part(part_name) {
                    sheet::parse(
                        part_name.as_str(),
                        part.data(),
                        &mut sheet,
                        &sst,
                        &mut wb.strings,
                    )?;
                }
            }
        }

        wb.sheets.push(sheet);
    }

    Ok(wb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ss_model::{CellRef, CellValue};
    use std::io::Cursor;

    /// Builds a minimal but structurally real xlsx in memory.
    ///
    /// Written by hand rather than fixtured so the test states exactly which
    /// structures it depends on. Real-Word/Excel files are the fidelity harness's
    /// job (`cargo xtask fidelity`), not the unit tests'.
    fn workbook_zip(parts: &[(&str, &str)]) -> Vec<u8> {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in parts {
            zip.start_file(*name, opts).expect("zip entry starts");
            zip.write_all(body.as_bytes()).expect("zip entry writes");
        }
        zip.finish().expect("zip finishes").into_inner()
    }

    const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>
</Types>"#;

    const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

    const WORKBOOK_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>
</Relationships>"#;

    const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets>
  <definedNames><definedName name="Rate">0.15</definedName></definedNames>
</workbook>"#;

    const SHARED_STRINGS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="1" uniqueCount="1">
  <si><t>Active</t></si>
</sst>"#;

    const SHEET1: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1"><v>12.5</v></c></row>
    <row r="2"><c r="A2"><f>B1*2</f><v>25</v></c></row>
  </sheetData>
  <mergeCells count="1"><mergeCell ref="C1:D1"/></mergeCells>
</worksheet>"#;

    fn sample() -> Vec<u8> {
        workbook_zip(&[
            ("[Content_Types].xml", CONTENT_TYPES),
            ("_rels/.rels", ROOT_RELS),
            ("xl/workbook.xml", WORKBOOK),
            ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
            ("xl/sharedStrings.xml", SHARED_STRINGS),
            ("xl/worksheets/sheet1.xml", SHEET1),
        ])
    }

    #[test]
    fn reads_a_whole_workbook_end_to_end() {
        let doc = XlsxDocument::read(Cursor::new(sample())).expect("opens");
        let wb = &doc.workbook;

        assert_eq!(wb.sheets.len(), 1);
        assert_eq!(wb.sheets[0].name, "Data");

        let sheet = &wb.sheets[0];
        match sheet.get(CellRef::new(0, 0)).unwrap().value {
            CellValue::Text(id) => assert_eq!(wb.strings.resolve(id), "Active"),
            other => panic!("expected shared string, got {other:?}"),
        }
        assert_eq!(
            sheet.get(CellRef::new(0, 1)).unwrap().value,
            CellValue::Number(12.5)
        );
        assert_eq!(
            sheet
                .formula_at(CellRef::new(1, 0))
                .map(|f| f.text.as_str()),
            Some("B1*2")
        );
        assert_eq!(sheet.merges.len(), 1);

        assert_eq!(wb.resolve_name("Rate", None).unwrap().refers_to, "0.15");
    }

    #[test]
    fn reading_leaves_every_part_retained() {
        // The vault's guarantee: nothing is reclassified just because we read it.
        // If this fails, an open-and-save would re-serialize parts from a model
        // that does not yet round-trip them.
        let doc = XlsxDocument::read(Cursor::new(sample())).expect("opens");
        for part in doc.package.parts() {
            let expected = if part.is_rels() {
                ooxml::PartClass::Derived
            } else {
                ooxml::PartClass::Retained
            };
            assert_eq!(
                part.class, expected,
                "{} was reclassified by the reader",
                part.name
            );
        }
    }

    #[test]
    fn a_word_document_is_rejected_as_the_wrong_kind_of_file() {
        let ct = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#;
        let rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;
        let zip = workbook_zip(&[
            ("[Content_Types].xml", ct),
            ("_rels/.rels", rels),
            ("word/document.xml", "<document/>"),
        ]);
        match XlsxDocument::read(Cursor::new(zip)) {
            Err(Error::NotAWorkbook(_)) => {}
            Err(other) => panic!("should report the wrong file type, got {other}"),
            Ok(_) => panic!("a .docx must not open as a workbook"),
        }
    }

    #[test]
    fn a_sheet_whose_part_is_missing_still_holds_its_slot() {
        // Otherwise every localSheetId after the gap points at the wrong sheet.
        let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Gone" sheetId="1" r:id="rId9"/>
    <sheet name="Data" sheetId="2" r:id="rId1"/>
  </sheets>
  <definedNames><definedName name="Local" localSheetId="1">Data!$A$1</definedName></definedNames>
</workbook>"#;
        let zip = workbook_zip(&[
            ("[Content_Types].xml", CONTENT_TYPES),
            ("_rels/.rels", ROOT_RELS),
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
            ("xl/sharedStrings.xml", SHARED_STRINGS),
            ("xl/worksheets/sheet1.xml", SHEET1),
        ]);
        let doc = XlsxDocument::read(Cursor::new(zip)).expect("opens despite the dangling sheet");
        assert_eq!(doc.workbook.sheets.len(), 2);
        assert_eq!(doc.workbook.sheets[0].name, "Gone");
        assert!(doc.workbook.sheets[0].cells.is_empty());
        assert_eq!(doc.workbook.sheets[1].name, "Data");
        // The scoped name must still resolve against sheet index 1.
        assert_eq!(
            doc.workbook
                .resolve_name("Local", Some(1))
                .unwrap()
                .refers_to,
            "Data!$A$1"
        );
    }

    #[test]
    fn parts_are_found_by_relationship_not_by_conventional_path() {
        // The same workbook with everything moved off the usual paths. A reader
        // that hardcodes /xl/workbook.xml fails here; Strict-profile files and
        // several third-party exporters look exactly like this.
        let ct = CONTENT_TYPES
            .replace("/xl/workbook.xml", "/book/main.xml")
            .replace("/xl/worksheets/sheet1.xml", "/book/grids/g1.xml")
            .replace("/xl/sharedStrings.xml", "/book/strings.xml");
        let root = ROOT_RELS.replace("xl/workbook.xml", "book/main.xml");
        let wbrels = WORKBOOK_RELS
            .replace("worksheets/sheet1.xml", "grids/g1.xml")
            .replace("Target=\"sharedStrings.xml\"", "Target=\"strings.xml\"");
        let zip = workbook_zip(&[
            ("[Content_Types].xml", &ct),
            ("_rels/.rels", &root),
            ("book/main.xml", WORKBOOK),
            ("book/_rels/main.xml.rels", &wbrels),
            ("book/strings.xml", SHARED_STRINGS),
            ("book/grids/g1.xml", SHEET1),
        ]);
        let doc = XlsxDocument::read(Cursor::new(zip)).expect("opens from non-standard paths");
        assert_eq!(doc.workbook.sheets.len(), 1);
        assert_eq!(
            doc.workbook.sheets[0]
                .get(CellRef::new(0, 1))
                .unwrap()
                .value,
            CellValue::Number(12.5)
        );
    }
}
