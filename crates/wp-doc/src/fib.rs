//! The File Information Block: where everything else in a `.doc` is.
//!
//! A `.doc` has no directory of its own. The CFB gives three streams —
//! `WordDocument`, a table stream, and `Data` — and the FIB at the front of
//! `WordDocument` is the map of the other two: a hundred and eight pairs of
//! (offset, length) into the table stream, and the character counts that say
//! which part of the text is the body and which is a footnote.
//!
//! **Which table stream is not fixed.** Bit 9 of the FIB's flags chooses between
//! `0Table` and `1Table`, and Word alternates them as it saves. Reading the
//! wrong one gives a piece table full of plausible nonsense, which is worse than
//! an error.

use crate::{Error, Result};

/// The signature at the front of every Word 6–2003 document.
const IDENT: u16 = 0xA5EC;

/// The map of a `.doc`.
#[derive(Debug, Clone)]
pub struct Fib {
    /// The version this was written by. 193 is Word 97; anything below 101 is
    /// Word 6 or 95, whose piece table this does not read.
    pub version: u16,
    /// Whether the piece table is a real one. A "fast-saved" document has
    /// several, out of order; a plain one still has the structure.
    pub complex: bool,
    /// Whether the document is encrypted, in which case there is nothing to be
    /// read without the password.
    pub encrypted: bool,
    /// `1Table` rather than `0Table`.
    pub second_table: bool,
    /// Where the text ends, in characters, for each of the seven ranges that
    /// share the same coordinate space. See [`Fib::ranges`].
    pub counts: Counts,
    /// (offset, length) into the table stream, by index into `FibRgFcLcb97`.
    pairs: Vec<(u32, u32)>,
}

/// How many characters each part of the document takes up.
///
/// A `.doc` puts the body, the footnotes, the headers, the annotations, the
/// endnotes and the text boxes end to end in one coordinate space. Knowing where
/// each ends is the only way to tell a footnote from the paragraph it is under.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub text: u32,
    pub footnotes: u32,
    pub headers: u32,
    pub macros: u32,
    pub annotations: u32,
    pub endnotes: u32,
    pub textboxes: u32,
    pub header_textboxes: u32,
}

/// One of the parts a character position can fall in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Body,
    Footnotes,
    Headers,
    Macros,
    Annotations,
    Endnotes,
    TextBoxes,
    HeaderTextBoxes,
}

impl Counts {
    /// The character ranges of each part, in the order they are laid end to end.
    pub fn ranges(&self) -> Vec<(Part, u32, u32)> {
        let mut at = 0u32;
        let mut out = Vec::new();
        for (part, count) in [
            (Part::Body, self.text),
            (Part::Footnotes, self.footnotes),
            (Part::Headers, self.headers),
            (Part::Macros, self.macros),
            (Part::Annotations, self.annotations),
            (Part::Endnotes, self.endnotes),
            (Part::TextBoxes, self.textboxes),
            (Part::HeaderTextBoxes, self.header_textboxes),
        ] {
            out.push((part, at, at + count));
            at += count;
        }
        out
    }
}

/// Indices into `FibRgFcLcb97`, of the few things this reads.
pub mod field {
    /// The piece table, and any property modifiers in front of it.
    pub const CLX: usize = 33;
    /// The bin table of character property exceptions.
    pub const PLCFBTE_CHPX: usize = 12;
    /// The bin table of paragraph property exceptions.
    pub const PLCFBTE_PAPX: usize = 13;
    /// The stylesheet.
    pub const STSHF: usize = 1;
    /// The section table.
    pub const PLCFSED: usize = 6;
    /// The font table: what every `ftc` in the file is the name of.
    pub const STTBF_FFN: usize = 15;
    /// The header document's story boundaries.
    pub const PLCFHDD: usize = 11;
    /// Document properties: the one flag this reads is `fFacingPages`, bit 0
    /// of the first byte, for even-and-odd headers.
    pub const DOP: usize = 31;
    /// Where the floating shapes of the main document are anchored.
    pub const PLC_SPA_MOM: usize = 40;
    /// The same for the header document, which is where a watermark lives.
    pub const PLC_SPA_HDR: usize = 41;
    /// The whole drawing layer: the picture store, and every shape.
    pub const DGG_INFO: usize = 50;
    /// The list definitions. **Its length stops at the end of the `LSTF` array
    /// and the levels run on past it**, so this one is read from its offset
    /// rather than through [`Fib::slice`].
    pub const PLF_LST: usize = 73;
    /// The list instances, which are what a paragraph's `sprmPIlfo` names.
    pub const PLF_LFO: usize = 74;
}

