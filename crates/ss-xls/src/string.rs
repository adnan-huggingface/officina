//! BIFF8 strings, and the shared string table they mostly live in.
//!
//! Every string carries a flag byte saying whether its characters are one byte
//! each (Latin-1, "compressed") or two (UTF-16). It may also be followed by
//! rich-text run data and by phonetic guide data, whose lengths are declared in
//! the same header and which have to be stepped over even when they are of no
//! interest.
//!
//! The trap is the shared string table. It is one logical run of strings split
//! across `CONTINUE` records, and a string may be cut in half by the split. The
//! second half then begins with **a fresh flag byte**, and the encoding is
//! allowed to change at that point: Excel writes the head of a string
//! compressed and its tail wide if a single accented character turns up late.
//! A reader that joins the `CONTINUE` payloads and parses the result as one
//! buffer gets every string after the first split wrong — and there is no
//! error, only mojibake several thousand strings in.

use crate::error::{Error, Result};
use crate::record::u16_at;

/// A string whose length is a `u16` count of characters: `XLUnicodeString`.
///
/// Returns the text and how many bytes it occupied.
pub(crate) fn long(data: &[u8], at: usize) -> Option<(String, usize)> {
    read(data, at, u16_at(data, at)? as usize, 2)
}

/// A string whose length is a single byte: `ShortXLUnicodeString`, which is
/// what sheet names and formula literals use.
pub(crate) fn short(data: &[u8], at: usize) -> Option<(String, usize)> {
    read(data, at, *data.get(at)? as usize, 1)
}

fn read(data: &[u8], at: usize, chars: usize, len_bytes: usize) -> Option<(String, usize)> {
    let flags = *data.get(at + len_bytes)?;
    let mut cursor = at + len_bytes + 1;

    // The two optional counts come *before* the characters, not after them.
    let runs = if flags & 0x08 != 0 {
        let n = u16_at(data, cursor)? as usize;
        cursor += 2;
        n
    } else {
        0
    };
    let phonetic = if flags & 0x04 != 0 {
        let n = crate::record::u32_at(data, cursor)? as usize;
        cursor += 4;
        n
    } else {
        0
    };

    let wide = flags & 0x01 != 0;
    let bytes = chars.checked_mul(if wide { 2 } else { 1 })?;
    let text = decode(data.get(cursor..cursor + bytes)?, wide);
    cursor += bytes + runs * 4 + phonetic;
    Some((text, cursor - at))
}

/// Compressed characters are Latin-1, not ASCII and not UTF-8: every byte is
/// the code point of the same number.
fn decode(bytes: &[u8], wide: bool) -> String {
    if wide {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        bytes.iter().map(|b| *b as char).collect()
    }
}

/// The shared string table, read across the `CONTINUE` boundaries it is
/// deliberately split on.
pub(crate) fn sst(parts: &[&[u8]]) -> Result<Vec<String>> {
    let mut cursor = Cursor::new(parts);
    // cstTotal — how many cells point into the table — is of no use to a reader
    // and is skipped. cstUnique is the number of strings.
    cursor.skip(4).ok_or(Error::Truncated("SST"))?;
    let unique = cursor.u32().ok_or(Error::Truncated("SST"))? as usize;

    let mut out = Vec::with_capacity(unique.min(1 << 16));
    for _ in 0..unique {
        match cursor.string() {
            Some(text) => out.push(text),
            // A table that ends early is kept as far as it got. The strings
            // already read are correct, and the cells pointing past them fall
            // back to blank — much better than refusing the workbook.
            None => break,
        }
    }
    Ok(out)
}

