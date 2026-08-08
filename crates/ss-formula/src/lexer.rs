//! Tokenizer for Excel formula text.
//!
//! The input is the formula *without* its leading `=`, exactly as stored in the
//! file — `ss-xlsx` keeps it that way, and so does Excel.
//!
//! Two things make this more than a textbook lexer:
//!
//! * References are lexed with maximal munch rather than assembled by the parser.
//!   `A1` is a cell but `A` is a name, `A:C` is a column span but `A1:C3` is a
//!   range of two cells joined by an operator, and telling those apart after the
//!   fact needs more lookahead than a Pratt parser wants to carry.
//! * Whitespace is significant. A space between two references is Excel's
//!   intersection operator, so it cannot simply be skipped — each token records
//!   whether space preceded it and the parser decides what that means.

use ss_model::CellError;

use crate::error::{ParseError, ParseErrorKind};

/// A cell address as *written*, with absoluteness preserved.
///
/// Absoluteness is a property of the reference, not of the address it resolves
/// to, so it belongs here rather than in `ss_model::CellRef`. It is what decides
/// whether a reference moves when the formula is copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct A1 {
    pub col: u32,
    pub col_abs: bool,
    pub row: u32,
    pub row_abs: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Number(f64),
    Text(String),
    Bool(bool),
    Error(CellError),
    /// A function name or a defined name. Which one it is depends on whether a
    /// `(` follows, which is the parser's business.
    Name(String),
    /// A `Sheet1!` or `'Q1 Results'!` qualifier. The `!` is consumed.
    Sheet(String),
    Cell(A1),
    /// `A:C` — whole columns.
    ColSpan {
        start: u32,
        start_abs: bool,
        end: u32,
        end_abs: bool,
    },
    /// `2:5` — whole rows.
    RowSpan {
        start: u32,
        start_abs: bool,
        end: u32,
        end_abs: bool,
    },

    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Semicolon,
    Colon,

    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Percent,
    Amp,

    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: Tok,
    /// Whether whitespace immediately preceded this token.
    ///
    /// Carries the intersection operator: `A1:B5 C1:D9` intersects, while
    /// `A1:B5` alone does not.
    pub space_before: bool,
    /// Byte offset of the token's start, for error reporting.
    pub at: usize,
}

pub fn tokenize(input: &str) -> Result<Vec<Token>, ParseError> {
    Lexer {
        src: input.as_bytes(),
        text: input,
        pos: 0,
    }
    .run()
}

