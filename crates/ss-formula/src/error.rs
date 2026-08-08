//! Parse errors.
//!
//! A formula that does not parse is *not* a fatal condition. Excel files contain
//! constructs we do not handle yet — structured table references, `LAMBDA`,
//! future functions carried under `_xlfn.` prefixes — and the right response is
//! to keep the text, mark the cell unevaluated, and carry on. Nothing here
//! should ever reach a user as a crash.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseErrorKind {
    UnexpectedChar,
    UnterminatedString,
    UnterminatedSheetName,
    ExpectedSheetSeparator,
    UnknownErrorLiteral,
    MalformedNumber,
    UnexpectedEnd,
    ExpectedExpression,
    ExpectedCloseParen,
    ExpectedCloseBrace,
    TrailingInput,
    /// A reference whose parts do not form a usable address.
    BadReference,
    /// Nesting deeper than we will parse. Guards the recursive descent against
    /// a hostile or generated formula blowing the stack.
    TooDeep,
}

impl ParseErrorKind {
    fn message(self) -> &'static str {
        match self {
            ParseErrorKind::UnexpectedChar => "unexpected character",
            ParseErrorKind::UnterminatedString => "unterminated string literal",
            ParseErrorKind::UnterminatedSheetName => "unterminated quoted sheet name",
            ParseErrorKind::ExpectedSheetSeparator => "expected `!` after a quoted sheet name",
            ParseErrorKind::UnknownErrorLiteral => "unrecognized error literal",
            ParseErrorKind::MalformedNumber => "malformed number",
            ParseErrorKind::UnexpectedEnd => "formula ended early",
            ParseErrorKind::ExpectedExpression => "expected an expression",
            ParseErrorKind::ExpectedCloseParen => "expected `)`",
            ParseErrorKind::ExpectedCloseBrace => "expected `}`",
            ParseErrorKind::TrailingInput => "unexpected text after the formula",
            ParseErrorKind::BadReference => "not a usable cell reference",
            ParseErrorKind::TooDeep => "formula nested too deeply",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseError {
    pub kind: ParseErrorKind,
    /// Byte offset into the formula text.
    pub at: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at offset {}", self.kind.message(), self.at)
    }
}

impl std::error::Error for ParseError {}
