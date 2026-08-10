//! Compound File Binary (MS-CFB) reader. Feeds the legacy .doc/.xls readers.
//!
//! A compound file is a small FAT filesystem inside a single file: sectors,
//! an allocation table, a directory of named streams. Everything Microsoft
//! shipped before the XML formats is one — `.doc`, `.xls`, `.ppt`, `.msi` — and
//! all of them begin with the same eight bytes.
//!
//! There are two allocation tables rather than one. Streams shorter than 4096
//! bytes are not given whole 512-byte sectors; they are packed into 64-byte
//! *mini* sectors inside one ordinary stream, the mini stream, which hangs off
//! the root directory entry and has its own FAT. A reader that implements only
//! the large path opens big files and silently returns nothing for small ones —
//! which in a .xls means the summary information, and in a small workbook the
//! workbook itself.
//!
//! The whole file is held in memory. These are legacy documents: the format's
//! own sector numbering runs out around 2 GB, and a reader that streams would
//! be seeking backwards constantly anyway, because a chain is in no particular
//! order.
//!
//! Read-only, and permanently so. `DESIGN.md` §9: the legacy formats are read
//! with save-as-modern as the escape hatch, so nothing here writes.

#![forbid(unsafe_code)]

mod dir;
mod error;
mod fat;
mod header;

#[cfg(any(test, feature = "test-support"))]
pub mod fixture;

use std::path::Path;

pub use dir::{Entry, Kind};
pub use error::{Error, Result};

use header::Header;

pub struct Cfb {
    data: Vec<u8>,
    header: Header,
    fat: Vec<u32>,
    mini_fat: Vec<u32>,
    /// The mini stream, already assembled: every stream below the cutoff is a
    /// slice of chained pieces of this.
    mini: Vec<u8>,
    entries: Vec<Entry>,
}

/// Written by hand rather than derived: the derived one prints the whole file.
impl std::fmt::Debug for Cfb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cfb")
            .field("bytes", &self.data.len())
            .field("entries", &self.entries.len())
            .finish()
    }
}

impl Cfb {
    pub fn open(path: impl AsRef<Path>) -> Result<Cfb> {
        Cfb::read(std::fs::read(path)?)
    }

    pub fn read(data: Vec<u8>) -> Result<Cfb> {
        let header = Header::parse(&data)?;
        let fat = fat::read_fat(&data, &header)?;

        let mini_fat = {
            let mut table = Vec::new();
            if dir::real(header.first_mini_fat) {
                let per_sector = header.sector_size / 4;
                let chain = fat::chain(&fat, header.first_mini_fat)?;
                // `mini_fat_count` is what the header claims; the chain is what
                // the file actually has. The shorter of the two is what can be
                // read without inventing entries.
                let take = chain.len().min(header.mini_fat_count as usize);
                for n in chain.into_iter().take(take) {
                    let s = fat::sector(&data, &header, n)?;
                    for i in 0..per_sector {
                        table.push(header::u32_at(s, i * 4));
                    }
                }
            }
            table
        };

        // The directory is itself a chain. Its length is not recorded in
        // version 3 — the chain simply ends — so every sector of it is read and
        // the unallocated entries are skipped by the walk.
        let dir_bytes = {
            let chain = fat::chain(&fat, header.first_dir)?;
            let mut bytes = Vec::with_capacity(chain.len() * header.sector_size);
            for n in chain {
                bytes.extend_from_slice(fat::sector(&data, &header, n)?);
            }
            bytes
        };
        let raw = dir::parse_all(&dir_bytes)?;
        let entries = dir::walk(&raw)?;

        // The root entry is the mini stream's own directory entry: its start
        // sector and size describe the mini stream, not a file of its own.
        let root = &raw[0];
        let mut cfb = Cfb {
            data,
            header,
            fat,
            mini_fat,
            mini: Vec::new(),
            entries,
        };
        cfb.mini = cfb.large(root.start, root.size)?;
        Ok(cfb)
    }

    /// Every stream and storage in the file, root first.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Find a stream by path — `Workbook`, or `Storage/Stream` inside one.
    ///
    /// Matched without regard to case. The format stores names verbatim and
    /// Excel writes `Workbook`, but the comparison Windows itself makes on
    /// these names is case-insensitive, and files written by other producers
    /// take advantage of it.
    pub fn entry(&self, path: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.kind == Kind::Stream && e.path.eq_ignore_ascii_case(path))
    }

    /// A stream's bytes, or `None` if there is no such stream.
    pub fn stream(&self, path: &str) -> Result<Option<Vec<u8>>> {
        match self.entry(path) {
            Some(entry) => self.read_stream(entry).map(Some),
            None => Ok(None),
        }
    }

    pub fn read_stream(&self, entry: &Entry) -> Result<Vec<u8>> {
        if entry.size < self.header.mini_cutoff as u64 {
            self.small(entry.start, entry.size)
        } else {
            self.large(entry.start, entry.size)
        }
    }

    /// A chain of ordinary sectors, truncated to the recorded size.
    fn large(&self, start: u32, size: u64) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(size as usize);
        for n in fat::chain(&self.fat, start)? {
            if out.len() as u64 >= size {
                break;
            }
            out.extend_from_slice(fat::sector(&self.data, &self.header, n)?);
        }
        if (out.len() as u64) < size {
            return Err(Error::Malformed("a stream shorter than its recorded size"));
        }
        out.truncate(size as usize);
        Ok(out)
    }

    /// A chain of mini sectors, cut out of the mini stream.
    fn small(&self, start: u32, size: u64) -> Result<Vec<u8>> {
        let unit = self.header.mini_sector_size;
        let mut out = Vec::with_capacity(size as usize);
        for n in fat::chain(&self.mini_fat, start)? {
            if out.len() as u64 >= size {
                break;
            }
            let at = n as usize * unit;
            let piece = self
                .mini
                .get(at..at + unit)
                .ok_or(Error::Malformed("a mini sector past the mini stream"))?;
            out.extend_from_slice(piece);
        }
        if (out.len() as u64) < size {
            return Err(Error::Malformed("a stream shorter than its recorded size"));
        }
        out.truncate(size as usize);
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
