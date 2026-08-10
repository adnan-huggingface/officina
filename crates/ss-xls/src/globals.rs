//! The workbook globals substream: everything shared between the sheets.
//!
//! It runs from the file's first `BOF` to the matching `EOF` and holds the
//! sheet directory, the shared strings, the style tables, the defined names,
//! and the tables a 3-D reference is resolved through. Each sheet's own
//! substream then starts at an absolute offset given here, which is why this
//! has to be read first and completely.

use ss_model::{DefinedName, SheetKind};

use crate::error::{Error, Result};
use crate::record::{kind, u16_at, u32_at, Records};
use crate::style::Styles;

/// A `BOUNDSHEET` entry: a sheet's name and where its substream begins.
pub(crate) struct BoundSheet {
    pub name: String,
    pub offset: usize,
    pub hidden: bool,
    pub kind: SheetKind,
}

pub(crate) struct Globals {
    pub sheets: Vec<BoundSheet>,
    pub strings: Vec<String>,
    pub styles: ss_model::style::Parts,
    pub defined: Vec<DefinedName>,
    /// The names alone, in the order `ptgName` indexes them.
    pub names: Vec<String>,
    pub xti: Vec<(u16, i16, i16)>,
    pub internal: Vec<bool>,
    pub date_1904: bool,
    /// The sheet that was showing when the workbook was saved.
    pub active: usize,
}

/// A `NAME` record, kept whole until every name is known.
///
/// Names refer to each other, so their expressions cannot be decompiled while
/// the list is still being built.
struct RawName {
    name: String,
    /// One-based sheet index, or zero for a workbook-scoped name.
    sheet: u16,
    rgce: Vec<u8>,
}

pub(crate) fn read(stream: &[u8]) -> Result<Globals> {
    let mut records = Records::new(stream);
    let first = records.pop().ok_or(Error::NotAWorkbook)?;
    if first.kind != kind::BOF {
        return Err(Error::NotAWorkbook);
    }
    let version = u16_at(&first.body(), 0).unwrap_or(0);
    if version < 0x0600 {
        return Err(Error::OldVersion(version));
    }

    let mut styles = Styles::new();
    let mut sheets = Vec::new();
    let mut strings = Vec::new();
    let mut raw_names: Vec<RawName> = Vec::new();
    let mut xti = Vec::new();
    let mut internal = Vec::new();
    let mut date_1904 = false;
    let mut active = 0usize;

    while let Some(record) = records.pop() {
        let body = record.body();
        match record.kind {
            kind::EOF => break,
            // Nothing past this point can be read, and reading half of it would
            // present cipher text as content.
            kind::FILEPASS => return Err(Error::Encrypted),
            kind::BOUNDSHEET => {
                if let Some(sheet) = bound_sheet(&body) {
                    sheets.push(sheet);
                }
            }
            kind::SST => strings = crate::string::sst(&record.parts)?,
            kind::FONT => styles.font(&body),
            kind::FORMAT => styles.format(&body),
            kind::XF => styles.xf(&body),
            kind::PALETTE => styles.palette(&body),
            kind::DATEMODE => date_1904 = crate::style::date_1904(&body),
            // WINDOW1: the workbook's own window, whose `itabCur` is the tab
            // that was in front.
            0x003D => active = u16_at(&body, 10).unwrap_or(0) as usize,
            0x0018 => {
                if let Some(name) = raw_name(&body) {
                    raw_names.push(name);
                }
            }
            // SUPBOOK: one per linked workbook, plus one for this workbook
            // itself. `0x0401` in the second field is what marks that one.
            0x01AE => internal.push(u16_at(&body, 2) == Some(0x0401)),
            0x0017 => {
                let count = u16_at(&body, 0).unwrap_or(0) as usize;
                for i in 0..count {
                    let at = 2 + i * 6;
                    let (Some(book), Some(first), Some(last)) = (
                        u16_at(&body, at),
                        u16_at(&body, at + 2),
                        u16_at(&body, at + 4),
                    ) else {
                        break;
                    };
                    xti.push((book, first as i16, last as i16));
                }
            }
            _ => {}
        }
    }

    if sheets.is_empty() {
        return Err(Error::NotAWorkbook);
    }

    let names: Vec<String> = raw_names.iter().map(|n| n.name.clone()).collect();
    let sheet_names: Vec<String> = sheets.iter().map(|s| s.name.clone()).collect();
    let context = crate::formula::Context {
        sheets: &sheet_names,
        xti: &xti,
        internal: &internal,
        names: &names,
    };
    let defined = raw_names
        .iter()
        .map(|raw| DefinedName {
            name: raw.name.clone(),
            refers_to: match crate::formula::parse(
                &raw.rgce,
                ss_model::CellRef::new(0, 0),
                &context,
            ) {
                crate::formula::Parsed::Text(text) => text,
                // A name whose expression cannot be read keeps its name and
                // loses its target, which is the half that can be recovered.
                _ => String::new(),
            },
            scope: (raw.sheet > 0).then(|| raw.sheet as usize - 1),
        })
        .collect();

    Ok(Globals {
        sheets,
        strings,
        styles: styles.into_parts(),
        defined,
        names,
        xti,
        internal,
        date_1904,
        active,
    })
}

fn bound_sheet(body: &[u8]) -> Option<BoundSheet> {
    let offset = u32_at(body, 0)? as usize;
    let flags = u16_at(body, 4)?;
    let (name, _) = crate::string::short(body, 6)?;
    Some(BoundSheet {
        name,
        offset,
        // 0 visible, 1 hidden, 2 "very hidden" — which is hidden with the menu
        // item greyed out, and is still hidden.
        hidden: flags & 0x03 != 0,
        kind: match (flags >> 8) & 0xFF {
            0x01 => SheetKind::Macro,
            0x02 => SheetKind::Chart,
            0x06 => SheetKind::Dialog,
            _ => SheetKind::Worksheet,
        },
    })
}

fn raw_name(body: &[u8]) -> Option<RawName> {
    let flags = u16_at(body, 0)?;
    let chars = *body.get(3)? as usize;
    let cce = u16_at(body, 4)? as usize;
    let sheet = u16_at(body, 8)?;

    // The name string here has no count byte of its own — the count is at
    // offset 3 and only the encoding flag is inline.
    let wide = *body.get(14)? & 0x01 != 0;
    let bytes = chars * if wide { 2 } else { 1 };
    let raw = body.get(15..15 + bytes)?;
    let name = if flags & 0x0020 != 0 {
        // A built-in name is a single byte, not text: Print_Area and its kind.
        builtin(*raw.first()?)?.to_string()
    } else if wide {
        String::from_utf16_lossy(
            &raw.chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect::<Vec<_>>(),
        )
    } else {
        raw.iter().map(|b| *b as char).collect()
    };

    Some(RawName {
        name,
        sheet,
        rgce: body.get(15 + bytes..15 + bytes + cce)?.to_vec(),
    })
}

/// The names Excel reserves. The file stores the code; the code *is* the name.
fn builtin(code: u8) -> Option<&'static str> {
    Some(match code {
        0x00 => "Consolidate_Area",
        0x01 => "Auto_Open",
        0x02 => "Auto_Close",
        0x03 => "Extract",
        0x04 => "Database",
        0x05 => "Criteria",
        0x06 => "Print_Area",
        0x07 => "Print_Titles",
        0x08 => "Recorder",
        0x09 => "Data_Form",
        0x0A => "Auto_Activate",
        0x0B => "Auto_Deactivate",
        0x0C => "Sheet_Title",
        0x0D => "_FilterDatabase",
        _ => return None,
    })
}
