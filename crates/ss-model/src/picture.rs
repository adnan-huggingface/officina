//! Pictures floating over a sheet.
//!
//! A logo at the top of a form is not decoration that can be skipped. On the
//! workbook this was built against, the first sheet's masthead — the company
//! name, in the company's own type — *is* an image, anchored over four rows of
//! otherwise empty cells. Read the cells and ignore the drawing and the sheet
//! opens with a blank space where the heading should be, which reads as a bug
//! in the file rather than in the reader.
//!
//! Like a chart, this is a view over a part kept verbatim. The bytes are the
//! image file's own, undecoded: whatever we make of them at paint time, the
//! package still holds exactly what it came with.

use std::sync::Arc;

use crate::chart::Anchor;

#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    /// The package part the bytes came from, e.g. `/xl/media/image1.png`.
    ///
    /// Doubles as the cache key for the decoded texture: two anchors sharing
    /// one image — a logo repeated on every sheet — decode once.
    pub part: String,
    /// `<xdr:cNvPr name>`. Excel's own "Picture 3" when nobody renamed it.
    pub name: String,
    pub anchor: Anchor,
    /// The image file exactly as the package holds it, still encoded.
    ///
    /// Shared rather than copied because a `Workbook` is cloned for undo, and
    /// a megabyte of PNG per keystroke is not a trade worth making.
    pub data: Arc<[u8]>,
    /// The part's content type, e.g. `image/png`. What the *package* claims,
    /// which is a better hint than the file extension and still only a hint.
    pub content_type: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::AnchorPoint;

    #[test]
    fn a_picture_shares_its_bytes_rather_than_copying_them() {
        let picture = Picture {
            part: "/xl/media/image1.png".into(),
            name: "Picture 3".into(),
            anchor: Anchor::OneCell {
                from: AnchorPoint::default(),
                width: 1,
                height: 1,
            },
            data: Arc::from(vec![1u8, 2, 3].into_boxed_slice()),
            content_type: "image/png".into(),
        };
        let copy = picture.clone();
        assert!(Arc::ptr_eq(&picture.data, &copy.data));
    }
}
