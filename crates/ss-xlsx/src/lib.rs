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

mod autofilter;
mod chart;
mod drawing;
mod error;
mod parts;
mod pivot;
mod shared_strings;
mod sheet;
mod styles;
mod table;
mod theme;
mod workbook_part;
mod write;
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

    // The theme before the styles: almost every colour in a modern workbook is
    // an index into the theme's scheme rather than an RGB triple, and a style
    // table built without it resolves the whole palette to nothing.
    let theme = match located.theme.as_ref().and_then(|name| {
        package
            .part(name)
            .map(|part| theme::parse(name.as_str(), part.data()))
    }) {
        Some(read) => read?,
        None => ss_model::color::Theme::default(),
    };

    // Styles before the sheets: a cell's `s` attribute indexes this table, and
    // without it every date in the document is a five-digit number.
    if let Some(name) = &located.styles {
        if let Some(part) = package.part(name) {
            wb.styles = styles::parse(name.as_str(), part.data(), theme)?;
        }
    }

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
            // Remembered so that a sheet the user later moves or deletes can
            // still be matched to its part. Position in the file's list is not
            // an identity once tabs can be dragged.
            sheet.part = Some(part_name.as_str().to_string());
            sheet.kind = match kind {
                "chartsheet" => SheetKind::Chart,
                "dialogsheet" => SheetKind::Dialog,
                "xlMacrosheet" | "macrosheet" => SheetKind::Macro,
                _ => SheetKind::Worksheet,
            };
            if sheet.kind.has_grid() {
                if let Some(part) = package.part(part_name) {
                    let extras = sheet::parse(
                        part_name.as_str(),
                        part.data(),
                        &mut sheet,
                        &sst,
                        &mut wb.strings,
                    )?;
                    let drawn = read_drawing(package, part_name, extras.drawing.as_deref())?;
                    sheet.charts = drawn.charts;
                    sheet.pictures = drawn.pictures;
                    sheet.tables = read_tables(package, part_name)?;
                    sheet.pivots = read_pivots(package, part_name)?;
                }
            }
        }

        wb.sheets.push(sheet);
    }

    // Clamped rather than trusted: `activeTab` is an index into a list this
    // file also wrote, and a file that disagrees with itself must not open on
    // a sheet that is not there.
    wb.active_sheet = meta.active_tab.min(wb.sheets.len().saturating_sub(1));

    Ok(wb)
}

/// Everything one drawing part anchors to a sheet.
#[derive(Default)]
struct Drawn {
    charts: Vec<ss_model::Chart>,
    pictures: Vec<ss_model::Picture>,
}

/// Follows worksheet -> drawing -> whatever is at the end, and reads it.
///
/// Two relationship hops, and a failure at any of them is silence rather than
/// an error: a chart we cannot read is a chart we do not draw, and the part is
/// preserved either way. Refusing to open the workbook over it would be much
/// worse than showing it without its charts.
///
/// Charts and pictures are read in one pass because they share the drawing:
/// one `<xdr:wsDr>` holds both, and which is which is settled by the content
/// type of the part each anchor points at, not by the anchor itself.
fn read_drawing(
    package: &Package,
    sheet_part: &ooxml::PartName,
    drawing_rel: Option<&str>,
) -> Result<Drawn> {
    let Some(rel_id) = drawing_rel else {
        return Ok(Drawn::default());
    };
    let Some(drawing_name) = resolve(package, sheet_part, rel_id) else {
        return Ok(Drawn::default());
    };
    let Some(drawing_part) = package.part(&drawing_name) else {
        return Ok(Drawn::default());
    };

    let anchored = drawing::parse(drawing_name.as_str(), drawing_part.data())?;
    let mut drawn = Drawn::default();
    for found in anchored {
        let Some(target) = resolve(package, &drawing_name, &found.rel_id) else {
            continue;
        };
        let Some(part) = package.part(&target) else {
            continue;
        };
        // The relationship may point at a chart, a picture, or an OLE object.
        // The content type is what says which.
        if part.content_type.starts_with("image/") {
            drawn.pictures.push(ss_model::Picture {
                part: target.as_str().to_string(),
                drawing_part: drawing_name.as_str().to_string(),
                anchor_index: found.index,
                name: found.name,
                anchor: found.anchor,
                data: std::sync::Arc::from(part.data()),
                content_type: part.content_type.clone(),
            });
            continue;
        }
        if !part.content_type.contains("chart") {
            continue;
        }
        let body = chart::parse(target.as_str(), part.data())?;
        let Some(kind) = body.kind else {
            continue;
        };
        drawn.charts.push(ss_model::Chart {
            part: target.as_str().to_string(),
            drawing_part: drawing_name.as_str().to_string(),
            anchor_index: found.index,
            anchor: found.anchor,
            kind,
            grouping: body.grouping,
            horizontal: body.horizontal,
            title: body.title,
            title_ref: body.title_ref,
            legend: body.legend,
            series: body.series,
        });
    }
    Ok(drawn)
}