struct Lexer<'a> {
    src: &'a [u8],
    text: &'a str,
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn run(mut self) -> Result<Vec<Token>, ParseError> {
        let mut out = Vec::new();
        loop {
            let space = self.skip_space();
            if self.pos >= self.src.len() {
                break;
            }
            let at = self.pos;
            let kind = self.next_token()?;
            out.push(Token {
                kind,
                space_before: space,
                at,
            });
        }
        Ok(out)
    }

    fn skip_space(&mut self) -> bool {
        let start = self.pos;
        while self
            .src
            .get(self.pos)
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        {
            self.pos += 1;
        }
        self.pos > start
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.src.get(self.pos + offset).copied()
    }

    fn err(&self, kind: ParseErrorKind) -> ParseError {
        ParseError { kind, at: self.pos }
    }

    fn next_token(&mut self) -> Result<Tok, ParseError> {
        let b = self.peek().expect("caller checked for input");

        match b {
            b'(' => self.single(Tok::LParen),
            b')' => self.single(Tok::RParen),
            b'{' => self.single(Tok::LBrace),
            b'}' => self.single(Tok::RBrace),
            b',' => self.single(Tok::Comma),
            b';' => self.single(Tok::Semicolon),
            b':' => self.single(Tok::Colon),
            b'+' => self.single(Tok::Plus),
            b'-' => self.single(Tok::Minus),
            b'*' => self.single(Tok::Star),
            b'/' => self.single(Tok::Slash),
            b'^' => self.single(Tok::Caret),
            b'%' => self.single(Tok::Percent),
            b'&' => self.single(Tok::Amp),
            b'=' => self.single(Tok::Eq),
            b'<' => match self.peek_at(1) {
                Some(b'>') => self.double(Tok::Ne),
                Some(b'=') => self.double(Tok::Le),
                _ => self.single(Tok::Lt),
            },
            b'>' => match self.peek_at(1) {
                Some(b'=') => self.double(Tok::Ge),
                _ => self.single(Tok::Gt),
            },
            b'"' => self.string(),
            b'#' => self.error_literal(),
            b'\'' => self.quoted_sheet(),
            b'0'..=b'9' => self.number_or_row_span(),
            b'.' => self.number_or_row_span(),
            b'$' | b'A'..=b'Z' | b'a'..=b'z' | b'_' | b'\\' => self.reference_or_name(),
            _ => Err(self.err(ParseErrorKind::UnexpectedChar)),
        }
    }

    fn single(&mut self, t: Tok) -> Result<Tok, ParseError> {
        self.pos += 1;
        Ok(t)
    }

    fn double(&mut self, t: Tok) -> Result<Tok, ParseError> {
        self.pos += 2;
        Ok(t)
    }

    /// `"abc"`, where a literal quote is doubled: `"say ""hi"""`.
    fn string(&mut self) -> Result<Tok, ParseError> {
        let start = self.pos;
        self.pos += 1; // opening quote
        let mut out = String::new();
        loop {
            match self.peek() {
                None => {
                    self.pos = start;
                    return Err(self.err(ParseErrorKind::UnterminatedString));
                }
                Some(b'"') => {
                    if self.peek_at(1) == Some(b'"') {
                        out.push('"');
                        self.pos += 2;
                    } else {
                        self.pos += 1;
                        return Ok(Tok::Text(out));
                    }
                }
                Some(_) => {
                    // Step by whole characters: cell text is routinely non-ASCII
                    // and slicing mid-sequence would panic.
                    let rest = &self.text[self.pos..];
                    let ch = rest.chars().next().expect("bytes remain");
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn error_literal(&mut self) -> Result<Tok, ParseError> {
        // Longest first: #N/A is a prefix of nothing, but #REF! and #NULL! share
        // no prefix either — matching in length order is simply the safe habit.
        const CODES: [&str; 9] = [
            "#GETTING_DATA",
            "#DIV/0!",
            "#VALUE!",
            "#NAME?",
            "#NULL!",
            "#REF!",
            "#NUM!",
            "#N/A",
            "#SPILL!",
        ];
        let rest = &self.text[self.pos..];
        for code in CODES {
            if rest.starts_with(code) {
                self.pos += code.len();
                // #SPILL! postdates our error set; it maps onto no CellError, so
                // it is reported as #VALUE! rather than silently dropped.
                let err = CellError::from_code(code).unwrap_or(CellError::Value);
                return Ok(Tok::Error(err));
            }
        }
        Err(self.err(ParseErrorKind::UnknownErrorLiteral))
    }

    /// `'Q1 Results'!` — a sheet name needing quotes. Inner quotes are doubled.
    fn quoted_sheet(&mut self) -> Result<Tok, ParseError> {
        let start = self.pos;
        self.pos += 1;
        let mut name = String::new();
        loop {
            match self.peek() {
                None => {
                    self.pos = start;
                    return Err(self.err(ParseErrorKind::UnterminatedSheetName));
                }
                Some(b'\'') => {
                    if self.peek_at(1) == Some(b'\'') {
                        name.push('\'');
                        self.pos += 2;
                    } else {
                        self.pos += 1;
                        break;
                    }
                }
                Some(_) => {
                    let rest = &self.text[self.pos..];
                    let ch = rest.chars().next().expect("bytes remain");
                    name.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
        if self.peek() != Some(b'!') {
            self.pos = start;
            return Err(self.err(ParseErrorKind::ExpectedSheetSeparator));
        }
        self.pos += 1;
        Ok(Tok::Sheet(name))
    }

    /// A number, or a whole-row span like `2:5`.
    ///
    /// Both start with digits, and only the `:` that may follow tells them apart.
    fn number_or_row_span(&mut self) -> Result<Tok, ParseError> {
        let start = self.pos;
        let digits_end = self.scan_digits(self.pos);

        // `2:5` is rows 2 through 5. `2.5` is a number, and `2:` with a
        // non-numeric right side is a syntax error the parser will report.
        if digits_end > start && self.src.get(digits_end) == Some(&b':') {
            if let Some(tok) = self.try_row_span(start) {
                return Ok(tok);
            }
        }

        let mut end = digits_end;
        if self.src.get(end) == Some(&b'.') {
            end = self.scan_digits(end + 1);
        }
        if matches!(self.src.get(end), Some(b'e' | b'E')) {
            let mut exp = end + 1;
            if matches!(self.src.get(exp), Some(b'+' | b'-')) {
                exp += 1;
            }
            let after = self.scan_digits(exp);
            if after > exp {
                end = after;
            }
        }

        let text = &self.text[start..end];
        let value: f64 = text
            .parse()
            .map_err(|_| self.err(ParseErrorKind::MalformedNumber))?;
        self.pos = end;
        Ok(Tok::Number(value))
    }

    fn try_row_span(&mut self, start: usize) -> Option<Tok> {
        let (first, after_first) = self.read_row(start)?;
        if self.src.get(after_first) != Some(&b':') {
            return None;
        }
        let (second, after_second) = self.read_row(after_first + 1)?;
        self.pos = after_second;
        Some(Tok::RowSpan {
            start: first.0.min(second.0),
            start_abs: first.1,
            end: first.0.max(second.0),
            end_abs: second.1,
        })
    }

    /// Reads `$?123` as a zero-based row index, returning it and the position after.
    fn read_row(&self, mut at: usize) -> Option<((u32, bool), usize)> {
        let abs = self.src.get(at) == Some(&b'$');
        if abs {
            at += 1;
        }
        let end = self.scan_digits(at);
        if end == at {
            return None;
        }
        let n: u32 = self.text[at..end].parse().ok()?;
        if n == 0 || n > ss_model::cell::MAX_ROWS {
            return None;
        }
        Some(((n - 1, abs), end))
    }

    fn scan_digits(&self, mut at: usize) -> usize {
        while self.src.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        at
    }

    /// A cell reference, a column span, a sheet qualifier, or a plain name.
    fn reference_or_name(&mut self) -> Result<Tok, ParseError> {
        let start = self.pos;

        // Try the reference shapes first, longest match wins. Each returns None
        // without consuming if the text does not fit, so a name like `SUM` or
        // `Tax_Rate` falls through cleanly.
        if let Some(tok) = self.try_cell(start) {
            return Ok(tok);
        }
        if let Some(tok) = self.try_col_span(start) {
            return Ok(tok);
        }

        // A bare name. `!` after it makes it a sheet qualifier instead.
        let mut end = start;
        while self.src.get(end).is_some_and(|&b| {
            b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'\\' || b >= 0x80
        }) {
            end += 1;
        }
        if end == start {
            return Err(self.err(ParseErrorKind::UnexpectedChar));
        }
        let word = &self.text[start..end];

        if self.src.get(end) == Some(&b'!') {
            self.pos = end + 1;
            return Ok(Tok::Sheet(word.to_owned()));
        }

        self.pos = end;
        match word {
            w if w.eq_ignore_ascii_case("TRUE") => Ok(Tok::Bool(true)),
            w if w.eq_ignore_ascii_case("FALSE") => Ok(Tok::Bool(false)),
            w => Ok(Tok::Name(w.to_owned())),
        }
    }

    /// `$?A$?1`, but only when not followed by more name characters.
    fn try_cell(&mut self, start: usize) -> Option<Tok> {
        let (col, col_abs, after_col) = self.read_col(start)?;
        let ((row, row_abs), after_row) = self.read_row(after_col)?;

        // `A1B` is a name, not cell A1 followed by B. Requiring the reference to
        // end at a non-name character is what keeps them apart.
        if self.src.get(after_row).is_some_and(|&b| {
            b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'\\' || b >= 0x80
        }) {
            return None;
        }

        // `Q1!A5` is sheet Q1, never cell Q1 — nothing may follow a cell
        // reference with `!`. Sheets named like cell addresses are common
        // (`Q1`, `Q4`, `A1`), and reading one as a cell silently points the
        // formula at the wrong place on the wrong sheet.
        if self.src.get(after_row) == Some(&b'!') {
            return None;
        }
        self.pos = after_row;
        Some(Tok::Cell(A1 {
            col,
            col_abs,
            row,
            row_abs,
        }))
    }

    /// `A:C` — whole columns.
    fn try_col_span(&mut self, start: usize) -> Option<Tok> {
        let (first, first_abs, after_first) = self.read_col(start)?;
        if self.src.get(after_first) != Some(&b':') {
            return None;
        }
        let (second, second_abs, after_second) = self.read_col(after_first + 1)?;
        // Same guard as `try_cell`: `A:CAT` is not a column span.
        if self
            .src
            .get(after_second)
            .is_some_and(|&b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return None;
        }
        self.pos = after_second;
        Some(Tok::ColSpan {
            start: first.min(second),
            start_abs: first_abs,
            end: first.max(second),
            end_abs: second_abs,
        })
    }

    /// Reads `$?AB` as a zero-based column index, returning it and the position after.
    fn read_col(&self, mut at: usize) -> Option<(u32, bool, usize)> {
        let abs = self.src.get(at) == Some(&b'$');
        if abs {
            at += 1;
        }
        let start = at;
        while self.src.get(at).is_some_and(u8::is_ascii_alphabetic) {
            at += 1;
        }
        // Excel's last column is XFD, so more than three letters is a name.
        if at == start || at - start > 3 {
            return None;
        }
        let col = ss_model::column_index(&self.text[start..at])?;
        Some((col, abs, at))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(input: &str) -> Vec<Tok> {
        tokenize(input)
            .expect("tokenizes")
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    fn cell(s: &str) -> Tok {
        let toks = kinds(s);
        assert_eq!(toks.len(), 1, "{s} should lex to one token, got {toks:?}");
        toks.into_iter().next().unwrap()
    }

    #[test]
    fn numbers_in_every_spelling() {
        assert_eq!(kinds("1"), [Tok::Number(1.0)]);
        assert_eq!(kinds("1.5"), [Tok::Number(1.5)]);
        assert_eq!(kinds(".5"), [Tok::Number(0.5)]);
        assert_eq!(kinds("1e3"), [Tok::Number(1000.0)]);
        assert_eq!(kinds("1.5E-2"), [Tok::Number(0.015)]);
    }

    #[test]
    fn strings_unescape_doubled_quotes() {
        assert_eq!(kinds(r#""abc""#), [Tok::Text("abc".into())]);
        assert_eq!(kinds(r#""say ""hi""""#), [Tok::Text(r#"say "hi""#.into())]);
        assert_eq!(kinds(r#""""#), [Tok::Text(String::new())]);
    }

    #[test]
    fn non_ascii_string_content_does_not_split_a_character() {
        assert_eq!(
            kinds("\"\u{4F60}\u{597D}\""),
            [Tok::Text("\u{4F60}\u{597D}".into())]
        );
    }

    #[test]
    fn unterminated_string_is_an_error_not_a_panic() {
        assert!(tokenize(r#""abc"#).is_err());
    }

    #[test]
    fn cell_references_keep_their_absoluteness() {
        assert_eq!(
            cell("A1"),
            Tok::Cell(A1 {
                col: 0,
                col_abs: false,
                row: 0,
                row_abs: false
            })
        );
        assert_eq!(
            cell("$A$1"),
            Tok::Cell(A1 {
                col: 0,
                col_abs: true,
                row: 0,
                row_abs: true
            })
        );
        assert_eq!(
            cell("$B2"),
            Tok::Cell(A1 {
                col: 1,
                col_abs: true,
                row: 1,
                row_abs: false
            })
        );
        assert_eq!(
            cell("XFD1048576"),
            Tok::Cell(A1 {
                col: 16_383,
                col_abs: false,
                row: 1_048_575,
                row_abs: false
            })
        );
    }

    #[test]
    fn things_that_look_like_references_but_are_names() {
        // The classic ambiguity. `A1B` and `ABCD1` are legal defined names.
        assert_eq!(kinds("A1B"), [Tok::Name("A1B".into())]);
        assert_eq!(kinds("ABCD1"), [Tok::Name("ABCD1".into())]);
        assert_eq!(kinds("Tax_Rate"), [Tok::Name("Tax_Rate".into())]);
        assert_eq!(kinds("SUM"), [Tok::Name("SUM".into())]);
        // Past the last column, so not a reference.
        assert_eq!(kinds("XFE1"), [Tok::Name("XFE1".into())]);
    }

    #[test]
    fn booleans_are_case_insensitive_keywords() {
        assert_eq!(kinds("TRUE"), [Tok::Bool(true)]);
        assert_eq!(kinds("false"), [Tok::Bool(false)]);
        assert_eq!(kinds("True"), [Tok::Bool(true)]);
    }

    #[test]
    fn ranges_lex_as_two_cells_and_a_colon() {
        assert_eq!(
            kinds("A1:B2"),
            [
                Tok::Cell(A1 {
                    col: 0,
                    col_abs: false,
                    row: 0,
                    row_abs: false
                }),
                Tok::Colon,
                Tok::Cell(A1 {
                    col: 1,
                    col_abs: false,
                    row: 1,
                    row_abs: false
                }),
            ]
        );
    }

    #[test]
    fn whole_column_and_row_spans_lex_as_units() {
        assert_eq!(
            kinds("A:C"),
            [Tok::ColSpan {
                start: 0,
                start_abs: false,
                end: 2,
                end_abs: false
            }]
        );
        assert_eq!(
            kinds("$B:$D"),
            [Tok::ColSpan {
                start: 1,
                start_abs: true,
                end: 3,
                end_abs: true
            }]
        );
        assert_eq!(
            kinds("2:5"),
            [Tok::RowSpan {
                start: 1,
                start_abs: false,
                end: 4,
                end_abs: false
            }]
        );
    }

    #[test]
    fn a_column_span_reversed_is_normalized() {
        assert_eq!(
            kinds("C:A"),
            [Tok::ColSpan {
                start: 0,
                start_abs: false,
                end: 2,
                end_abs: false
            }]
        );
    }

    #[test]
    fn sheet_qualifiers_quoted_and_bare() {
        assert_eq!(
            kinds("Sheet1!A1"),
            [
                Tok::Sheet("Sheet1".into()),
                Tok::Cell(A1 {
                    col: 0,
                    col_abs: false,
                    row: 0,
                    row_abs: false
                })
            ]
        );
        assert_eq!(
            kinds("'Q1 Results'!A1"),
            [
                Tok::Sheet("Q1 Results".into()),
                Tok::Cell(A1 {
                    col: 0,
                    col_abs: false,
                    row: 0,
                    row_abs: false
                })
            ]
        );
    }

    #[test]
    fn a_sheet_name_may_contain_a_doubled_quote() {
        assert_eq!(
            kinds("'Bob''s Data'!A1"),
            [
                Tok::Sheet("Bob's Data".into()),
                Tok::Cell(A1 {
                    col: 0,
                    col_abs: false,
                    row: 0,
                    row_abs: false
                })
            ]
        );
    }

    #[test]
    fn error_literals_lex_whole() {
        assert_eq!(kinds("#DIV/0!"), [Tok::Error(CellError::Div0)]);
        assert_eq!(kinds("#N/A"), [Tok::Error(CellError::NotAvailable)]);
        assert_eq!(kinds("#REF!"), [Tok::Error(CellError::Ref)]);
        assert_eq!(kinds("#NAME?"), [Tok::Error(CellError::Name)]);
    }

    #[test]
    fn two_character_comparisons_beat_one() {
        assert_eq!(kinds("<="), [Tok::Le]);
        assert_eq!(kinds(">="), [Tok::Ge]);
        assert_eq!(kinds("<>"), [Tok::Ne]);
        assert_eq!(kinds("<"), [Tok::Lt]);
        assert_eq!(kinds(">"), [Tok::Gt]);
    }

    #[test]
    fn space_before_is_recorded_for_the_intersection_operator() {
        let toks = tokenize("A1:B5 C1:D9").expect("tokenizes");
        // The token starting the second reference is the one that must know.
        let second_ref = toks
            .iter()
            .position(|t| t.space_before)
            .expect("some token follows a space");
        assert_eq!(
            toks[second_ref].kind,
            Tok::Cell(A1 {
                col: 2,
                col_abs: false,
                row: 0,
                row_abs: false
            })
        );
        assert!(!toks[0].space_before);
    }

    #[test]
    fn a_call_lexes_into_its_pieces() {
        assert_eq!(
            kinds("SUM(A1,2)"),
            [
                Tok::Name("SUM".into()),
                Tok::LParen,
                Tok::Cell(A1 {
                    col: 0,
                    col_abs: false,
                    row: 0,
                    row_abs: false
                }),
                Tok::Comma,
                Tok::Number(2.0),
                Tok::RParen,
            ]
        );
    }

    #[test]
    fn junk_is_rejected() {
        assert!(tokenize("@").is_err());
        assert!(tokenize("#WHAT!").is_err());
    }
}
