//! Open Packaging Conventions container handling, and the Preservation Vault.
//!
//! Every part of an OPC package is classified on open. The classification decides
//! what happens to that part on save, and it is the mechanism behind the project's
//! central guarantee: we never destroy data we failed to understand.
//!
//! See `DESIGN.md` §3.

#![forbid(unsafe_code)]

pub mod content_types;
pub mod error;
pub mod name;
pub mod package;
pub mod rels;
mod xml;

pub use content_types::ContentTypes;
pub use error::{Error, Result};
pub use name::PartName;
pub use package::{Package, Part};
pub use rels::{Relationship, Relationships, TargetMode};

/// How a part is treated when the package is written back out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartClass {
    /// Parsed into a document model; re-serialized from that model on save.
    ///
    /// Unknown elements and attributes *within* a modeled part are still captured
    /// as opaque nodes and re-emitted in document order — "modeled" does not mean
    /// "fully understood".
    Modeled,

    /// Not understood. The original bytes are held and written back byte-identically.
    ///
    /// Custom XML, embedded OLE objects, ink annotations, vendor extensions, VBA
    /// projects. This is the default for anything unrecognized, deliberately: an
    /// unknown part must survive, not disappear.
    Retained,

    /// Regenerated from scratch on every save.
    ///
    /// Content types, relationship files, and document statistics — parts that
    /// describe the package rather than the document, and that would be wrong if
    /// carried over unchanged after an edit.
    Derived,
}

impl PartClass {
    /// The safe default for an unrecognized part.
    pub const fn default_for_unknown() -> Self {
        PartClass::Retained
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_parts_are_retained_not_dropped() {
        assert_eq!(PartClass::default_for_unknown(), PartClass::Retained);
    }
}
