//! Scriva — the word processor.
//!
//! A library plus a thin binary, the same split as Calx and for the same reason:
//! the caret model, the editing operations and the undo history are all testable
//! with no window, which is the only way they could be tested at all.

#![forbid(unsafe_code)]

pub mod app;
pub mod clip;
pub mod drawings;
pub mod edit;
pub mod find;
mod icons;
mod menus;
pub mod pictures;
pub mod publish;
pub mod revise;
pub mod shaper;
pub mod text;
pub mod view;
