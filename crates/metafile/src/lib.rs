//! Playing back a Windows metafile, so that a drawing made of records becomes
//! a drawing made of ink.
//!
//! **A metafile is not a picture; it is a recording of the calls that drew
//! one.** Word keeps a pasted diagram this way — an EMF of the GDI the drawing
//! program made — and a reader that only knows how to put decoded pixels in a
//! rectangle shows a page with a hole in it where the diagram was. An
//! engineering specification is mostly diagrams, so the hole is the document.
//!
//! What comes out is deliberately the smallest thing that can be drawn on a
//! screen, a page and a printer alike: filled outlines, stroked runs of
//! segments, and words on a baseline. Those are what `wp-print`'s own drawing
//! operations already carry, which is why nothing downstream of here has to
//! learn what a metafile is — the same three shapes a chart is drawn with
//! draw a diagram too.
//!
//! **What is not played.** Bitmaps inside the metafile (a photograph pasted
//! into a diagram), clipping regions, hatched and patterned brushes, and every
//! raster operation but the plain copy. A drawing that needs one of those
//! loses that part of itself rather than the whole; there is no fallback that
//! would be more honest than the ink that *is* understood.

pub mod emf;

/// One metafile, played.
///
/// The coordinates are points from the picture's own top-left corner, and
/// [`Picture::size`] is what the picture would be printed at unscaled — so a
/// caller with a box to fill scales by `box / size` and needs to know nothing
/// else.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    pub size: (f64, f64),
    pub prims: Vec<Prim>,
}

/// One piece of ink.
#[derive(Debug, Clone, PartialEq)]
pub enum Prim {
    /// A closed outline, filled. Never stroked: a metafile that wants both
    /// says so twice, and saying it once here would draw an edge the drawing
    /// did not ask for.
    Fill {
        points: Vec<(f64, f64)>,
        rgb: [u8; 3],
    },
    /// A run of connected segments, stroked. Curves arrive already flattened:
    /// what a device can draw is a line, and how finely a curve is cut is a
    /// decision better made once here than three times downstream.
    Stroke {
        points: Vec<(f64, f64)>,
        rgb: [u8; 3],
        width: f64,
    },
    /// Words on a baseline, starting at `x`.
    ///
    /// The advances are the metafile's own — a drawing program records where
    /// it put every character — so the line is set exactly as it was drawn
    /// rather than as this machine's copy of the face would set it.
    Text {
        x: f64,
        baseline: f64,
        text: String,
        advances: Vec<f64>,
        family: String,
        size: f64,
        bold: bool,
        italic: bool,
        rgb: [u8; 3],
        /// Degrees clockwise, which is the direction a page turns text.
        rotation: f64,
    },
}

/// Plays whichever kind of metafile the bytes are, or `None` when they are not
/// one this understands.
pub fn read(bytes: &[u8]) -> Option<Picture> {
    // An EMF says so in its own header rather than at the front of the file:
    // the first record is `EMR_HEADER` and its signature sits inside it.
    match bytes.get(40..44) {
        Some(b" EMF") => emf::play(bytes),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn bytes_that_are_not_a_metafile_are_refused_rather_than_played() {
        assert!(super::read(b"this is a text file").is_none());
        assert!(super::read(&[0u8; 128]).is_none());
    }
}
