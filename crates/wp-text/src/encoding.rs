//! Deciding what bytes a text file is written in.
//!
//! The same problem `ss-csv` solved for spreadsheets, and the same answer, for
//! the same reason: **the fallback must be an encoding that cannot fail.** A
//! reader that gives up on a byte it does not understand refuses a file the user
//! can plainly read in Notepad, and "this file is not text" is never a helpful
//! thing to say about a text file.
//!
//! The order is: a byte-order mark, then unmarked UTF-16 by where its NULs fall,
//! then UTF-8, then Windows-1252 — which maps all 256 bytes and therefore always
//! succeeds.

/// What a file turned out to be written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    /// UTF-8 with a byte-order mark, which Notepad writes and which has to be
    /// written back or the file changes for no reason.
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    /// The fallback. Every byte maps to something, so this never fails.
    Windows1252,
}

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";
const UTF16LE_BOM: &[u8] = b"\xFF\xFE";
const UTF16BE_BOM: &[u8] = b"\xFE\xFF";

/// Which encoding `bytes` is in, and where the text starts.
pub fn detect(bytes: &[u8]) -> (Encoding, usize) {
    if bytes.starts_with(UTF8_BOM) {
        return (Encoding::Utf8Bom, UTF8_BOM.len());
    }
    // The UTF-16 marks are checked before UTF-8's because `FF FE` is also the
    // start of a valid — if unlikely — Windows-1252 file, and a mark is a
    // statement rather than a guess.
    if bytes.starts_with(UTF16LE_BOM) {
        return (Encoding::Utf16Le, UTF16LE_BOM.len());
    }
    if bytes.starts_with(UTF16BE_BOM) {
        return (Encoding::Utf16Be, UTF16BE_BOM.len());
    }
    if let Some(guess) = unmarked_utf16(bytes) {
        return (guess, 0);
    }
    if std::str::from_utf8(bytes).is_ok() {
        return (Encoding::Utf8, 0);
    }
    (Encoding::Windows1252, 0)
}

/// UTF-16 with no mark, guessed from where the zero bytes fall.
///
/// English text in UTF-16LE is `T\0e\0x\0t\0`: every second byte is zero, and
/// which second byte says which way round it is. Nothing else produces that
/// pattern, and a file with no zeroes at all is not UTF-16 of any kind.
fn unmarked_utf16(bytes: &[u8]) -> Option<Encoding> {
    let sample = &bytes[..bytes.len().min(512)];
    if sample.len() < 4 {
        return None;
    }
    let pairs = sample.len() / 2;
    let even_zero = (0..pairs).filter(|i| sample[i * 2] == 0).count();
    let odd_zero = (0..pairs).filter(|i| sample[i * 2 + 1] == 0).count();
    let most = pairs * 3 / 4;
    if odd_zero >= most && even_zero < most {
        return Some(Encoding::Utf16Le);
    }
    if even_zero >= most && odd_zero < most {
        return Some(Encoding::Utf16Be);
    }
    None
}

/// Decodes `bytes`, and says what it decided.
pub fn decode(bytes: &[u8]) -> (String, Encoding) {
    let (encoding, skip) = detect(bytes);
    let body = &bytes[skip.min(bytes.len())..];
    let text = match encoding {
        Encoding::Utf8 | Encoding::Utf8Bom => String::from_utf8_lossy(body).into_owned(),
        Encoding::Utf16Le => utf16(body, true),
        Encoding::Utf16Be => utf16(body, false),
        Encoding::Windows1252 => body.iter().map(|&b| cp1252(b)).collect(),
    };
    (text, encoding)
}

