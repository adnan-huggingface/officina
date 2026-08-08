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
pub mod error;
pub mod eval;
pub mod functions;
pub mod graph;
pub mod lexer;
pub mod parser;
pub mod value;
pub mod workbook;

pub use ast::{Area, BinaryOp, Expr, Reference, SheetRef, UnaryOp};
pub use error::{ParseError, ParseErrorKind};
pub use eval::{Context, Evaluator, Position};
pub use graph::{AreaRef, DependencyGraph, Node, Precedents};
pub use lexer::A1;
pub use parser::parse;
pub use value::{Array, Operand, RefSet, Value};
pub use workbook::{recalculate, Recalculation, WorkbookContext};
