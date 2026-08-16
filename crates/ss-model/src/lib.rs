//! Spreadsheet document model: workbook, sheets, sparse cell store, styles.
//!
//! The model is deliberately independent of xlsx. `ss-xlsx` maps between this and
//! the file format; keeping them separate is what lets csv, xls, and a future
//! format share one editing core — and what keeps format quirks from leaking into
//! the formula engine.

#![forbid(unsafe_code)]

pub mod cell;
pub mod color;
pub mod comment;
pub mod cond;
pub mod datetime;
pub mod filter;
pub mod formula;
pub mod numfmt;
pub mod picture;
pub mod pivot;
pub mod shift;
pub mod store;
pub mod strings;
pub mod style;
pub mod table;
pub mod workbook;

/// Charts live in their own crate, because a document has them too and there
/// is only one `<c:chartSpace>` in the world. Re-exported here so that a
/// workbook's chart is still `ss_model::chart::Chart`, which is where it has
/// always been.
pub use ::chart;
pub use ::chart::{Anchor, Chart, ChartKind, Series};
pub use cell::{column_index, column_name, Cell, CellError, CellRef, CellValue, FormulaId};
pub use color::{Color, Theme};
pub use comment::Comment;
pub use cond::{ConditionalFormat, DataValidation};
pub use filter::{AutoFilter, Compare, Criterion, FilterColumn, FilterKind};
pub use formula::{Formula, FormulaKind};
pub use numfmt::{format_general, FormatValue, Formatted, NumberFormat};
pub use picture::Picture;
pub use pivot::PivotTable;
pub use shift::{Axis, Move, Shift};
pub use store::CellStore;
pub use strings::{StrId, StringTable};
pub use style::{
    Alignment, Border, BorderStyle, CellFormat, Dxf, Edge, Fill, Font, HAlign, Look, Pattern,
    StyleId, StyleTable, Underline, VAlign,
};
pub use table::{Band, Table, TableStyle};
pub use workbook::{
    CellRange, DefinedName, Panes, Protection, Sheet, SheetKind, SheetView, Workbook,
};
