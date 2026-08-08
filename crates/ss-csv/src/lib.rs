//! CSV and TSV: sniffing what a file actually is, and reading it without
//! holding it all in memory.
//!
//! "CSV" is not a format. It is a family of conventions that disagree about the
//! separator, the quote character, the line ending, the encoding, and whether
//! `""` inside a quoted field is one quote or two. A reader that assumes commas
//! and UTF-8 opens perhaps half the files a user will hand it, and — worse —
//! opens the other half *wrongly* rather than failing: a semicolon file read as
//! comma-separated becomes one enormous column, and a Windows-1252 file read as
//! UTF-8 either errors or silently loses every accented character.
//!
//! So the entry point is [`sniff`], and everything else takes the [`Dialect`]
//! it returns.
//!
//! **Nothing here holds a whole file.** [`Reader`] pulls records off a
//! `BufRead`, reusing one buffer per record, so a hundred-megabyte export costs
//! the size of its longest row rather than of the file.

#![forbid(unsafe_code)]

use std::io::{self, BufRead, Write};

pub mod encoding;
pub mod sheet;

pub use encoding::Encoding;
pub use sheet::{read_into, write_sheet, Imported};

/// What a particular file's conventions turn out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dialect {
    pub delimiter: u8,
    pub quote: u8,
    /// `""` inside a quoted field means one quote (RFC 4180), rather than a
    /// backslash escape. Both exist in the wild; the doubled quote is what
    /// Excel writes and reads.
    pub doubled_quotes: bool,
    /// What a writer should emit. A reader accepts all three regardless.
    pub newline: Newline,
}

impl Default for Dialect {
    fn default() -> Self {
        Dialect {
            delimiter: b',',
            quote: b'"',
            doubled_quotes: true,
            newline: Newline::Crlf,
        }
    }
}

impl Dialect {
    pub fn tsv() -> Dialect {
        Dialect {
            delimiter: b'\t',
            ..Dialect::default()
        }
    }