impl Fib {
    pub fn read(stream: &[u8]) -> Result<Fib> {
        if stream.len() < 32 {
            return Err(Error::Malformed("the WordDocument stream is too short"));
        }
        if u16(stream, 0) != IDENT {
            return Err(Error::NotADocument);
        }
        let version = u16(stream, 2);
        let flags = u16(stream, 10);
        let complex = flags & 0x0004 != 0;
        let encrypted = flags & 0x0100 != 0;
        let second_table = flags & 0x0200 != 0;
        if version < 101 {
            // Word 6 and Word 95 write a FIB with no `csw`/`cslw` arrays at all,
            // and a different piece table. Saying so beats reading rubbish.
            return Err(Error::TooOld(version));
        }

        // The FIB is a chain of counted arrays, and each count is needed to find
        // the next one. There is no fixed offset for `rgFcLcb`.
        let mut at = 32usize;
        let csw = u16(stream, at) as usize;
        at += 2 + csw * 2;
        let cslw = u16(stream, at) as usize;
        at += 2;
        let rg_lw = at;
        at += cslw * 4;
        let count = u16(stream, at) as usize;
        at += 2;
        if stream.len() < at + count * 8 {
            return Err(Error::Malformed("the FIB runs past the end of the stream"));
        }
        let pairs = (0..count)
            .map(|index| {
                let base = at + index * 8;
                (u32(stream, base), u32(stream, base + 4))
            })
            .collect();

        // `rgLw97`: index 3 onward are the character counts, in the order the
        // parts are laid out.
        let lw = |index: usize| -> u32 {
            if cslw > index {
                u32(stream, rg_lw + index * 4)
            } else {
                0
            }
        };
        Ok(Fib {
            version,
            complex,
            encrypted,
            second_table,
            counts: Counts {
                text: lw(3),
                footnotes: lw(4),
                headers: lw(5),
                macros: lw(6),
                annotations: lw(7),
                endnotes: lw(8),
                textboxes: lw(9),
                header_textboxes: lw(10),
            },
            pairs,
        })
    }

    /// The name of the table stream this document's FIB points into.
    pub fn table_stream(&self) -> &'static str {
        match self.second_table {
            true => "1Table",
            false => "0Table",
        }
    }

    /// One (offset, length) pair, or `None` if it is empty or absent.
    pub fn at(&self, index: usize) -> Option<(usize, usize)> {
        let (offset, length) = self.pairs.get(index).copied()?;
        (length > 0).then_some((offset as usize, length as usize))
    }

    /// The bytes one pair points at, inside the table stream.
    pub fn slice<'a>(&self, table: &'a [u8], index: usize) -> Option<&'a [u8]> {
        let (offset, length) = self.at(index)?;
        table.get(offset..offset + length)
    }
}

pub(crate) fn u16(data: &[u8], at: usize) -> u16 {
    match data.get(at..at + 2) {
        Some(bytes) => u16::from_le_bytes([bytes[0], bytes[1]]),
        None => 0,
    }
}

pub(crate) fn u32(data: &[u8], at: usize) -> u32 {
    match data.get(at..at + 4) {
        Some(bytes) => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_that_is_not_a_document_is_recognised_rather_than_parsed() {
        let bytes = vec![0u8; 64];
        assert!(matches!(Fib::read(&bytes), Err(Error::NotADocument)));
    }

    #[test]
    fn the_parts_are_laid_end_to_end_in_one_coordinate_space() {
        // This is the thing about a `.doc` that a reader has to know: character
        // position 500 is a footnote or a header depending only on the counts.
        let counts = Counts {
            text: 100,
            footnotes: 20,
            headers: 30,
            ..Counts::default()
        };
        let ranges = counts.ranges();
        assert_eq!(ranges[0], (Part::Body, 0, 100));
        assert_eq!(ranges[1], (Part::Footnotes, 100, 120));
        assert_eq!(ranges[2], (Part::Headers, 120, 150));
    }
}
