//! Spreadsheet document model: workbook, sheets, sparse cell store, styles.
//!
//! The model is deliberately independent of xlsx. `ss-xlsx` maps between this and
//! the file format; keeping them separate is what lets csv, xls, and a future
//! format share one editing core — and what keeps format quirks from leaking into
//! the formula engine.

#![forbid(unsafe_code)]

pub mod cell;
pub mod store;
pub mod strings;
pub mod style;
pub mod workbook;

pub use cell::{column_index, column_name, Cell, CellError, CellRef, CellValue, FormulaId};
pub use store::CellStore;
pub use strings::{StrId, StringTable};
pub use style::StyleId;
pub use workbook::{CellRange, DefinedName, Sheet, Workbook};
