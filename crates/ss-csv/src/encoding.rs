//! Working out what bytes a text file is made of.
//!
//! There is no reliable way to do this and it still has to be done, because
//! guessing wrong is not a small error. A Windows-1252 export read as UTF-8
//! either fails outright or drops every accented character; a UTF-16 file read
//! as UTF-8 comes back as text separated by NULs.
//!
//! The order here is: believe a byte-order mark, then try UTF-8, then look for
//! the NUL pattern of unmarked UTF-16, and fall back to Windows-1252 — which is
//! the only choice that cannot fail, since every byte has a meaning in it.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    /// UTF-8 with a byte-order mark. Excel writes one; keeping the distinction
    /// means a file that had one gets one back.
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    /// The Windows ANSI code page, and the fallback for anything unrecognized.
    /// Every byte maps to a character, so decoding can never fail.
    Windows1252,
}

impl Encoding {
    /// The bytes a writer should put at the front of the file.
    pub fn bom(self) -> &'static [u8] {
        match self {
            Encoding::Utf8Bom => &[0xEF, 0xBB, 0xBF],
            Encoding::Utf16Le => &[0xFF, 0xFE],
            Encoding::Utf16Be => &[0xFE, 0xFF],
            _ => &[],
        }
    }

    /// Decodes, replacing anything undecodable rather than failing.
    ///
    /// A text importer that refuses a file over one bad byte is worse than one
    /// that shows the rest: the user can see what happened and fix it, which
    /// they cannot do with an error message.
    pub fn decode(self, bytes: &[u8]) -> String {
        let bytes = strip_bom(bytes, self);
        match self {
            Encoding::Utf8 | Encoding::Utf8Bom => String::from_utf8_lossy(bytes).into_owned(),
            Encoding::Utf16Le | Encoding::Utf16Be => {
                let units: Vec<u16> = bytes
                    .chunks_exact(2)
                    .map(|pair| {
                        let pair = [pair[0], pair[1]];
                        if self == Encoding::Utf16Le {
                            u16::from_le_bytes(pair)
                        } else {
                            u16::from_be_bytes(pair)
                        }
                    })
                    .collect();
                String::from_utf16_lossy(&units)
            }
            Encoding::Windows1252 => bytes.iter().map(|b| from_1252(*b)).collect(),
        }
    }

    /// Encodes, for a writer. Anything a code page cannot hold becomes `?`,
    /// which is what every Windows application does in the same position.
    pub fn encode(self, text: &str) -> Vec<u8> {
        let mut out = self.bom().to_vec();
        match self {
            Encoding::Utf8 | Encoding::Utf8Bom => out.extend_from_slice(text.as_bytes()),
            Encoding::Utf16Le => {
                out.extend(text.encode_utf16().flat_map(|u| u.to_le_bytes()));
            }
            Encoding::Utf16Be => {
                out.extend(text.encode_utf16().flat_map(|u| u.to_be_bytes()));
            }
            Encoding::Windows1252 => {
                out.extend(text.chars().map(|c| to_1252(c).unwrap_or(b'?')));
            }
        }
        out
    }
}

fn strip_bom(bytes: &[u8], encoding: Encoding) -> &[u8] {
    let bom = encoding.bom();
    match bytes.strip_prefix(bom) {
        Some(rest) if !bom.is_empty() => rest,
        _ => bytes,
    }
}

/// Guesses from a sample of the file's start.
pub fn detect(sample: &[u8]) -> Encoding {
    if sample.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Encoding::Utf8Bom;
    }
    if sample.starts_with(&[0xFF, 0xFE]) {
        return Encoding::Utf16Le;
    }
    if sample.starts_with(&[0xFE, 0xFF]) {
        return Encoding::Utf16Be;
    }

    // Unmarked UTF-16 gives itself away: ASCII text in it is every other byte
    // zero, and which side the zeros fall on says which endianness.
    let (even_nulls, odd_nulls) =
        sample
            .iter()
            .take(512)
            .enumerate()
            .fold((0usize, 0usize), |(even, odd), (i, b)| {
                match (*b == 0, i % 2 == 0) {
                    (true, true) => (even + 1, odd),
                    (true, false) => (even, odd + 1),
                    _ => (even, odd),
                }
            });
    let looked_at = sample.len().min(512);
    if looked_at >= 8 {
        if odd_nulls * 4 > looked_at && even_nulls == 0 {
            return Encoding::Utf16Le;
        }
        if even_nulls * 4 > looked_at && odd_nulls == 0 {
            return Encoding::Utf16Be;
        }
    }

    // A truncated sample can cut a multi-byte character in half, so a failure
    // in the last three bytes is not evidence of anything.
    match std::str::from_utf8(sample) {
        Ok(_) => Encoding::Utf8,
        Err(e) if e.error_len().is_none() && e.valid_up_to() + 4 >= sample.len() => Encoding::Utf8,
        Err(_) => Encoding::Windows1252,
    }
}

