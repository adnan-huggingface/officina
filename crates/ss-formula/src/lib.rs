//! Formula lexer, parser, dependency graph, evaluation, and the function library.
//!
//! The pieces run in that order. `parser` turns text into an `Expr`; `graph`
//! reads the `Expr` to decide what depends on what and in which order to
//! recalculate; `eval` walks the `Expr` against a [`Context`] to produce a value.
//!
//! The function library is deliberately kept behind one entry point,
//! [`functions::call`], so an unknown function is `#NAME?` — a value — rather
//! than anything that can fail. A formula we cannot compute must never cost the
//! user their document.

#![forbid(unsafe_code)]

pub mod ast;
pub mod clip;
pub mod cond;
pub mod edit;
pub mod error;
pub mod eval;
pub mod filter;
pub mod find;
pub mod functions;
pub mod graph;
pub mod lexer;
pub mod parser;
pub mod sheets;
pub mod sort;
pub mod tools;
pub mod translate;
pub mod value;
pub mod workbook;

/// Serial dates live in the model — a cell value *is* a serial, and the grid
/// needs the calendar to render one as a date without the formula engine.
pub use ss_model::datetime;

pub use ast::{Area, BinaryOp, Expr, Reference, SheetRef, UnaryOp};
pub use edit::{apply, Change, Patch};
pub use error::{ParseError, ParseErrorKind};
pub use eval::{Context, Evaluator, Position};
pub use graph::{AreaRef, DependencyGraph, Node, Precedents};
pub use lexer::A1;
pub use parser::parse;
pub use ss_model::{Axis, Shift};
pub use translate::translate;
pub use value::{Array, Operand, RefSet, Value};
pub use workbook::{recalculate, Recalculation, WorkbookContext};