    /// The dialect a file extension implies, before the contents are looked at.
    pub fn for_extension(extension: &str) -> Dialect {
        match extension.to_ascii_lowercase().as_str() {
            "tsv" | "tab" => Dialect::tsv(),
            _ => Dialect::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Newline {
    Lf,
    Crlf,
}

impl Newline {
    pub fn as_str(self) -> &'static str {
        match self {
            Newline::Lf => "\n",
            Newline::Crlf => "\r\n",
        }
    }
}

/// The delimiters worth considering, in the order ties are broken.
///
/// Tab before semicolon before pipe, and the comma last of the four despite
/// being the most common: a comma appears inside quoted prose constantly, so a
/// file that scores equally on tab and comma is almost certainly tab-separated
/// prose rather than comma-separated data.
const CANDIDATES: [u8; 4] = *b"\t;|,";

/// Works out a file's encoding and dialect from its first few kilobytes.
///
/// The delimiter is chosen by *consistency*, not by frequency. A file of
/// English sentences separated by semicolons contains far more commas than
/// semicolons, but the number of commas per line varies wildly while the number
/// of semicolons is the same on every line. Frequency picks the comma and
/// produces garbage; consistency picks the semicolon.
pub fn sniff(sample: &[u8]) -> (Encoding, Dialect) {
    let encoding = encoding::detect(sample);
    let text = encoding.decode(sample);
    (encoding, sniff_dialect(&text))
}

/// The dialect alone, for text that has already been decoded.
pub fn sniff_dialect(text: &str) -> Dialect {
    // Excel writes this line to say what it used, and reads it back. Honouring
    // it beats any amount of guessing.
    if let Some(rest) = text.strip_prefix("sep=") {
        if let Some(delimiter) = rest.chars().next().filter(|c| c.is_ascii()) {
            return Dialect {
                delimiter: delimiter as u8,
                ..Dialect::default()
            };
        }
    }

    let lines = sample_lines(text, 32);
    if lines.is_empty() {
        return Dialect::default();
    }

    let mut best = (0usize, Dialect::default().delimiter, 0usize);
    for candidate in CANDIDATES {
        let counts: Vec<usize> = lines
            .iter()
            .map(|line| count_outside_quotes(line, candidate, b'"'))
            .collect();
        // The most common non-zero count, and how many lines agree on it.
        let Some(modal) = counts
            .iter()
            .copied()
            .filter(|c| *c > 0)
            .max_by_key(|target| counts.iter().filter(|c| *c == target).count())
        else {
            continue;
        };
        let agreeing = counts.iter().filter(|c| **c == modal).count();
        // A candidate has to appear on essentially every line to win. One stray
        // pipe in a comment line should not turn a CSV into a PSV.
        if agreeing * 4 < lines.len() * 3 {
            continue;
        }
        if agreeing > best.0 || (agreeing == best.0 && modal > best.2) {
            best = (agreeing, candidate, modal);
        }
    }

    Dialect {
        delimiter: if best.0 == 0 {
            Dialect::default().delimiter
        } else {
            best.1
        },
        newline: if text.contains("\r\n") {
            Newline::Crlf
        } else {
            Newline::Lf
        },
        ..Dialect::default()
    }
}

/// Up to `limit` lines, ignoring blank ones and never splitting inside quotes.
fn sample_lines(text: &str, limit: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut quoted = false;
    for (index, byte) in text.bytes().enumerate() {
        match byte {
            b'"' => quoted = !quoted,
            b'\n' if !quoted => {
                let line = text[start..index].trim_end_matches('\r');
                if !line.trim().is_empty() {
                    out.push(line);
                }
                start = index + 1;
                if out.len() >= limit {
                    return out;
                }
            }
            _ => {}
        }
    }
    let last = text[start..].trim_end_matches('\r');
    if !last.trim().is_empty() && out.len() < limit {
        out.push(last);
    }
    out
}

/// Counts a byte outside quoted regions.
///
/// This is the whole difference between sniffing that works and sniffing that
/// does not: `"Smith, John";42` has one semicolon and one comma, and only the
/// semicolon separates anything.
fn count_outside_quotes(line: &str, wanted: u8, quote: u8) -> usize {
    let mut count = 0;
    let mut quoted = false;
    for byte in line.bytes() {
        if byte == quote {
            quoted = !quoted;
        } else if byte == wanted && !quoted {
            count += 1;
        }
    }
    count
}

/// A streaming record reader.
///
/// One record per call, into a buffer the reader owns, so memory is the size of
/// the longest row rather than of the file.
pub struct Reader<R: BufRead> {
    source: R,
    dialect: Dialect,
    /// The characters of the current record, with separators removed.
    text: String,
    /// Where each field starts and ends within `text`.
    bounds: Vec<(usize, usize)>,
    line: Vec<u8>,
    decoder: Encoding,
    /// Whether the `sep=` line has been consumed.
    started: bool,
    finished: bool,
}

impl<R: BufRead> Reader<R> {
    pub fn new(source: R, encoding: Encoding, dialect: Dialect) -> Self {
        Reader {
            source,
            dialect,
            text: String::new(),
            bounds: Vec::new(),
            line: Vec::new(),
            decoder: encoding,
            started: false,
            finished: false,
        }
    }

    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// Reads the next record, or `Ok(false)` at the end of the file.
    ///
    /// A record is not a line: a quoted field may contain newlines, and this
    /// keeps reading lines until the quotes balance.
    pub fn next_record(&mut self) -> io::Result<bool> {
        self.text.clear();
        self.bounds.clear();
        let mut raw = String::new();
        let mut quoted = false;

        loop {
            self.line.clear();
            let read = read_line(&mut self.source, &mut self.line, self.decoder)?;
            if read == 0 {
                if raw.is_empty() {
                    self.finished = true;
                    return Ok(false);
                }
                break;
            }
            let chunk = self.decoder.decode(&self.line);
            let chunk = chunk.trim_end_matches('\n').trim_end_matches('\r');

            if !self.started {
                self.started = true;
                // Excel's own dialect declaration, consumed rather than read as
                // a one-cell row.
                if let Some(rest) = chunk.strip_prefix("sep=") {
                    if rest.chars().count() == 1 {
                        continue;
                    }
                }
            }

            quoted = toggles_quotes(chunk, self.dialect.quote, quoted);
            raw.push_str(chunk);
            if !quoted {
                break;
            }
            // The newline was inside a quoted field, so it is content.
            raw.push('\n');
        }

        self.split(&raw);
        Ok(true)
    }

    /// The fields of the record just read.
    pub fn record(&self) -> impl Iterator<Item = &str> {
        self.bounds.iter().map(|(a, b)| &self.text[*a..*b])
    }

    pub fn field_count(&self) -> usize {
        self.bounds.len()
    }

    fn split(&mut self, raw: &str) {
        let quote = self.dialect.quote as char;
        let delimiter = self.dialect.delimiter as char;
        let mut field_start = self.text.len();
        let mut in_quotes = false;
        let mut chars = raw.chars().peekable();

        while let Some(c) = chars.next() {
            if in_quotes {
                if c == quote {
                    if self.dialect.doubled_quotes && chars.peek() == Some(&quote) {
                        // `""` is one literal quote, not the end of the field.
                        chars.next();
                        self.text.push(quote);
                    } else {
                        in_quotes = false;
                    }
                } else {
                    self.text.push(c);
                }
            } else if c == quote {
                in_quotes = true;
            } else if c == delimiter {
                let end = self.text.len();
                self.bounds.push((field_start, end));
                field_start = end;
            } else {
                self.text.push(c);
            }
        }
        self.bounds.push((field_start, self.text.len()));
    }
}

/// True when the chunk leaves a quoted field open.
fn toggles_quotes(chunk: &str, quote: u8, mut quoted: bool) -> bool {
    let quote = quote as char;
    let mut chars = chunk.chars().peekable();
    while let Some(c) = chars.next() {
        if c != quote {
            continue;
        }
        if quoted && chars.peek() == Some(&quote) {
            chars.next();
            continue;
        }
        quoted = !quoted;
    }
    quoted
}

/// Reads one line's worth of bytes, in whatever unit the encoding counts in.
fn read_line<R: BufRead>(
    source: &mut R,
    out: &mut Vec<u8>,
    encoding: Encoding,
) -> io::Result<usize> {
    match encoding {
        // A UTF-16 newline is two bytes and one of them is zero, so a
        // byte-oriented `read_until` would cut a code unit in half.
        Encoding::Utf16Le | Encoding::Utf16Be => {
            let mut unit = [0u8; 2];
            let mut read = 0;
            loop {
                match fill(source, &mut unit)? {
                    0 => break,
                    n => read += n,
                }
                out.extend_from_slice(&unit);
                let value = match encoding {
                    Encoding::Utf16Le => u16::from_le_bytes(unit),
                    _ => u16::from_be_bytes(unit),
                };
                if value == u16::from(b'\n') {
                    break;
                }
            }
            Ok(read)
        }
        _ => source.read_until(b'\n', out),
    }
}

fn fill<R: BufRead>(source: &mut R, buffer: &mut [u8; 2]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < 2 {
        let read = source.read(&mut buffer[filled..])?;
        if read == 0 {
            return Ok(filled);
        }
        filled += read;
    }
    Ok(filled)
}

/// Writes one record, quoting only the fields that need it.
///
/// Excel quotes a field that contains the delimiter, a quote, or a newline, and
/// nothing else. Quoting everything is legal and makes a diff against Excel's
/// own output useless.
pub fn write_record<W: Write>(
    out: &mut W,
    fields: impl IntoIterator<Item = impl AsRef<str>>,
    dialect: Dialect,
) -> io::Result<()> {
    let mut first = true;
    for field in fields {
        if !first {
            out.write_all(&[dialect.delimiter])?;
        }
        first = false;
        write_field(out, field.as_ref(), dialect)?;
    }
    out.write_all(dialect.newline.as_str().as_bytes())
}

fn write_field<W: Write>(out: &mut W, field: &str, dialect: Dialect) -> io::Result<()> {
    let quote = dialect.quote as char;
    let needs = field
        .chars()
        .any(|c| c == dialect.delimiter as char || c == quote || c == '\n' || c == '\r')
        || field.starts_with(' ')
        || field.ends_with(' ');
    if !needs {
        return out.write_all(field.as_bytes());
    }
    out.write_all(&[dialect.quote])?;
    for c in field.chars() {
        if c == quote {
            out.write_all(&[dialect.quote])?;
        }
        let mut buffer = [0u8; 4];
        out.write_all(c.encode_utf8(&mut buffer).as_bytes())?;
    }
    out.write_all(&[dialect.quote])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn records(text: &str, dialect: Dialect) -> Vec<Vec<String>> {
        let mut reader = Reader::new(Cursor::new(text.as_bytes()), Encoding::Utf8, dialect);
        let mut out = Vec::new();
        while reader.next_record().expect("reads") {
            out.push(reader.record().map(str::to_string).collect());
        }
        out
    }

    #[test]
    fn the_delimiter_is_chosen_by_consistency_not_by_frequency() {
        // Six commas, three semicolons. The commas are inside prose and vary
        // per line; the semicolons are one per line. Counting frequency picks
        // the comma and turns this into one column of nonsense.
        let text = concat!(
            "Name;Note\n",
            "Smith;bought apples, pears, and figs\n",
            "Jones;asked about delivery, twice\n",
        );
        assert_eq!(sniff_dialect(text).delimiter, b';');
    }

    #[test]
    fn a_delimiter_inside_quotes_does_not_count() {
        let text = "\"Smith, John\";42\n\"Jones, Mary\";17\n\"Brown, Ann\";99\n";
        assert_eq!(sniff_dialect(text).delimiter, b';');
    }

    #[test]
    fn excels_own_declaration_is_believed_over_any_guess() {
        let text = "sep=|\nName|Note\nSmith,x|y\n";
        assert_eq!(sniff_dialect(text).delimiter, b'|');
    }

    #[test]
    fn a_declaration_line_is_consumed_rather_than_read_as_data() {
        let rows = records(
            "sep=|\na|b\nc|d\n",
            Dialect {
                delimiter: b'|',
                ..Dialect::default()
            },
        );
        assert_eq!(rows, vec![vec!["a", "b"], vec!["c", "d"]]);
    }

    #[test]
    fn quotes_doubled_inside_a_field_are_one_quote() {
        let rows = records("a,\"he said \"\"hi\"\"\",c\n", Dialect::default());
        assert_eq!(rows, vec![vec!["a", "he said \"hi\"", "c"]]);
    }

    #[test]
    fn a_newline_inside_quotes_is_content_and_not_a_new_record() {
        let rows = records("a,\"line one\nline two\",c\nd,e,f\n", Dialect::default());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][1], "line one\nline two");
        assert_eq!(rows[1], vec!["d", "e", "f"]);
    }

