//! The piece table: where each character actually is.
//!
//! A `.doc` does not hold its text in one place. Word's fast save appends the
//! edits to the end of the file and leaves the old text where it was, so the
//! document is a list of *pieces*, each saying "characters 1200 to 1450 live at
//! byte 0x4A20". Reading the file from front to back gives the text of several
//! different drafts interleaved. This is the single thing that separates a
//! reader that works from one that produces convincing rubbish.
//!
//! **A piece is 8-bit or 16-bit, and the flag is hidden in the address.** Bit 30
//! of the byte offset means "this run is Windows-1252, one byte per character",
//! and the real offset is the rest of the value halved. Word does this to keep
//! ASCII documents small, and it means a piece table read as UTF-16 throughout
//! gives Chinese for English text.

use crate::fib::{u16 as read_u16, u32 as read_u32, Fib};
use crate::{Error, Result};

/// One run of characters that live together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Piece {
    /// Character position of the first character, in the document's one
    /// coordinate space.
    pub start: u32,
    /// One past the last.
    pub end: u32,
    /// Byte offset into the `WordDocument` stream.
    pub offset: usize,
    /// One byte per character (Windows-1252) rather than two (UTF-16).
    pub compressed: bool,
    /// The property modifier this piece carries, if any. Rarely used, and not
    /// applied here — recorded so it is not silently claimed to be absent.
    pub prm: u16,
}