/// A position in a list of `CONTINUE` payloads, read as one stream.
struct Cursor<'a> {
    parts: &'a [&'a [u8]],
    part: usize,
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(parts: &'a [&'a [u8]]) -> Cursor<'a> {
        Cursor {
            parts,
            part: 0,
            at: 0,
        }
    }

    fn left(&self) -> usize {
        self.parts
            .get(self.part)
            .map_or(0, |p| p.len().saturating_sub(self.at))
    }

    /// Move to the next payload. Fixed-size fields are never split across one,
    /// so this is only ever called at a boundary.
    fn next_part(&mut self) -> Option<()> {
        self.part += 1;
        self.at = 0;
        (self.part < self.parts.len()).then_some(())
    }

    fn byte(&mut self) -> Option<u8> {
        let b = *self.parts.get(self.part)?.get(self.at)?;
        self.at += 1;
        Some(b)
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes([self.byte()?, self.byte()?]))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes([
            self.byte()?,
            self.byte()?,
            self.byte()?,
            self.byte()?,
        ]))
    }

    /// Step over bytes that carry no text. Unlike character data, these do not
    /// get a fresh flag byte when they cross into the next payload.
    fn skip(&mut self, mut count: usize) -> Option<()> {
        while count > 0 {
            if self.left() == 0 {
                self.next_part()?;
                continue;
            }
            let step = count.min(self.left());
            self.at += step;
            count -= step;
        }
        Some(())
    }

    fn string(&mut self) -> Option<String> {
        if self.left() == 0 {
            self.next_part()?;
        }
        let chars = self.u16()? as usize;
        let flags = self.byte()?;
        let runs = if flags & 0x08 != 0 {
            self.u16()? as usize
        } else {
            0
        };
        let phonetic = if flags & 0x04 != 0 {
            self.u32()? as usize
        } else {
            0
        };

        let mut wide = flags & 0x01 != 0;
        let mut left = chars;
        let mut text = String::with_capacity(chars);
        while left > 0 {
            if self.left() == 0 {
                self.next_part()?;
                // The flag byte that starts a continued string. Only the low
                // bit is meaningful here, and it may disagree with the one the
                // string started with.
                wide = self.byte()? & 0x01 != 0;
                continue;
            }
            let unit = if wide { 2 } else { 1 };
            let take = left.min(self.left() / unit);
            if take == 0 {
                // A single trailing byte with wide characters still to come.
                // Not something Excel writes; stepping to the next payload is
                // the only reading that does not invent a character.
                self.next_part()?;
                wide = self.byte()? & 0x01 != 0;
                continue;
            }
            let bytes = &self.parts[self.part][self.at..self.at + take * unit];
            text.push_str(&decode(bytes, wide));
            self.at += take * unit;
            left -= take;
        }

        self.skip(runs * 4 + phonetic)?;
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lay out one SST string the way the format does.
    fn entry(text: &str, wide: bool) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(text.chars().count() as u16).to_le_bytes());
        out.push(if wide { 1 } else { 0 });
        if wide {
            for unit in text.encode_utf16() {
                out.extend_from_slice(&unit.to_le_bytes());
            }
        } else {
            out.extend(text.chars().map(|c| c as u8));
        }
        out
    }

    fn header(unique: usize) -> Vec<u8> {
        let mut out = (unique as u32).to_le_bytes().to_vec();
        out.extend_from_slice(&(unique as u32).to_le_bytes());
        out
    }

    #[test]
    fn compressed_characters_are_latin_one_rather_than_ascii() {
        // 0xE9 is e-acute in Latin-1 and not valid UTF-8 at all.
        let mut part = header(1);
        part.extend_from_slice(&[1, 0, 0, 0xE9]);
        let table = sst(&[&part]).expect("reads");
        assert_eq!(table, vec!["é"]);
    }

    #[test]
    fn a_table_of_several_strings_keeps_their_order() {
        let mut part = header(3);
        part.extend(entry("one", false));
        part.extend(entry("two", true));
        part.extend(entry("", false));
        let table = sst(&[&part]).expect("reads");
        assert_eq!(table, vec!["one", "two", ""]);
    }

    #[test]
    fn a_string_cut_by_a_continue_keeps_its_second_half() {
        // "abcdef" split after "abc". The continuation opens with its own flag
        // byte, which is the whole difficulty of this record.
        let mut first = header(1);
        first.extend_from_slice(&[6, 0, 0]);
        first.extend_from_slice(b"abc");
        let mut second = vec![0u8]; // still compressed
        second.extend_from_slice(b"def");
        let table = sst(&[&first, &second]).expect("reads");
        assert_eq!(table, vec!["abcdef"]);
    }

    #[test]
    fn a_string_may_change_encoding_where_it_was_cut() {
        // The head is one byte per character and the tail is two. Joining the
        // payloads and parsing once reads the tail as Latin-1 and produces six
        // characters of noise instead of three.
        let mut first = header(1);
        first.extend_from_slice(&[6, 0, 0]);
        first.extend_from_slice(b"abc");
        let mut second = vec![1u8]; // wide from here on
        for unit in "déf".encode_utf16() {
            second.extend_from_slice(&unit.to_le_bytes());
        }
        let table = sst(&[&first, &second]).expect("reads");
        assert_eq!(table, vec!["abcdéf"]);
    }

    #[test]
    fn a_string_that_ends_exactly_at_a_boundary_does_not_eat_a_flag_byte() {
        // The next payload starts a new string, so its first byte is a length,
        // not an encoding flag. Consuming it here loses a character from every
        // string that follows.
        let mut first = header(2);
        first.extend(entry("abc", false));
        let second = entry("def", false);
        let table = sst(&[&first, &second]).expect("reads");
        assert_eq!(table, vec!["abc", "def"]);
    }

    #[test]
    fn rich_text_runs_are_stepped_over_rather_than_read() {
        let mut part = header(2);
        // "hi" with two formatting runs, then an ordinary string after them.
        part.extend_from_slice(&[2, 0, 0x08, 2, 0]);
        part.extend_from_slice(b"hi");
        part.extend_from_slice(&[0, 0, 5, 0, 1, 0, 6, 0]);
        part.extend(entry("next", false));
        let table = sst(&[&part]).expect("reads");
        assert_eq!(table, vec!["hi", "next"]);
    }

    #[test]
    fn phonetic_data_is_stepped_over_too() {
        let mut part = header(2);
        part.extend_from_slice(&[2, 0, 0x04, 3, 0, 0, 0]);
        part.extend_from_slice(b"hi");
        part.extend_from_slice(&[0xAA, 0xBB, 0xCC]);
        part.extend(entry("next", false));
        let table = sst(&[&part]).expect("reads");
        assert_eq!(table, vec!["hi", "next"]);
    }

    #[test]
    fn a_table_that_stops_early_keeps_what_it_had() {
        let mut part = header(4);
        part.extend(entry("only", false));
        let table = sst(&[&part]).expect("reads");
        assert_eq!(table, vec!["only"]);
    }

    #[test]
    fn a_short_string_reports_the_bytes_it_used() {
        let mut data = vec![0u8; 3];
        data.extend_from_slice(&[3, 0]);
        data.extend_from_slice(b"abc");
        let (text, used) = short(&data, 3).expect("reads");
        assert_eq!(text, "abc");
        assert_eq!(used, 5, "one length byte, one flag byte, three characters");
    }
}
