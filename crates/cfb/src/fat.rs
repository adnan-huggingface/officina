//! Sector allocation: the FAT, the DIFAT that indexes it, and chain walking.
//!
//! A compound file is a FAT filesystem in a file. The FAT is one array of
//! `u32`, one entry per sector, each holding the *next* sector of whatever
//! chain that sector belongs to. The DIFAT is the index of which sectors hold
//! the FAT itself: the first 109 entries live in the header, and only a file
//! past about 56 MB needs the chain of DIFAT sectors after them.

use crate::error::{Error, Result};
use crate::header::{u32_at, Header, DIFSECT, ENDOFCHAIN, FATSECT, FREESECT, MAXREGSECT};

/// One sector's bytes.
pub(crate) fn sector<'a>(data: &'a [u8], header: &Header, n: u32) -> Result<&'a [u8]> {
    if n > MAXREGSECT {
        return Err(Error::Malformed("a sentinel used as a sector number"));
    }
    let start = header.offset(n);
    let end = start + header.sector_size as u64;
    if end > data.len() as u64 {
        return Err(Error::Malformed("a sector past the end of the file"));
    }
    Ok(&data[start as usize..end as usize])
}

/// Every FAT entry, in sector order.
pub(crate) fn read_fat(data: &[u8], header: &Header) -> Result<Vec<u32>> {
    let per_sector = header.sector_size / 4;
    // An upper bound that cannot be exceeded by an honest file: there is one
    // FAT entry per sector, so there cannot be more FAT sectors than the file
    // has sectors. Without this a file claiming four billion FAT sectors would
    // be an allocation rather than an error.
    let sectors_in_file = (data.len() / header.sector_size).max(1);

    let mut fat_sectors: Vec<u32> = header.difat_head.clone();

    // The DIFAT chain. Each of its sectors holds `per_sector - 1` FAT sector
    // numbers and, in its last slot, the next DIFAT sector.
    let mut next = header.first_difat;
    let mut seen = 0usize;
    while next <= MAXREGSECT {
        if seen > sectors_in_file {
            return Err(Error::Malformed("a loop in the DIFAT chain"));
        }
        seen += 1;
        let s = sector(data, header, next)?;
        for i in 0..per_sector - 1 {
            let entry = u32_at(s, i * 4);
            if entry == FREESECT {
                continue;
            }
            fat_sectors.push(entry);
        }
        next = u32_at(s, (per_sector - 1) * 4);
    }

    // `fat_count` is a stated length, not a trusted one. A file whose header
    // and DIFAT disagree is read to whichever is shorter rather than refused:
    // the disagreement is usually a truncated writer, and the sectors that are
    // there are still readable.
    let stated = header.fat_count as usize;
    if stated < fat_sectors.len() {
        fat_sectors.truncate(stated);
    }
    if fat_sectors.len() > sectors_in_file + 1 {
        return Err(Error::Malformed(
            "more FAT sectors than the file has sectors",
        ));
    }

    let mut fat = Vec::with_capacity(fat_sectors.len() * per_sector);
    for n in fat_sectors {
        let s = sector(data, header, n)?;
        for i in 0..per_sector {
            fat.push(u32_at(s, i * 4));
        }
    }
    Ok(fat)
}

/// The sectors of one chain, from `start` to the end-of-chain marker.
///
/// Bounded by the length of the FAT, because a chain cannot visit more sectors
/// than exist — which is also the cheapest way to refuse a file whose chain
/// loops back on itself rather than following it forever.
pub(crate) fn chain(fat: &[u32], start: u32) -> Result<Vec<u32>> {
    let mut out = Vec::new();
    let mut at = start;
    while at <= MAXREGSECT {
        if out.len() > fat.len() {
            return Err(Error::Malformed("a loop in a sector chain"));
        }
        out.push(at);
        let next = *fat
            .get(at as usize)
            .ok_or(Error::Malformed("a chain past the end of the FAT"))?;
        // FAT and DIFAT sectors are marked in the FAT itself. Meeting one while
        // following a stream's chain means the chain has walked into the
        // filesystem's own bookkeeping.
        if matches!(next, FATSECT | DIFSECT) {
            return Err(Error::Malformed("a chain running into the FAT"));
        }
        if next == FREESECT {
            return Err(Error::Malformed("a chain running into free space"));
        }
        at = next;
    }
    if at != ENDOFCHAIN && !out.is_empty() {
        return Err(Error::Malformed("a chain with no end"));
    }
    Ok(out)
}
