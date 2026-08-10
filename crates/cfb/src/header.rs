//! The first 512 bytes, which say where everything else is.

use crate::error::{Error, Result};

/// `D0 CF 11 E0 A1 B1 1A E1` — the same eight bytes in front of every .doc,
/// .xls, .ppt and .msi ever written.
pub(crate) const SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// Sector numbers at or above this are not sectors; they are the sentinels
/// below.
pub(crate) const MAXREGSECT: u32 = 0xFFFF_FFFA;
pub(crate) const DIFSECT: u32 = 0xFFFF_FFFC;
pub(crate) const FATSECT: u32 = 0xFFFF_FFFD;
pub(crate) const ENDOFCHAIN: u32 = 0xFFFF_FFFE;
pub(crate) const FREESECT: u32 = 0xFFFF_FFFF;

/// The 109 DIFAT entries that fit in the header itself. Only a file with more
/// than 109 FAT sectors — about 56 MB at the v3 sector size — needs the DIFAT
/// chain at all, which is why the chain is the part readers get wrong.
const DIFAT_IN_HEADER: usize = 109;

#[derive(Debug, Clone)]
pub(crate) struct Header {
    pub sector_size: usize,
    pub mini_sector_size: usize,
    /// Streams shorter than this live in the mini stream. Always 4096 in
    /// practice, but it is a field, so it is read as one.
    pub mini_cutoff: u32,
    pub first_dir: u32,
    pub first_mini_fat: u32,
    pub mini_fat_count: u32,
    pub first_difat: u32,
    pub fat_count: u32,
    /// The DIFAT entries stored in the header, already trimmed of free slots.
    pub difat_head: Vec<u32>,
}

pub(crate) fn u16_at(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([data[at], data[at + 1]])
}

pub(crate) fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

pub(crate) fn u64_at(data: &[u8], at: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&data[at..at + 8]);
    u64::from_le_bytes(b)
}

impl Header {
    pub(crate) fn parse(data: &[u8]) -> Result<Header> {
        if data.len() < 512 {
            return Err(Error::NotCompound);
        }
        if data[..8] != SIGNATURE {
            return Err(Error::NotCompound);
        }

        // Byte order. The spec allows exactly one value, and a file claiming
        // big-endian is not something to guess at.
        if u16_at(data, 28) != 0xFFFE {
            return Err(Error::Unsupported("big-endian byte order"));
        }

        let sector_shift = u16_at(data, 30);
        let major = u16_at(data, 26);
        // Version 3 is 512-byte sectors, version 4 is 4096. The shift is what
        // actually decides; the version is checked so that a file which
        // disagrees with itself is refused rather than half-read.
        let sector_size = match (major, sector_shift) {
            (3, 9) => 512usize,
            (4, 12) => 4096usize,
            _ => return Err(Error::Unsupported("sector size")),
        };
        let mini_shift = u16_at(data, 32);
        if mini_shift != 6 {
            return Err(Error::Unsupported("mini sector size"));
        }

        let mut difat_head = Vec::with_capacity(DIFAT_IN_HEADER);
        for i in 0..DIFAT_IN_HEADER {
            let sector = u32_at(data, 76 + i * 4);
            if sector == FREESECT {
                break;
            }
            difat_head.push(sector);
        }

        Ok(Header {
            sector_size,
            mini_sector_size: 1 << mini_shift,
            mini_cutoff: u32_at(data, 56),
            first_dir: u32_at(data, 48),
            first_mini_fat: u32_at(data, 60),
            mini_fat_count: u32_at(data, 64),
            // `difat_count` at 72 is deliberately not kept: the chain ends
            // where it ends, and a header that disagrees with its own chain
            // should not be able to cut it short.
            first_difat: u32_at(data, 68),
            fat_count: u32_at(data, 44),
            difat_head,
        })
    }

    /// Where a sector begins.
    ///
    /// The header occupies the first sector-sized block whatever the sector
    /// size is, so sector 0 starts one sector in — which is why a 4096-byte
    /// file has 3584 bytes of padding after its header.
    pub(crate) fn offset(&self, sector: u32) -> u64 {
        (sector as u64 + 1) * self.sector_size as u64
    }
}
