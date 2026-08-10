//! Reading legacy `.xls` workbooks — Excel 97 through 2003 — into the
//! [`ss_model`] document model.
//!
//! **Read-only, by design and permanently.** `DESIGN.md` §9 puts the legacy
//! binary formats behind a save-as-modern escape hatch: a file opened here has
//! no package to write back to, so Calx offers Save As and not Save. That is
//! not a gap to be filled later. Round-tripping BIFF would mean reproducing
//! every record we chose not to model — and there are hundreds — in a format
//! whose own author has not written it since 2003.
//!
//! What that leaves is a reader whose job is to be *honest about its own
//! limits*. Where a record cannot be understood with certainty it is skipped,
//! and where a formula cannot be decompiled with certainty the cell keeps the
//! value Excel cached in it and loses only the expression. A cell showing the
//! right number with nothing behind it is a limitation a user can see. A cell
//! showing a formula that is subtly not the one in the file is not.
//!
//! The layers, outermost first:
//!
//! - `cfb_reader` opens the container and hands over the `Workbook` stream.
//! - [`record`] cuts that stream into records and rejoins the ones `CONTINUE`
//!   split.
//! - [`globals`] reads the first substream: sheets, strings, styles, names.
//! - [`sheet`] reads one substream per sheet into cells.
//! - [`formula`] turns a record's RPN token stream back into formula text.
//!
//! ## What is not read
//!
//! Charts, drawings, pictures, comments, conditional formatting, data
//! validation, autofilters, pivot tables, hyperlinks and print settings are all
//! in this format and none of them are modeled here. They are the same features
//! `ss-xlsx` reads from the modern format, in an entirely different encoding,
//! and each is its own body of work. Cells, values, formulas, styles, merges
//! and sheet geometry are what a legacy file is opened *for*.

#![forbid(unsafe_code)]

mod error;
mod formula;
mod func;
mod globals;
mod record;
mod sheet;
mod string;
mod style;

use std::path::Path;

use ss_model::Workbook;

pub use error::{Error, Result};

/// The name Excel 97 and later give the stream. `Book` is Excel 5 and 95, which
/// this reader refuses by version rather than by name — a `Book` stream can
/// hold BIFF8 when the file was saved for backward compatibility.
const STREAM_NAMES: [&str; 2] = ["Workbook", "Book"];

/// A legacy workbook, read.
#[derive(Debug)]
pub struct XlsDocument {
    pub workbook: Workbook,
    /// The 1904 date system, which Excel for Macintosh used. Every date serial
    /// in the file is 1462 days apart from the same date in a 1900 workbook, so
    /// this has to travel with the values.
    pub date_1904: bool,
}

pub fn open(path: impl AsRef<Path>) -> Result<XlsDocument> {
    read(std::fs::read(path)?)
}

pub fn read(data: Vec<u8>) -> Result<XlsDocument> {
    let cfb = cfb_reader::Cfb::read(data)?;
    let stream = STREAM_NAMES
        .iter()
        .find_map(|name| cfb.stream(name).transpose())
        .transpose()?
        .ok_or(Error::NotAWorkbook)?;
    from_stream(&stream)
}

/// Read a workbook stream that has already been taken out of its container.
fn from_stream(stream: &[u8]) -> Result<XlsDocument> {
    let globals = globals::read(stream)?;

    let mut workbook = Workbook::new();
    workbook.defined_names = globals.defined.clone();
    workbook.styles = ss_model::style::StyleTable::from_parts(globals.styles.clone());
    workbook.active_sheet = globals.active.min(globals.sheets.len().saturating_sub(1));

    for meta in &globals.sheets {
        let sheet = sheet::read(stream, meta, &globals, &mut workbook.strings)?;
        workbook.sheets.push(sheet);
    }

    Ok(XlsDocument {
        workbook,
        date_1904: globals.date_1904,
    })
}

#[cfg(test)]
mod tests;