fn utf16(bytes: &[u8], little: bool) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| {
            if little {
                u16::from_le_bytes([pair[0], pair[1]])
            } else {
                u16::from_be_bytes([pair[0], pair[1]])
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}

/// Windows-1252's eight-bit half. The other 224 code points are Latin-1's, and
/// the thirty-two that differ are the ones in `0x80..=0x9F` — where Latin-1 has
/// control characters and 1252 has the curly quotes and the em dash that a
/// Windows text file is full of.
fn cp1252(byte: u8) -> char {
    const HIGH: [char; 32] = [
        '\u{20AC}', '\u{FFFD}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{FFFD}',
        '\u{017D}', '\u{FFFD}', '\u{FFFD}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}',
        '\u{2022}', '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}',
        '\u{0153}', '\u{FFFD}', '\u{017E}', '\u{0178}',
    ];
    match byte {
        0x80..=0x9F => HIGH[(byte - 0x80) as usize],
        other => other as char,
    }
}

/// Encodes text back, with the mark the file came in with.
pub fn encode(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf8Bom => {
            let mut out = UTF8_BOM.to_vec();
            out.extend_from_slice(text.as_bytes());
            out
        }
        Encoding::Utf16Le => {
            let mut out = UTF16LE_BOM.to_vec();
            out.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
            out
        }
        Encoding::Utf16Be => {
            let mut out = UTF16BE_BOM.to_vec();
            out.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
            out
        }
        // Anything 1252 cannot spell is written as a question mark, which is
        // what every editor on Windows does. A save that silently dropped it
        // would be worse; a save that refused would be worse still.
        Encoding::Windows1252 => text.chars().map(to_cp1252).collect(),
    }
}

fn to_cp1252(c: char) -> u8 {
    if (c as u32) < 0x80 || ((c as u32) >= 0xA0 && (c as u32) <= 0xFF) {
        return c as u8;
    }
    for byte in 0x80u8..=0x9F {
        if cp1252(byte) == c {
            return byte;
        }
    }
    b'?'
}

/// Which line ending a file uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    /// `\r\n`. What Notepad writes, and what a file that came from Windows has.
    Crlf,
    Lf,
    /// `\r` alone — classic Mac. Rare, and a file that has it is unreadable in
    /// an editor that assumes the other two.
    Cr,
}

impl LineEnding {
    pub const fn as_str(self) -> &'static str {
        match self {
            LineEnding::Crlf => "\r\n",
            LineEnding::Lf => "\n",
            LineEnding::Cr => "\r",
        }
    }
}

/// The line ending a file uses, from the first one in it.
///
/// The *first*, not the most common: a file with mixed endings is a file that
/// has been edited by two programs, and following the first one keeps the diff
/// to what the user actually changed.
pub fn line_ending(text: &str) -> LineEnding {
    let bytes = text.as_bytes();
    for (index, &byte) in bytes.iter().enumerate() {
        if byte == b'\r' {
            return if bytes.get(index + 1) == Some(&b'\n') {
                LineEnding::Crlf
            } else {
                LineEnding::Cr
            };
        }
        if byte == b'\n' {
            return LineEnding::Lf;
        }
    }
    // A file with no line ending at all: on Windows, what the user expects.
    LineEnding::Crlf
}

