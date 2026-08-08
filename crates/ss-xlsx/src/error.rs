//! Errors from reading a workbook.

use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// The container itself could not be read.
    Package(ooxml::Error),

    /// The package opened but is not a spreadsheet.
    ///
    /// Distinct from a malformed package: handing Calx a .docx should say so,
    /// not report corruption.
    NotAWorkbook(&'static str),

    /// A part we must understand did not parse.
    Xml { part: String, source: String },

    /// A required part is referenced but absent from the package.
    MissingPart {
        referenced_by: String,
        rel_id: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Package(e) => write!(f, "{e}"),
            Error::NotAWorkbook(why) => write!(f, "not a spreadsheet: {why}"),
            Error::Xml { part, source } => write!(f, "malformed XML in {part}: {source}"),
            Error::MissingPart {
                referenced_by,
                rel_id,
            } => write!(
                f,
                "{referenced_by} references {rel_id}, which is not in the package"
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Package(e) => Some(e),
            _ => None,
        }
    }
}

impl From<ooxml::Error> for Error {
    fn from(e: ooxml::Error) -> Self {
        Error::Package(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Package(ooxml::Error::Io(e))
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Attaches the part name to a quick-xml failure.
///
/// A bare "unexpected end of file" is useless when a workbook has forty parts.
pub(crate) fn xml_err(part: &str, e: impl fmt::Display) -> Error {
    Error::Xml {
        part: part.to_owned(),
        source: e.to_string(),
    }
}
