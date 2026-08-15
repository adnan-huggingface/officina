//! Putting laid-out pages on paper: a PDF for a file, GDI for a printer.
//!
//! Scriva's layout engine finishes its work in points from each page's
//! top-left corner, and the screen is only one of the places that geometry can
//! go. This crate takes the same [`wp_layout::block::Page`]s the screen paints
//! and renders them twice more — [`pdf`] writes a self-contained file with the
//! document's fonts embedded, [`win`] drives a Windows printer through the
//! system print dialog. Both go through [`ops`], the one flattening of a page
//! into drawing operations, so that what prints is what the screen showed down
//! to the choice of substitute font.
//!
//! The crate knows nothing about egui, packages, or the file system: fonts
//! arrive through [`Faces`] and images arrive decoded, so the whole of the PDF
//! path is testable on a machine with no window and no fonts.

pub mod ops;
pub mod pdf;
mod ttf;
#[cfg(windows)]
pub mod win;

use std::sync::Arc;

/// Supplies the font file a document's face name resolves to.
///
/// The implementation must answer with the *same* resolution the screen used —
/// the exact-name table first, the generic shape as fallback — or the exported
/// page wears different type than the one the user approved. Returning the
/// same [`Arc`] for the same underlying file is what lets one file serve
/// several document names without being embedded twice.
pub trait Faces {
    fn face(&mut self, family: &str, bold: bool, italic: bool) -> Option<Arc<[u8]>>;
}

/// One decoded image, plus its original bytes when they were a JPEG a PDF can
/// carry as-is.
pub struct Raster {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA, 8 bits each — what every renderer here consumes.
    pub rgba: Vec<u8>,
    /// The part's own bytes, when passing them through beats re-encoding.
    pub jpeg: Option<Vec<u8>>,
}