/// The drawing a worksheet points at, if it points at one.
///
/// Found by relationship *type* rather than by the `<drawing r:id>` in the
/// sheet, because the writer needs it even when nothing in the model still
/// refers to it — a sheet whose only picture was deleted has no picture left
/// to name the part.
pub(crate) fn drawing_of(
    package: &Package,
    sheet_part: &ooxml::PartName,
) -> Option<ooxml::PartName> {
    let rels = package.relationships(sheet_part).ok()?;
    let rel = rels.iter().find(|r| r.rel_type.ends_with("/drawing"))?;
    rel.resolve(sheet_part)?.ok()
}

/// The anchors in a drawing that hold a picture or a chart, and the geometry
/// the *file* gives them — which is what a changed anchor is compared
/// against. Both kinds, and deliberately in the same map: the writer decides
/// deletion by "in the file, absent from the model", so an anchor kind this
/// map covered but the model did not would be silently stripped on save.
pub(crate) fn object_anchors(
    package: &Package,
    drawing_name: &ooxml::PartName,
) -> Result<std::collections::BTreeMap<usize, ss_model::Anchor>> {
    let mut out = std::collections::BTreeMap::new();
    let Some(part) = package.part(drawing_name) else {
        return Ok(out);
    };
    for found in drawing::parse(drawing_name.as_str(), part.data())? {
        let Some(target) = resolve(package, drawing_name, &found.rel_id) else {
            continue;
        };
        let Some(target_part) = package.part(&target) else {
            continue;
        };
        if target_part.content_type.starts_with("image/") {
            out.insert(found.index, found.anchor);
            continue;
        }
        // Chart anchors, but only the charts the *reader* keeps: a chart of a
        // kind we do not recognize never reaches the model, and an anchor in
        // this map with no model object behind it would read as "deleted".
        if target_part.content_type.contains("chart") {
            let known = chart::parse(target.as_str(), target_part.data())
                .ok()
                .and_then(|body| body.kind)
                .is_some();
            if known {
                out.insert(found.index, found.anchor);
            }
        }
    }
    Ok(out)
}

/// A relationship id, resolved against the part that declared it.
fn resolve(package: &Package, from: &ooxml::PartName, rel_id: &str) -> Option<ooxml::PartName> {
    let rels = package.relationships(from).ok()?;
    let rel = rels.iter().find(|r| r.id == rel_id)?;
    rel.resolve(from)?.ok()
}

/// Reads the tables a worksheet points at.
///
/// Found by relationship type, like pivots: a worksheet does not name its
/// tables from inside `sheetData`. A part that will not parse is skipped
/// silently — it is preserved either way, and a table we cannot read costs the
/// user its shading, not their file.
fn read_tables(package: &Package, sheet_part: &ooxml::PartName) -> Result<Vec<ss_model::Table>> {
    let Ok(rels) = package.relationships(sheet_part) else {
        return Ok(Vec::new());
    };
    let targets: Vec<ooxml::PartName> = rels
        .iter()
        .filter(|rel| rel.rel_type.ends_with("/table"))
        .filter_map(|rel| rel.resolve(sheet_part)?.ok())
        .collect();

    let mut out = Vec::new();
    for name in targets {
        let Some(part) = package.part(&name) else {
            continue;
        };
        let Some(parsed) = table::parse(name.as_str(), part.data())? else {
            continue;
        };
        out.push(ss_model::Table {
            part: name.as_str().to_string(),
            name: parsed.name,
            range: parsed.range,
            header_rows: parsed.header_rows,
            totals_rows: parsed.totals_rows,
            style: parsed.style,
            header_dxf: parsed.header_dxf,
            data_dxf: parsed.data_dxf,
            totals_dxf: parsed.totals_dxf,
        });
    }
    Ok(out)
}

/// Reads whatever pivot tables a worksheet points at.
///
/// Found by relationship *type* rather than by a reference in the sheet: unlike
/// a drawing, a pivot table is not named from inside `sheetData` at all. As
/// with charts, a part we cannot read is silently skipped — the definition is
/// preserved either way, and refusing to open the workbook over it would be far
/// worse than showing the cells without knowing they are a pivot.
fn read_pivots(
    package: &Package,
    sheet_part: &ooxml::PartName,
) -> Result<Vec<ss_model::PivotTable>> {
    let Ok(rels) = package.relationships(sheet_part) else {
        return Ok(Vec::new());
    };
    let targets: Vec<ooxml::PartName> = rels
        .iter()
        .filter(|rel| rel.rel_type.ends_with("/pivotTable"))
        .filter_map(|rel| rel.resolve(sheet_part)?.ok())
        .collect();

    let mut out = Vec::new();
    for name in targets {
        let Some(part) = package.part(&name) else {
            continue;
        };
        let layout = pivot::parse_table(name.as_str(), part.data())?;
        // The cache is one more hop, and it is where the field names live.
        let cache = package
            .relationships(&name)
            .ok()
            .and_then(|rels| {
                rels.iter()
                    .find(|rel| rel.rel_type.ends_with("/pivotCacheDefinition"))
                    .and_then(|rel| rel.resolve(&name)?.ok())
            })
            .and_then(|target| {
                package
                    .part(&target)
                    .map(|part| pivot::parse_cache(target.as_str(), part.data()))
            })
            .transpose()?
            .unwrap_or_default();

        if let Some(table) = pivot::build(name.as_str(), layout, cache) {
            out.push(table);
        }
    }
    Ok(out)
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