    #[test]
    fn empty_fields_survive_at_both_ends() {
        // A trailing delimiter means a trailing empty field, which is a column
        // of data and not a formatting artefact.
        let rows = records(",a,,b,\n", Dialect::default());
        assert_eq!(rows, vec![vec!["", "a", "", "b", ""]]);
    }

    #[test]
    fn a_file_with_no_delimiter_at_all_is_one_column() {
        let rows = records(
            "alpha\nbeta\ngamma\n",
            sniff_dialect("alpha\nbeta\ngamma\n"),
        );
        assert_eq!(rows, vec![vec!["alpha"], vec!["beta"], vec!["gamma"]]);
    }

    #[test]
    fn writing_quotes_only_what_has_to_be_quoted() {
        let mut out = Vec::new();
        write_record(
            &mut out,
            [
                "plain",
                "has,comma",
                "has\"quote",
                "has\nnewline",
                " padded ",
            ],
            Dialect {
                newline: Newline::Lf,
                ..Dialect::default()
            },
        )
        .expect("writes");
        assert_eq!(
            String::from_utf8(out).expect("utf-8"),
            "plain,\"has,comma\",\"has\"\"quote\",\"has\nnewline\",\" padded \"\n"
        );
    }

    #[test]
    fn what_is_written_reads_back_as_what_went_in() {
        let rows = vec![
            vec!["a".to_string(), "b,c".to_string()],
            vec!["\"q\"".to_string(), "multi\nline".to_string()],
            vec![String::new(), "  spaced  ".to_string()],
        ];
        let mut out = Vec::new();
        for row in &rows {
            write_record(&mut out, row, Dialect::default()).expect("writes");
        }
        let text = String::from_utf8(out).expect("utf-8");
        assert_eq!(records(&text, Dialect::default()), rows);
    }

    #[test]
    fn a_record_costs_the_longest_row_and_not_the_file() {
        // A hundred thousand rows through a reader that keeps one row.
        let mut source = String::new();
        for i in 0..100_000 {
            source.push_str(&format!("{i},value{i}\n"));
        }
        let mut reader = Reader::new(
            Cursor::new(source.as_bytes()),
            Encoding::Utf8,
            Dialect::default(),
        );
        let mut count = 0;
        let mut last = String::new();
        while reader.next_record().expect("reads") {
            count += 1;
            last = reader.record().next().unwrap_or_default().to_string();
            assert!(
                reader.text.len() < 64,
                "the buffer grew to {}",
                reader.text.len()
            );
        }
        assert_eq!(count, 100_000);
        assert_eq!(last, "99999");
    }
}