/// Splits into lines, whatever the endings are.
pub fn lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' => {
                out.push(&text[start..index]);
                index += if bytes.get(index + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                start = index;
            }
            b'\n' => {
                out.push(&text[start..index]);
                index += 1;
                start = index;
            }
            _ => index += 1,
        }
    }
    // A trailing newline ends the last line rather than starting an empty one:
    // a file of three lines that ends in a newline is three paragraphs, and
    // opening it in Word gives four only if the reader is wrong.
    if start < bytes.len() {
        out.push(&text[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_byte_order_mark_is_a_statement_rather_than_a_guess() {
        assert_eq!(detect(b"\xEF\xBB\xBFhi"), (Encoding::Utf8Bom, 3));
        assert_eq!(detect(b"\xFF\xFEh\0i\0"), (Encoding::Utf16Le, 2));
        assert_eq!(detect(b"\xFE\xFF\0h\0i"), (Encoding::Utf16Be, 2));
    }

    #[test]
    fn unmarked_utf16_is_found_by_where_its_zeroes_fall() {
        let (text, encoding) = decode(b"H\0e\0l\0l\0o\0");
        assert_eq!(encoding, Encoding::Utf16Le);
        assert_eq!(text, "Hello");

        let (text, encoding) = decode(b"\0H\0e\0l\0l\0o");
        assert_eq!(encoding, Encoding::Utf16Be);
        assert_eq!(text, "Hello");
    }

    #[test]
    fn ordinary_utf8_is_read_as_utf8() {
        let (text, encoding) = decode("café — naïve".as_bytes());
        assert_eq!(encoding, Encoding::Utf8);
        assert_eq!(text, "café — naïve");
    }

    #[test]
    fn the_fallback_cannot_fail() {
        // A reader that gives up refuses a file the user can plainly read.
        // 0x93 and 0x94 are the curly quotes a Windows text file is full of,
        // and they are not valid UTF-8.
        let (text, encoding) = decode(b"He said \x93hello\x94 \x97 and left");
        assert_eq!(encoding, Encoding::Windows1252);
        assert_eq!(text, "He said \u{201C}hello\u{201D} \u{2014} and left");
    }

    #[test]
    fn a_file_comes_back_in_the_encoding_it_went_in_as() {
        for encoding in [
            Encoding::Utf8,
            Encoding::Utf8Bom,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            let text = "Simple ASCII text.";
            let bytes = encode(text, encoding);
            let (back, detected) = decode(&bytes);
            assert_eq!(back, text, "{encoding:?} did not round trip");
            assert_eq!(detected, encoding, "{encoding:?} was not recognised again");
        }
    }

    #[test]
    fn pure_ascii_is_read_as_utf8_whatever_it_was_written_as() {
        // Not a fault: ASCII *is* valid UTF-8, and nothing in the bytes can say
        // otherwise. It matters because a Windows-1252 file with no high bytes
        // comes back as UTF-8 and is byte-identical either way.
        let bytes = encode("plain", Encoding::Windows1252);
        assert_eq!(decode(&bytes), ("plain".to_owned(), Encoding::Utf8));
        assert_eq!(bytes, b"plain");
    }

    #[test]
    fn a_windows_1252_file_with_high_bytes_round_trips_as_itself() {
        let text = "He said \u{201C}hello\u{201D}";
        let bytes = encode(text, Encoding::Windows1252);
        let (back, detected) = decode(&bytes);
        assert_eq!(detected, Encoding::Windows1252);
        assert_eq!(back, text);
    }

    #[test]
    fn a_character_windows_1252_cannot_spell_becomes_a_question_mark() {
        // Silently dropping it would be worse, and refusing to save would be
        // worse still.
        assert_eq!(encode("a\u{4E2D}b", Encoding::Windows1252), b"a?b");
        assert_eq!(encode("\u{2014}", Encoding::Windows1252), b"\x97");
    }

    #[test]
    fn the_first_line_ending_is_the_files_line_ending() {
        assert_eq!(line_ending("one\r\ntwo"), LineEnding::Crlf);
        assert_eq!(line_ending("one\ntwo"), LineEnding::Lf);
        assert_eq!(line_ending("one\rtwo"), LineEnding::Cr);
        // Mixed: the first wins, so the diff is what the user changed.
        assert_eq!(line_ending("one\r\ntwo\nthree"), LineEnding::Crlf);
        assert_eq!(line_ending("no endings"), LineEnding::Crlf);
    }

    #[test]
    fn a_trailing_newline_ends_the_last_line_rather_than_starting_one() {
        assert_eq!(lines("a\nb\nc\n"), ["a", "b", "c"]);
        assert_eq!(lines("a\r\nb\r\n"), ["a", "b"]);
        assert_eq!(lines("a\n\nb"), ["a", "", "b"], "a blank line is a line");
        assert_eq!(lines(""), Vec::<&str>::new());
        assert_eq!(lines("one"), ["one"]);
    }
}
