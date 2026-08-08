//! Errors from package handling.

use std::fmt;

use crate::name::PartName;

#[derive(Debug)]
pub enum Error {
    /// The file is not a readable zip container.
    NotAPackage(String),
    /// `[Content_Types].xml` is missing. Every OPC package must have one.
    MissingContentTypes,
    /// A part exists in the container but no content type applies to it.
    ///
    /// Not fatal for retained parts — we fall back to a binary content type —
    /// but worth surfacing, because it usually means a malformed producer.
    UnknownContentType(PartName),
    /// XML that we must understand did not parse.
    Xml {
        part: PartName,
        source: String,
    },
    /// A part name violates the OPC naming rules (§9.1.1.1 of ECMA-376 part 2).
    BadPartName {
        raw: String,
        reason: &'static str,
    },
    /// A relationship points at a part that is not in the package.
    DanglingRelationship {
        source: PartName,
        target: String,
    },
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotAPackage(why) => write!(f, "not a valid OPC package: {why}"),
            Error::MissingContentTypes => f.write_str("package has no [Content_Types].xml"),
            Error::UnknownContentType(p) => write!(f, "no content type declared for part {p}"),
            Error::Xml { part, source } => write!(f, "malformed XML in {part}: {source}"),
            Error::BadPartName { raw, reason } => {
                write!(f, "invalid part name {raw:?}: {reason}")
            }
            Error::DanglingRelationship { source, target } => {
                write!(
                    f,
                    "relationship from {source} points at missing part {target:?}"
                )
            }
            Error::Io(e) => write!(f, "io error: {e}"),
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
