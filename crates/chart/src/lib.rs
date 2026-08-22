//! Office charts: the model both applications share, and the reader for the
//! part they both carry.
//!
//! A chart is the one thing a workbook and a document hold in exactly the same
//! shape. `xl/charts/chart1.xml` and `word/charts/chart1.xml` are both a
//! `<c:chartSpace>` — the same elements, the same series, the same caches — so
//! reading it twice would be reading it twice differently, and the second one
//! would be the one nobody tested.
//!
//! What is *not* shared is where the chart goes: a workbook pins it to cells
//! ([`Anchor`]), a document puts it in a line of text or on the page. That is
//! why [`Plot`] — everything a painter needs — is a type of its own.
//!
//! [`draw`] is the same argument one level up: a chart is drawn on a screen,
//! into a PDF and onto a printer, so where its ink goes is worked out once, in
//! numbers, and each renderer only chooses ink.

pub mod clipboard;
pub mod draw;
mod model;
pub mod read;

pub use model::{
    Anchor, AnchorPoint, Axis, Chart, ChartKind, Grouping, LegendPosition, Paint, Plot, Series,
    TickMark, EMU_PER_POINT,
};