/// The 27 places Windows-1252 differs from Latin-1. Everything else is the code
/// point with the same number, which is what makes the fallback total.
const HIGH: [char; 32] = [
    '\u{20AC}', '\u{81}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{8D}', '\u{017D}', '\u{8F}',
    '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{9D}', '\u{017E}', '\u{0178}',
];

fn from_1252(byte: u8) -> char {
    match byte {
        0x80..=0x9F => HIGH[(byte - 0x80) as usize],
        other => other as char,
    }
}

fn to_1252(c: char) -> Option<u8> {
    if let Some(index) = HIGH.iter().position(|h| *h == c) {
        return Some(0x80 + index as u8);
    }
    match c as u32 {
        // The 0x80-0x9F range is spoken for by the table above.
        code @ 0..=0x7F => Some(code as u8),
        code @ 0xA0..=0xFF => Some(code as u8),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_byte_order_mark_is_believed() {
        assert_eq!(detect(b"\xEF\xBB\xBFa,b"), Encoding::Utf8Bom);
        assert_eq!(detect(b"\xFF\xFEa\0"), Encoding::Utf16Le);
        assert_eq!(detect(b"\xFE\xFF\0a"), Encoding::Utf16Be);
    }

    #[test]
    fn the_mark_is_not_part_of_the_text() {
        assert_eq!(Encoding::Utf8Bom.decode(b"\xEF\xBB\xBFName"), "Name");
    }

    #[test]
    fn unmarked_utf16_is_found_by_where_its_nulls_fall() {
        let le: Vec<u8> = "Name,Value\n"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        let be: Vec<u8> = "Name,Value\n"
            .encode_utf16()
            .flat_map(|u| u.to_be_bytes())
            .collect();
        assert_eq!(detect(&le), Encoding::Utf16Le);
        assert_eq!(detect(&be), Encoding::Utf16Be);
        assert_eq!(Encoding::Utf16Le.decode(&le), "Name,Value\n");
        assert_eq!(Encoding::Utf16Be.decode(&be), "Name,Value\n");
    }

    #[test]
    fn a_windows_export_is_not_mistaken_for_broken_utf8() {
        // `caf\xE9` is "café" in Windows-1252 and invalid UTF-8. Read as UTF-8
        // the accented character is lost; read as 1252 it is right.
        let bytes = b"caf\xE9,42\n";
        assert_eq!(detect(bytes), Encoding::Windows1252);
        assert_eq!(Encoding::Windows1252.decode(bytes), "café,42\n");
    }

    #[test]
    fn the_twenty_seven_places_1252_is_not_latin_1() {
        // 0x93 and 0x94 are smart quotes, not control characters. Getting this
        // wrong is how "curly quotes" become invisible glyphs.
        assert_eq!(Encoding::Windows1252.decode(b"\x93hi\x94"), "“hi”");
        assert_eq!(Encoding::Windows1252.decode(b"\x80"), "€");
    }

    #[test]
    fn a_sample_cut_mid_character_is_still_utf8() {
        // The sniffer only ever sees the first few kilobytes, and a multi-byte
        // character straddling the cut is not evidence of a different encoding.
        let mut text = "é".repeat(300).into_bytes();
        text.truncate(text.len() - 1);
        assert_eq!(detect(&text), Encoding::Utf8);
    }

    #[test]
    fn what_is_encoded_decodes_back() {
        for encoding in [
            Encoding::Utf8,
            Encoding::Utf8Bom,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
            Encoding::Windows1252,
        ] {
            let text = "Name,café,“quoted”\n";
            let bytes = encoding.encode(text);
            assert_eq!(encoding.decode(&bytes), text, "{encoding:?}");
        }
    }

    #[test]
    fn a_character_a_code_page_cannot_hold_becomes_a_question_mark() {
        // Rather than failing the save. Every Windows application does this,
        // and losing one glyph beats losing the file.
        assert_eq!(Encoding::Windows1252.encode("漢"), b"?");
    }
}
