//! Text layout: shaping, line breaking, and pagination.
//!
//! This is the part of a word processor that *is* the word processor. Everything
//! else — the reader, the writer, the ribbon — is machinery around the question
//! of where each line ends and where each page breaks.
//!
//! Three properties are designed in rather than added later:
//!
//! **Measurement is a trait.** [`shape::Shaper`] is all this crate knows about
//! fonts, so it can be laid out headlessly against [`shape::Fixed`] — a shaper
//! whose every glyph is half its point size. A layout engine tested against a
//! real face is tested against a moving target; one tested against arithmetic
//! can assert that a line holds eleven characters and mean it.
//!
//! **Every fragment carries where it came from.** [`inline::Source`] is a run
//! index and a byte range, so a click on a point of a line resolves to a
//! position in the document. Retrofitting that after the fact means laying the
//! text out twice.
//!
//! **The breaks are Word's, not the best ones.** Line breaking is greedy and
//! pagination takes the first fit, because matching Word's *breaks* is the goal
//! (`DESIGN.md` §5). A better algorithm would produce better-looking pages that
//! break in different places, which is the one thing this must not do.

#![forbid(unsafe_code)]

pub mod block;
pub mod field;
pub mod inline;
pub mod linebreak;
pub mod memo;
pub mod notes;
pub mod resolve;
pub mod shape;

pub use block::{Flow, Frame, Page, Placement};
pub use field::{FieldMark, FieldValues};
pub use inline::{Content, Context, Fragment, LaidParagraph, Line, ListLabel, Source};
pub use memo::Memo;
pub use notes::NoteMarks;
pub use resolve::TextStyle;
pub use shape::{Advance, Fixed, FontRequest, Metrics, Shaper};
