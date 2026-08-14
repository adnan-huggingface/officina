//! What can go wrong reading or writing a `.docx`.

use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Package(ooxml::Error),
    /// The package opened but is not a word processing document — most often a
    /// `.xlsx` that has been renamed. Reported as its own thing rather than as
    /// corruption, because the two need completely different advice.
    NotADocument(&'static str),
    /// A relationship names a part that is not in the package.
    MissingPart {
        referenced_by: String,
        rel_id: String,
    },
    Xml {
        part: String,
        detail: String,
    },
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Package(e) => write!(f, "{e}"),
            Error::NotADocument(why) => write!(f, "not a Word document: {why}"),
            Error::MissingPart {
                referenced_by,
                rel_id,
            } => write!(f, "{referenced_by} refers to {rel_id}, which is not there"),
            Error::Xml { part, detail } => write!(f, "{part}: {detail}"),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<ooxml::Error> for Error {
    fn from(e: ooxml::Error) -> Self {
        Error::Package(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
