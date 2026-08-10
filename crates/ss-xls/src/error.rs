//! Errors from reading a legacy workbook.

use std::fmt;

#[derive(Debug)]
pub enum Error {
    Container(cfb_reader::Error),

    /// A compound file, but not a workbook — a .doc, most likely.
    NotAWorkbook,

    /// BIFF, but an older dialect than this reader implements.
    ///
    /// Worth its own variant: "Excel 4 files are not supported" is actionable
    /// and "corrupt" is not.
    OldVersion(u16),

    /// The workbook stream is encrypted. Excel 97's own encryption, RC4 or the
    /// XOR obfuscation before it — either way there is nothing to read without
    /// the password.
    Encrypted,

    /// A record is shorter than the fields it must contain.
    Truncated(&'static str),

    /// Structurally wrong in a way that is not a truncation.
    Malformed(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Container(e) => write!(f, "{e}"),
            Error::NotAWorkbook => write!(f, "not a spreadsheet: no workbook stream"),
            Error::OldVersion(v) => write!(
                f,
                "this is an Excel {} workbook, and only Excel 97 and later (BIFF8) are read",
                match v {
                    0x0200 => "2",
                    0x0300 => "3",
                    0x0400 => "4",
                    0x0500 => "5 or 95",
                    _ => "workbook of an unknown version",
                }
            ),
            Error::Encrypted => write!(f, "the workbook is password-protected"),
            Error::Truncated(what) => write!(f, "a truncated {what} record"),
            Error::Malformed(what) => write!(f, "malformed workbook: {what}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Container(e) => Some(e),
            _ => None,
        }
    }
}

impl From<cfb_reader::Error> for Error {
    fn from(e: cfb_reader::Error) -> Self {
        Error::Container(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Container(cfb_reader::Error::Io(e))
    }
}

pub type Result<T> = std::result::Result<T, Error>;
