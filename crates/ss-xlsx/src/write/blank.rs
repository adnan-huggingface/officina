//! Authoring a package for a workbook that has never been in a file.
//!
//! "Never write a part we did not author or retain" cuts both ways: these parts
//! have no original to preserve, so they are ours to write — and being ours,
//! they are written as small as Excel will accept rather than as a copy of
//! whatever a template happened to contain.
//!
//! Only the skeleton is built here. The sheets go out with an empty
//! `<sheetData/>`, the shared string table with no entries, and the style table
//! with the one default style; the cells, the strings, and any format the user
//! typed are then put in by the same splice writers that edit a real file. One
//! code path writes cells, whether the workbook came from Excel or from nothing.

use ooxml::{Package, PartName, Relationship, Relationships, TargetMode};

use ss_model::{SheetKind, Workbook};

use crate::error::Result;
use crate::write::splice::escape_value;

const MAIN_NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL_BASE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

const WORKBOOK_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml";
const WORKSHEET_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";
const STYLES_TYPE: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml";
const SST_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml";

const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n";

/// Builds a package holding `workbook`'s structure but none of its contents.
///
/// Takes the workbook mutably to record which part each sheet was given. That
/// is the same identity a sheet read from a file carries, and without it the
/// very first save of a new workbook would look to the writer like a workbook
/// full of sheets it had never written.
pub fn package_for(workbook: &mut Workbook) -> Result<Package> {
    let mut package = Package::empty();

    let mut root = Relationships::new();
    root.insert(Relationship {
        id: "rId1".to_string(),
        rel_type: format!("{REL_BASE}/officeDocument"),
        target: "xl/workbook.xml".to_string(),
        mode: TargetMode::Internal,
    });
    put(&mut package, "/_rels/.rels", "", root.to_xml())?;

    let mut rels = Relationships::new();
    let mut sheets = String::new();
    for index in 0..workbook.sheets.len() {
        // A chart or dialog sheet has no grid to write and no part we could
        // invent for it, but it still occupies a slot: `localSheetId` on a
        // defined name counts every `<sheet>`, present part or not.
        if !matches!(workbook.sheets[index].kind, SheetKind::Worksheet) {
            continue;
        }
        let n = index + 1;
        let id = format!("rId{n}");
        let target = format!("worksheets/sheet{n}.xml");
        rels.insert(Relationship {
            id: id.clone(),
            rel_type: format!("{REL_BASE}/worksheet"),
            target: target.clone(),
            mode: TargetMode::Internal,
        });
        sheets.push_str(&format!(
            "<sheet name=\"{}\" sheetId=\"{n}\" r:id=\"{id}\"{}/>",
            escape_value(&workbook.sheets[index].name),
            if workbook.sheets[index].hidden {
                " state=\"hidden\""
            } else {
                ""
            },
        ));
        let part = format!("/xl/{target}");
        put(&mut package, &part, WORKSHEET_TYPE, worksheet())?;
        workbook.sheets[index].part = Some(part);
    }

    let styles_id = rels.next_id();
    rels.insert(Relationship {
        id: styles_id,
        rel_type: format!("{REL_BASE}/styles"),
        target: "styles.xml".to_string(),
        mode: TargetMode::Internal,
    });
    let sst_id = rels.next_id();
    rels.insert(Relationship {
        id: sst_id,
        rel_type: format!("{REL_BASE}/sharedStrings"),
        target: "sharedStrings.xml".to_string(),
        mode: TargetMode::Internal,
    });

    put(
        &mut package,
        "/xl/workbook.xml",
        WORKBOOK_TYPE,
        workbook_part(&sheets, workbook),
    )?;
    put(
        &mut package,
        "/xl/_rels/workbook.xml.rels",
        "",
        rels.to_xml(),
    )?;
    put(&mut package, "/xl/styles.xml", STYLES_TYPE, styles())?;
    put(
        &mut package,
        "/xl/sharedStrings.xml",
        SST_TYPE,
        shared_strings(),
    )?;

    Ok(package)
}

fn put(package: &mut Package, name: &str, content_type: &str, data: Vec<u8>) -> Result<()> {
    let name = PartName::new(name).map_err(crate::Error::Package)?;
    let content_type = if content_type.is_empty() {
        // Relationship parts are covered by the `rels` extension default, so
        // declaring them again would only add noise to [Content_Types].xml.
        package
            .content_types()
            .get(&name)
            .unwrap_or("application/xml")
            .to_string()
    } else {
        content_type.to_string()
    };
    package.put_part(name, &content_type, data);
    Ok(())
}

fn workbook_part(sheets: &str, workbook: &Workbook) -> Vec<u8> {
    let mut names = String::new();
    for name in &workbook.defined_names {
        let scope = match name.scope {
            Some(index) => format!(" localSheetId=\"{index}\""),
            None => String::new(),
        };
        names.push_str(&format!(
            "<definedName name=\"{}\"{scope}>{}</definedName>",
            escape_value(&name.name),
            crate::write::splice::escape_text(&name.refers_to),
        ));
    }
    let names = if names.is_empty() {
        String::new()
    } else {
        format!("<definedNames>{names}</definedNames>")
    };
    format!(
        "{DECL}<workbook xmlns=\"{MAIN_NS}\" xmlns:r=\"{REL_NS}\">\
         <sheets>{sheets}</sheets>{names}</workbook>"
    )
    .into_bytes()
}

fn worksheet() -> Vec<u8> {
    format!("{DECL}<worksheet xmlns=\"{MAIN_NS}\"><sheetData/></worksheet>").into_bytes()
}

fn shared_strings() -> Vec<u8> {
    format!("{DECL}<sst xmlns=\"{MAIN_NS}\" count=\"0\" uniqueCount=\"0\"/>").into_bytes()
}

/// The smallest style table Excel will open.
///
/// Every one of these elements is required by the schema even when empty, and
/// the second fill is not optional either: `fillId="1"` is reserved for the
/// gray125 pattern and Excel repairs a file whose fill table is shorter than two.
fn styles() -> Vec<u8> {
    format!(
        "{DECL}<styleSheet xmlns=\"{MAIN_NS}\">\
         <fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>\
         <fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill>\
         <fill><patternFill patternType=\"gray125\"/></fill></fills>\
         <borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders>\
         <cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>\
         <cellXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/></cellXfs>\
         <cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles>\
         </styleSheet>"
    )
    .into_bytes()
}
