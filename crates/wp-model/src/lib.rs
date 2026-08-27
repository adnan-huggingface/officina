//! Word document model: paragraph/run tree, styles, sections, numbering, tables.
//!
//! The model is deliberately independent of `.docx`. `wp-docx` maps between this
//! and the file format, `wp-layout` turns it into pages, and keeping all three
//! apart is what lets plain text, Markdown and `.doc` share one editing core —
//! and what keeps the format's quirks out of the layout engine.
//!
//! **Nothing here is resolved.** A paragraph stores what its own `<w:pPr>` said
//! and nothing more; the size of a run is a question for
//! [`style::StyleTable::resolve_run`], not a field. That is not an optimisation
//! but a correctness requirement: a model that flattened formatting on read
//! could not write the document back, because the file records the difference
//! between "this run is 12pt" and "this run inherits 12pt", and a user editing
//! the style expects the second kind to move.
//!
//! The same rule governs everything the model does not understand. Revisions,
//! comments, fields and content controls are carried from the first chunk rather
//! than from the one that makes them editable, because a reader that flattens
//! them has already destroyed them.
//!
//! Where to start reading: [`doc::Document`] for the tree, [`style`] for how
//! formatting is resolved out of it, and [`units`] for why there are five length
//! types rather than one.

#![forbid(unsafe_code)]

pub mod banding;
pub mod color;
pub mod doc;
pub mod field;
pub mod numbering;
pub mod outline;
pub mod prop;
pub mod revision;
pub mod section;
pub mod style;
pub mod table;
pub mod units;

pub use banding::{Band, CellAt, TablePart, TableScheme};
pub use color::{Color, Highlight, Theme, ThemeSlot};
pub use doc::{
    Block, Break, Document, Drawing, HeaderFooter, Hyperlink, Inline, Note, NoteKind, Paragraph,
    Piece, Run, Scope, Sdt, SdtKind, Settings, Wrap,
};
pub use field::Field;
pub use numbering::{AbstractNum, Counters, Level, Num, NumFormat, Numbering};
pub use outline::{Bookmark, Heading, TocEntry};
pub use prop::{
    Border, BorderStyle, Fonts, Indent, Justify, Lang, Layer, LineSpacing, NumRef, ParaBorders,
    ParaProps, RunProps, Script, Shading, ShadingPattern, Spacing, TabKind, TabLeader, TabStop,
    TextAlign, ThemeFont, Toggle, Toggles, Underline, UnderlineKind, VertAlign,
};
pub use revision::{Anchor, Comment, Mark, People, PropChange, Revision};
pub use section::{
    Bands, Columns, HeaderId, HeaderKind, HeaderRef, Orientation, PageBox, PageMargins, PageSize,
    SectionProps, SectionStart,
};
pub use style::{DocDefaults, Layers, Style, StyleId, StyleKind, StyleTable};
pub use table::{Cell, CellProps, Row, RowProps, Table, TableProps, VMerge, Width};
pub use units::{Eighth, Emu, HalfPoint, Line240, Pct50, Twips};
