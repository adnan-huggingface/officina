//! Spreadsheet document model: workbook, sheets, sparse cell store, styles.
//!
//! The model is deliberately independent of xlsx. `ss-xlsx` maps between this and
//! the file format; keeping them separate is what lets csv, xls, and a future
//! format share one editing core — and what keeps format quirks from leaking into
//! the formula engine.

#![forbid(unsafe_code)]

pub mod cell;
pub mod color;
pub mod cond;
pub mod datetime;
pub mod formula;
pub mod numfmt;
pub mod shift;
pub mod store;
pub mod strings;
pub mod style;
pub mod workbook;

pub use cell::{column_index, column_name, Cell, CellError, CellRef, CellValue, FormulaId};
pub use color::{Color, Theme};
pub use cond::{ConditionalFormat, DataValidation};
pub use formula::{Formula, FormulaKind};
pub use numfmt::{format_general, FormatValue, Formatted, NumberFormat};
pub use shift::{Axis, Shift};
pub use store::CellStore;
pub use strings::{StrId, StringTable};
pub use style::{
    Alignment, Border, BorderStyle, CellFormat, Dxf, Edge, Fill, Font, HAlign, Look, Pattern,
    StyleId, StyleTable, Underline, VAlign,
};
pub use workbook::{CellRange, DefinedName, Sheet, SheetKind, Workbook};
