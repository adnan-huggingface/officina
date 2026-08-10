//! The directory: 128-byte entries describing storages and streams.
//!
//! The entries are stored as a red-black tree of siblings, which matters to a
//! writer keeping it balanced and not at all to a reader: walking child and
//! sibling links visits every entry exactly once whatever the colours say.

use crate::error::{Error, Result};
use crate::header::{u16_at, u32_at, u64_at, MAXREGSECT};

pub(crate) const ENTRY_SIZE: usize = 128;
/// The index that means "no such entry" in a child or sibling link.
const NOSTREAM: u32 = 0xFFFF_FFFF;

/// What a directory entry stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The single root entry, which is also where the mini stream lives.
    Root,
    /// A directory.
    Storage,
    /// A file.
    Stream,
}

/// One entry, with the path it turned out to have.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The entry's own name, as stored.
    pub name: String,
    /// `Storage/Stream`, slash-separated, with the root contributing nothing.
    pub path: String,
    pub kind: Kind,
    pub size: u64,
    pub(crate) start: u32,
}

pub(crate) fn parse_all(bytes: &[u8]) -> Result<Vec<Raw>> {
    let mut out = Vec::with_capacity(bytes.len() / ENTRY_SIZE);
    for chunk in bytes.chunks_exact(ENTRY_SIZE) {
        out.push(Raw::parse(chunk)?);
    }
    Ok(out)
}

/// An entry before its path is known.
#[derive(Debug, Clone)]
pub(crate) struct Raw {
    pub name: String,
    pub kind: Option<Kind>,
    pub left: u32,
    pub right: u32,
    pub child: u32,
    pub start: u32,
    pub size: u64,
}

impl Raw {
    fn parse(e: &[u8]) -> Result<Raw> {
        // The name is UTF-16 and its length is in *bytes*, including the
        // terminator. A reader that treats it as a character count loses the
        // last letter of every name in the file.
        let name_bytes = u16_at(e, 64) as usize;
        let name_bytes = name_bytes.min(64).saturating_sub(2);
        let mut units = Vec::with_capacity(name_bytes / 2);
        for i in (0..name_bytes).step_by(2) {
            units.push(u16_at(e, i));
        }
        let name = String::from_utf16_lossy(&units);

        let kind = match e[66] {
            0 => None, // unallocated: a hole left by a deletion
            1 => Some(Kind::Storage),
            2 => Some(Kind::Stream),
            5 => Some(Kind::Root),
            _ => return Err(Error::Malformed("an unknown directory entry type")),
        };

        // In version 3 only the low four bytes of the size are meaningful; the
        // high four are documented as "must be zero" and are not always. Files
        // that big do not exist in this format, so the high word is dropped
        // rather than believed.
        let size = u64_at(e, 120) & 0xFFFF_FFFF;

        Ok(Raw {
            name,
            kind,
            left: u32_at(e, 68),
            right: u32_at(e, 72),
            child: u32_at(e, 76),
            start: u32_at(e, 116),
            size,
        })
    }
}

/// Flatten the tree into a list of entries with paths.
///
/// `visited` is not defensive tidiness: a corrupt file whose sibling link
/// points at its own parent would otherwise walk forever.
pub(crate) fn walk(raw: &[Raw]) -> Result<Vec<Entry>> {
    let root = raw.first().ok_or(Error::Malformed("no root entry"))?;
    if root.kind != Some(Kind::Root) {
        return Err(Error::Malformed("the first entry is not the root"));
    }

    let mut out = vec![Entry {
        name: root.name.clone(),
        path: String::new(),
        kind: Kind::Root,
        size: root.size,
        start: root.start,
    }];
    let mut visited = vec![false; raw.len()];
    visited[0] = true;

    // (entry index, the path of its parent storage)
    let mut stack: Vec<(u32, String)> = Vec::new();
    if root.child != NOSTREAM {
        stack.push((root.child, String::new()));
    }

    while let Some((index, parent)) = stack.pop() {
        let Some(entry) = raw.get(index as usize) else {
            return Err(Error::Malformed("a directory link past the last entry"));
        };
        if std::mem::replace(&mut visited[index as usize], true) {
            return Err(Error::Malformed("a loop in the directory tree"));
        }
        let Some(kind) = entry.kind else { continue };

        let path = if parent.is_empty() {
            entry.name.clone()
        } else {
            format!("{parent}/{}", entry.name)
        };

        if entry.child != NOSTREAM {
            stack.push((entry.child, path.clone()));
        }
        for sibling in [entry.left, entry.right] {
            if sibling != NOSTREAM {
                stack.push((sibling, parent.clone()));
            }
        }

        out.push(Entry {
            name: entry.name.clone(),
            path,
            kind,
            size: entry.size,
            start: entry.start,
        });
    }

    Ok(out)
}

/// A sector number that is a sector, not a sentinel.
pub(crate) fn real(sector: u32) -> bool {
    sector <= MAXREGSECT
}
