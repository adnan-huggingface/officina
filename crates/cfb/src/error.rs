//! What can go wrong opening a compound file.

use std::fmt;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),

    /// The eight-byte signature is not there.
    ///
    /// Kept apart from [`Error::Malformed`] on purpose: handing the legacy
    /// reader an .xlsx should say "this is not a compound file", not "this file
    /// is corrupt".
    NotCompound,

    /// Well-formed, but uses something this reader does not implement.
    Unsupported(&'static str),

    /// The structure contradicts itself — a chain that runs off the end of the
    /// file, a directory sector that is not there, a loop.
    Malformed(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::NotCompound => write!(f, "not a compound file"),
            Error::Unsupported(what) => write!(f, "unsupported compound file: {what}"),
            Error::Malformed(what) => write!(f, "malformed compound file: {what}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