impl Piece {
    /// The number of characters in the piece.
    pub fn len(&self) -> u32 {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

/// The piece table of a document.
#[derive(Debug, Clone, Default)]
pub struct Pieces {
    pub pieces: Vec<Piece>,
}

impl Pieces {
    /// Reads the `Clx` the FIB points at.
    ///
    /// A `Clx` is a run of property modifiers, each introduced by `0x01`, then
    /// exactly one piece table introduced by `0x02`. The modifiers have to be
    /// walked past rather than searched over: `0x02` is an ordinary byte inside
    /// them, and looking for it finds one every time.
    pub fn read(fib: &Fib, table: &[u8]) -> Result<Pieces> {
        let Some(clx) = fib.slice(table, crate::fib::field::CLX) else {
            return Err(Error::Malformed("the document has no piece table"));
        };
        let mut at = 0usize;
        while let Some(&marker) = clx.get(at) {
            match marker {
                0x01 => {
                    // `Prc`: a signed 16-bit length, then that many bytes.
                    let length = read_u16(clx, at + 1) as usize;
                    at += 3 + length;
                }
                0x02 => {
                    let length = read_u32(clx, at + 1) as usize;
                    let body = clx
                        .get(at + 5..at + 5 + length)
                        .ok_or(Error::Malformed("the piece table is cut short"))?;
                    return Ok(Pieces {
                        pieces: parse(body),
                    });
                }
                _ => return Err(Error::Malformed("the piece table has an unknown marker")),
            }
        }
        Err(Error::Malformed("the piece table is missing"))
    }

    /// The text of a range of character positions.
    ///
    /// Word's own special characters are turned into the ones the model uses:
    /// the paragraph mark stays as `\n`, a cell mark and a row mark stay as
    /// themselves for the table reader to find, and the field and note
    /// characters are kept so nothing shifts position.
    pub fn text(&self, stream: &[u8], from: u32, to: u32) -> String {
        let mut out = String::new();
        for piece in &self.pieces {
            if piece.end <= from || piece.start >= to {
                continue;
            }
            let start = piece.start.max(from);
            let end = piece.end.min(to);
            let skip = (start - piece.start) as usize;
            let count = (end - start) as usize;
            match piece.compressed {
                true => {
                    let at = piece.offset + skip;
                    for &byte in stream.get(at..at + count).unwrap_or_default() {
                        out.push(from_1252(byte));
                    }
                }
                false => {
                    let at = piece.offset + skip * 2;
                    let bytes = stream.get(at..at + count * 2).unwrap_or_default();
                    let units: Vec<u16> = bytes
                        .chunks_exact(2)
                        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                        .collect();
                    out.push_str(&String::from_utf16_lossy(&units));
                }
            }
        }
        out
    }

    /// Where a character position is in the `WordDocument` stream, and whether
    /// the byte there is one character or half of one.
    ///
    /// The bin tables index by byte offset rather than by character position, so
    /// going the other way is what makes a property exception findable.
    pub fn offset_of(&self, cp: u32) -> Option<(usize, bool)> {
        let piece = self
            .pieces
            .iter()
            .find(|piece| cp >= piece.start && cp < piece.end)?;
        let skip = (cp - piece.start) as usize;
        Some(match piece.compressed {
            true => (piece.offset + skip, true),
            false => (piece.offset + skip * 2, false),
        })
    }

    /// The character position a byte offset holds, going the other way.
    pub fn cp_of(&self, offset: usize) -> Option<u32> {
        for piece in &self.pieces {
            let width = if piece.compressed { 1 } else { 2 };
            let bytes = piece.len() as usize * width;
            if offset >= piece.offset && offset < piece.offset + bytes {
                return Some(piece.start + ((offset - piece.offset) / width) as u32);
            }
        }
        None
    }

    /// The last character position the table covers.
    pub fn last(&self) -> u32 {
        self.pieces.last().map(|piece| piece.end).unwrap_or(0)
    }
}

/// A `PlcPcd`: n+1 character positions, then n eight-byte descriptors.
fn parse(body: &[u8]) -> Vec<Piece> {
    // Each piece costs four bytes of position and eight of descriptor, and there
    // is one more position than there are pieces.
    if body.len() < 4 {
        return Vec::new();
    }
    let count = (body.len() - 4) / 12;
    let positions: Vec<u32> = (0..=count).map(|index| read_u32(body, index * 4)).collect();
    let base = (count + 1) * 4;
    (0..count)
        .map(|index| {
            let at = base + index * 8;
            let raw = read_u32(body, at + 2);
            // Bit 30 says one byte per character, and the address is doubled to
            // make room for the flag.
            let compressed = raw & 0x4000_0000 != 0;
            let offset = match compressed {
                true => (raw & 0x3FFF_FFFF) as usize / 2,
                false => raw as usize,
            };
            Piece {
                start: positions[index],
                end: positions[index + 1],
                offset,
                compressed,
                prm: read_u16(body, at + 6),
            }
        })
        .collect()
}

/// Windows-1252 to Unicode.
///
/// The 32 characters between 0x80 and 0x9F are the whole difference from
/// Latin-1, and they are exactly the ones an English document uses: the curly
/// quotes, the em dash and the ellipsis.
pub fn from_1252(byte: u8) -> char {
    const HIGH: [char; 32] = [
        '\u{20AC}', '\u{81}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{8D}',
        '\u{017D}', '\u{8F}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}',
        '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}',
        '\u{9D}', '\u{017E}', '\u{0178}',
    ];
    match byte {
        0x80..=0x9F => HIGH[(byte - 0x80) as usize],
        other => other as char,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(pieces: &[(u32, u32, u32)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (start, _, _) in pieces {
            out.extend_from_slice(&start.to_le_bytes());
        }
        out.extend_from_slice(&pieces.last().unwrap().1.to_le_bytes());
        for (_, _, raw) in pieces {
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&raw.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
        }
        out
    }

    #[test]
    fn bit_thirty_means_one_byte_per_character_and_halves_the_address() {
        // Read as UTF-16 throughout, an English document comes out as Chinese.
        let bytes = table(&[(0, 5, 0x4000_0000 | (0x0200 * 2))]);
        let pieces = parse(&bytes);
        assert_eq!(pieces.len(), 1);
        assert!(pieces[0].compressed);
        assert_eq!(pieces[0].offset, 0x0200);
    }

    #[test]
    fn an_uncompressed_piece_keeps_its_address_as_it_is() {
        let bytes = table(&[(0, 5, 0x0400)]);
        let pieces = parse(&bytes);
        assert!(!pieces[0].compressed);
        assert_eq!(pieces[0].offset, 0x0400);
    }

    #[test]
    fn the_text_of_a_range_comes_from_whichever_pieces_hold_it() {
        // The point of the whole exercise: characters that are next to each
        // other in the document are not next to each other in the file.
        let mut stream = vec![0u8; 64];
        stream[16..21].copy_from_slice(b"first");
        stream[40..46].copy_from_slice(b"second");
        let pieces = Pieces {
            pieces: vec![
                Piece {
                    start: 0,
                    end: 5,
                    offset: 16,
                    compressed: true,
                    prm: 0,
                },
                Piece {
                    start: 5,
                    end: 11,
                    offset: 40,
                    compressed: true,
                    prm: 0,
                },
            ],
        };
        assert_eq!(pieces.text(&stream, 0, 11), "firstsecond");
        assert_eq!(pieces.text(&stream, 5, 11), "second");
        assert_eq!(pieces.text(&stream, 3, 7), "stse");
    }

    #[test]
    fn a_byte_offset_and_a_character_position_convert_both_ways() {
        let pieces = Pieces {
            pieces: vec![Piece {
                start: 100,
                end: 110,
                offset: 0x200,
                compressed: false,
                prm: 0,
            }],
        };
        assert_eq!(pieces.offset_of(105), Some((0x200 + 10, false)));
        assert_eq!(pieces.cp_of(0x200 + 10), Some(105));
    }

    #[test]
    fn the_windows_1252_gap_is_the_punctuation_an_english_document_uses() {
        assert_eq!(from_1252(0x93), '\u{201C}');
        assert_eq!(from_1252(0x97), '—');
        assert_eq!(from_1252(b'A'), 'A');
    }
}
