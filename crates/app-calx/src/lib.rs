//! Calx — the spreadsheet application.
//!
//! Split into a library and a thin binary so the grid has a real public surface:
//! the geometry, the selection model, and the frame planner are all testable
//! without a window, and `pub` on them means what it says.

#![forbid(unsafe_code)]

pub mod grid;
