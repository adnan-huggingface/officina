//! Formula lexer, parser, dependency graph, and incremental recalculation.
//!
//! Evaluation itself — the function library — is C6 and C7. What is here is the
//! machinery that decides *what* to evaluate and *in what order*, which is the
//! part that has to be right before any function can be trusted.

#![forbid(unsafe_code)]

pub mod ast;
pub mod error;
pub mod graph;
pub mod lexer;
pub mod parser;

pub use ast::{Area, BinaryOp, Expr, Reference, SheetRef, UnaryOp};
pub use error::{ParseError, ParseErrorKind};
pub use graph::{DependencyGraph, Node, Precedents};
pub use lexer::A1;
pub use parser::parse;
