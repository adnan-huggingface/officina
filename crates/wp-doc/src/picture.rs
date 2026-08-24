//! Inline pictures: the ones that sit in a line of text like a very large
//! character.
//!
//! A floating picture is anchored (see [`crate::art`]); an inline one is not.
//! Its `\x01` in the text carries a `sprmCPicLocation` that is an offset into
//! the `Data` stream, and there — not in the shared drawing layer — is a `PICF`
//! header followed by the shape and, usually, the picture's own bytes.
//!
//! **`sprmCPicLocation` does not always mean a picture.** The same property on
//! a character whose `sprmCFData` is set points at a form field's binary data
//! instead, and reading that as a `PICF` gives a picture of whatever the field
//! happens to contain. The caller has to know which it is looking at; this
//! module reads a `PICF` and says so if the bytes are not one.

use crate::art::{records, Blip, Shape};
use crate::fib::{u16 as read_u16, u32 as read_u32};

/// One picture, as the text asked for it to be drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    /// The size to draw it at, in twips — the size the author asked for, with
    /// whatever scaling they then applied already worked in. Not the size of
    /// the image: a photograph dragged smaller keeps every pixel.
    pub width: i32,
    pub height: i32,
    /// The bytes, when they are held with the picture rather than in the
    /// document's shared store.
    pub blip: Option<Blip>,
    /// The shape, whose `pib` names a picture in the shared store when the
    /// bytes are not here.
    pub shape: Option<Shape>,
}

/// Reads the `PICFAndOfficeArtData` at an offset into the `Data` stream.
///
/// `None` when there is no whole `PICF` there, which is what a caller that
/// guessed wrong about `sprmCFData` deserves to be told.
pub fn at(data: &[u8], offset: usize) -> Option<Picture> {
    /// The size of a `PICF`, which is also where the picture that follows it
    /// starts. The header states its own size and it has always been this.
    const PICF: usize = 68;

    let picf = data.get(offset..offset + PICF)?;
    // `mfpf.mm`. The one value that matters here is 0x0066, which says a
    // name follows the header and the drawing starts after it instead.
    let mm = read_u16(picf, 6);
    let width = scaled(read_u16(picf, 28), read_u16(picf, 32));
    let height = scaled(read_u16(picf, 30), read_u16(picf, 34));
    let from = match mm {
        // 0x0066 puts a counted name between the header and the drawing.
        0x0066 => offset + PICF + 1 + *data.get(offset + PICF)? as usize,
        _ => offset + PICF,
    };
    // `lcb` counts the whole structure, so the drawing ends where it says —
    // and a picture that claims more than the stream holds is read to the end
    // of the stream rather than refused, since the bytes that are there are
    // still the picture's.
    let end = offset
        .saturating_add(read_u32(picf, 0) as usize)
        .clamp(from, data.len());
    let art = data.get(from..end)?;

    let mut picture = Picture {
        width,
        height,
        blip: None,
        shape: None,
    };
    for record in records(art) {
        match record.kind {
            // `OfficeArtSpContainer` — the shape, which is often only a frame
            // round a `pib` naming a picture in the document's own store.
            0xF004 => picture.shape = crate::art::shape_of(record.data),
            _ => {
                if picture.blip.is_none() {
                    picture.blip = crate::art::blip_of(record);
                }
            }
        }
    }
    Some(picture)
}

/// A dimension and its scaling factor, which the file states separately: the
/// size the picture wants to be, and the thousandths of a percent of it the
/// author dragged it to.
fn scaled(goal: u16, scale: u16) -> i32 {
    let goal = goal as i32;
    match scale {
        0 => goal,
        scale => goal * scale as i32 / 1000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `PICF` stating a size and a scaling, with a drawing after it.
    fn picf(mm: u16, goal: (u16, u16), scale: (u16, u16), art: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; 68];
        let total = (68 + art.len()) as u32;
        out[0..4].copy_from_slice(&total.to_le_bytes());
        out[4..6].copy_from_slice(&68u16.to_le_bytes());
        out[6..8].copy_from_slice(&mm.to_le_bytes());
        out[28..30].copy_from_slice(&goal.0.to_le_bytes());
        out[30..32].copy_from_slice(&goal.1.to_le_bytes());
        out[32..34].copy_from_slice(&scale.0.to_le_bytes());
        out[34..36].copy_from_slice(&scale.1.to_le_bytes());
        out.extend(art);
        out
    }

    fn png_record() -> Vec<u8> {
        let mut body = vec![0xAAu8; 16];
        body.push(0xFF);
        body.extend(b"\x89PNG");
        let mut out = Vec::new();
        out.extend((0x6E0u16 << 4).to_le_bytes());
        out.extend(0xF01Eu16.to_le_bytes());
        out.extend((body.len() as u32).to_le_bytes());
        out.extend(body);
        out
    }

    #[test]
    fn the_size_is_the_one_the_author_dragged_it_to() {
        // Half size on both axes: a reader that draws `dxaGoal` puts a picture
        // twice as wide as Word does into the paragraph.
        let data = picf(0x0064, (2000, 1000), (500, 500), &png_record());
        let picture = at(&data, 0).expect("a picture");
        assert_eq!((picture.width, picture.height), (1000, 500));
        assert_eq!(picture.blip.expect("its bytes").data, b"\x89PNG");
    }

    #[test]
    fn a_scale_of_zero_is_no_scaling_rather_than_nothing_at_all() {
        let data = picf(0x0064, (2000, 1000), (0, 0), &png_record());
        let picture = at(&data, 0).expect("a picture");
        assert_eq!((picture.width, picture.height), (2000, 1000));
    }

    #[test]
    fn a_shape_file_picture_starts_after_the_name_the_header_does_not_count() {
        // `mm` 0x0066 puts a counted name between the header and the drawing.
        // Reading the drawing at the fixed offset finds the middle of a name.
        let art = png_record();
        let mut data = picf(0x0066, (100, 100), (1000, 1000), &[]);
        data.push(3);
        data.extend(b"abc");
        data.extend(&art);
        let total = data.len() as u32;
        data[0..4].copy_from_slice(&total.to_le_bytes());
        let picture = at(&data, 0).expect("a picture");
        assert_eq!(picture.blip.expect("its bytes").data, b"\x89PNG");
    }

    #[test]
    fn bytes_that_are_not_a_header_are_refused_rather_than_read() {
        assert!(at(&[0u8; 12], 0).is_none());
    }
}
