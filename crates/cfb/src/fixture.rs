//! Builds compound files, for tests.
//!
//! This is **not** a writer and is not a step towards one. It exists because
//! the only honest way to test a reader of a binary format, on a machine with
//! no Excel and no downloaded corpus, is to lay the bytes out from the
//! specification and then read them back with entirely separate code. It
//! allocates naively, never updates a file in place, and builds the directory
//! as a right-leaning list rather than the red-black tree a real writer must
//! maintain — deliberately, because a list is the shape most likely to catch a
//! reader that recurses without a bound.
//!
//! Version 3 only: 512-byte sectors, 64-byte mini sectors, 4096-byte cutoff.

use crate::header::{DIFSECT, ENDOFCHAIN, FATSECT, FREESECT, SIGNATURE};

const SECTOR: usize = 512;
const MINI: usize = 64;
const CUTOFF: usize = 4096;
const PER_SECTOR: usize = SECTOR / 4;
const DIFAT_IN_HEADER: usize = 109;
const NOSTREAM: u32 = 0xFFFF_FFFF;

/// A file being assembled: root-level streams, in the order they were added.
#[derive(Default)]
pub struct Builder {
    streams: Vec<(String, Vec<u8>)>,
}

impl Builder {
    pub fn new() -> Builder {
        Builder::default()
    }

    pub fn stream(mut self, name: &str, data: impl Into<Vec<u8>>) -> Builder {
        self.streams.push((name.to_owned(), data.into()));
        self
    }

    pub fn build(self) -> Vec<u8> {
        let mut mini = Vec::new();
        let mut mini_fat: Vec<u32> = Vec::new();
        // Where each stream starts, and in which of the two allocation schemes.
        let mut placed: Vec<(u32, u64)> = Vec::new();
        let mut large: Vec<&[u8]> = Vec::new();
        // Which entries are waiting for a large-stream address. An in-band
        // marker would have to be a `u32`, and every `u32` that is not a
        // sector number already means something else.
        let mut waiting: Vec<usize> = Vec::new();

        for (_, data) in &self.streams {
            if data.is_empty() {
                placed.push((ENDOFCHAIN, 0));
            } else if data.len() < CUTOFF {
                let first = (mini.len() / MINI) as u32;
                mini.extend_from_slice(data);
                pad_to(&mut mini, MINI);
                let count = mini.len() / MINI - first as usize;
                for i in 0..count {
                    mini_fat.push(if i + 1 == count {
                        ENDOFCHAIN
                    } else {
                        first + i as u32 + 1
                    });
                }
                placed.push((first, data.len() as u64));
            } else {
                waiting.push(placed.len());
                placed.push((FREESECT, data.len() as u64)); // patched below
                large.push(data);
            }
        }

        let dir_sectors = sectors_for((self.streams.len() + 1) * 128);
        let mini_fat_sectors = sectors_for(mini_fat.len() * 4);
        let mini_sectors = sectors_for(mini.len());
        let large_sectors: Vec<usize> = large.iter().map(|d| sectors_for(d.len())).collect();
        let content: usize =
            dir_sectors + mini_fat_sectors + mini_sectors + large_sectors.iter().sum::<usize>();

        // The FAT has to describe itself, so its size is a fixpoint: more FAT
        // sectors need more FAT entries, and past 109 they need DIFAT sectors
        // which need entries of their own.
        let (fat_sectors, difat_sectors) = {
            let (mut f, mut d) = (0usize, 0usize);
            loop {
                let total = content + f + d;
                let nf = total.div_ceil(PER_SECTOR).max(1);
                let nd = nf.saturating_sub(DIFAT_IN_HEADER).div_ceil(PER_SECTOR - 1);
                if (nf, nd) == (f, d) {
                    break (f, d);
                }
                (f, d) = (nf, nd);
            }
        };

        let mut next = 0u32;
        let mut take = |n: usize| {
            let at = next;
            next += n as u32;
            at
        };
        let dir_at = take(dir_sectors);
        let mini_fat_at = take(mini_fat_sectors);
        let mini_at = take(mini_sectors);
        let large_at: Vec<u32> = large_sectors.iter().map(|n| take(*n)).collect();
        let difat_at = take(difat_sectors);
        let fat_at = take(fat_sectors);
        let total = next as usize;

        let mut fat = vec![FREESECT; fat_sectors * PER_SECTOR];
        let mut run = |start: u32, count: usize| {
            for i in 0..count {
                fat[start as usize + i] = if i + 1 == count {
                    ENDOFCHAIN
                } else {
                    start + i as u32 + 1
                };
            }
        };
        run(dir_at, dir_sectors);
        run(mini_fat_at, mini_fat_sectors);
        run(mini_at, mini_sectors);
        for (at, count) in large_at.iter().zip(&large_sectors) {
            run(*at, *count);
        }
        for i in 0..difat_sectors {
            fat[difat_at as usize + i] = DIFSECT;
        }
        for i in 0..fat_sectors {
            fat[fat_at as usize + i] = FATSECT;
        }

        // Now that the large streams have addresses, the placements can be
        // finished. They were queued in order, so they match `large_at`.
        for (slot, at) in waiting.iter().zip(&large_at) {
            placed[*slot].0 = *at;
        }

        let mut out = vec![0u8; SECTOR * (total + 1)];

        // Header.
        out[..8].copy_from_slice(&SIGNATURE);
        put16(&mut out, 24, 0x003E); // minor version, as Excel writes it
        put16(&mut out, 26, 3);
        put16(&mut out, 28, 0xFFFE);
        put16(&mut out, 30, 9);
        put16(&mut out, 32, 6);
        put32(&mut out, 44, fat_sectors as u32);
        put32(&mut out, 48, dir_at);
        put32(&mut out, 56, CUTOFF as u32);
        put32(
            &mut out,
            60,
            if mini_fat_sectors > 0 {
                mini_fat_at
            } else {
                ENDOFCHAIN
            },
        );
        put32(&mut out, 64, mini_fat_sectors as u32);
        put32(
            &mut out,
            68,
            if difat_sectors > 0 {
                difat_at
            } else {
                ENDOFCHAIN
            },
        );
        put32(&mut out, 72, difat_sectors as u32);
        for i in 0..DIFAT_IN_HEADER {
            let value = if i < fat_sectors {
                fat_at + i as u32
            } else {
                FREESECT
            };
            put32(&mut out, 76 + i * 4, value);
        }

        // The DIFAT chain, for the FAT sectors past the 109th.
        for i in 0..difat_sectors {
            let base = (difat_at as usize + i + 1) * SECTOR;
            for slot in 0..PER_SECTOR - 1 {
                let which = DIFAT_IN_HEADER + i * (PER_SECTOR - 1) + slot;
                let value = if which < fat_sectors {
                    fat_at + which as u32
                } else {
                    FREESECT
                };
                put32(&mut out, base + slot * 4, value);
            }
            let next = if i + 1 < difat_sectors {
                difat_at + i as u32 + 1
            } else {
                ENDOFCHAIN
            };
            put32(&mut out, base + (PER_SECTOR - 1) * 4, next);
        }

        // The directory: the root, then one entry per stream, linked right.
        let mut dir = vec![0u8; (self.streams.len() + 1) * 128];
        write_entry(
            &mut dir,
            0,
            "Root Entry",
            5,
            if mini.is_empty() { ENDOFCHAIN } else { mini_at },
            mini.len() as u64,
            if self.streams.is_empty() { NOSTREAM } else { 1 },
            NOSTREAM,
        );
        for (i, (name, _)) in self.streams.iter().enumerate() {
            let (start, size) = placed[i];
            let right = if i + 2 <= self.streams.len() {
                i as u32 + 2
            } else {
                NOSTREAM
            };
            write_entry(&mut dir, i + 1, name, 2, start, size, NOSTREAM, right);
        }
        blit(&mut out, dir_at, &dir);

        let mut mini_fat_bytes = Vec::with_capacity(mini_fat.len() * 4);
        for entry in &mini_fat {
            mini_fat_bytes.extend_from_slice(&entry.to_le_bytes());
        }
        pad_to(&mut mini_fat_bytes, SECTOR);
        for slot in mini_fat_bytes.chunks_exact_mut(4).skip(mini_fat.len()) {
            slot.copy_from_slice(&FREESECT.to_le_bytes());
        }
        blit(&mut out, mini_fat_at, &mini_fat_bytes);
        blit(&mut out, mini_at, &mini);
        for (data, at) in large.iter().zip(&large_at) {
            blit(&mut out, *at, data);
        }

        let mut fat_bytes = Vec::with_capacity(fat.len() * 4);
        for entry in &fat {
            fat_bytes.extend_from_slice(&entry.to_le_bytes());
        }
        blit(&mut out, fat_at, &fat_bytes);

        out
    }
}

fn sectors_for(bytes: usize) -> usize {
    bytes.div_ceil(SECTOR)
}

fn pad_to(data: &mut Vec<u8>, unit: usize) {
    let over = data.len() % unit;
    if over != 0 {
        data.resize(data.len() + unit - over, 0);
    }
}

fn blit(out: &mut [u8], sector: u32, data: &[u8]) {
    let at = (sector as usize + 1) * SECTOR;
    out[at..at + data.len()].copy_from_slice(data);
}

fn put16(out: &mut [u8], at: usize, v: u16) {
    out[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

fn put32(out: &mut [u8], at: usize, v: u32) {
    out[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

#[allow(clippy::too_many_arguments)]
fn write_entry(
    dir: &mut [u8],
    index: usize,
    name: &str,
    kind: u8,
    start: u32,
    size: u64,
    child: u32,
    right: u32,
) {
    let at = index * 128;
    let units: Vec<u16> = name.encode_utf16().collect();
    for (i, unit) in units.iter().enumerate() {
        put16(dir, at + i * 2, *unit);
    }
    // The length counts the terminating NUL, and counts bytes.
    put16(dir, at + 64, (units.len() as u16 + 1) * 2);
    dir[at + 66] = kind;
    dir[at + 67] = 1; // black
    put32(dir, at + 68, NOSTREAM); // left
    put32(dir, at + 72, right);
    put32(dir, at + 76, child);
    put32(dir, at + 116, start);
    dir[at + 120..at + 128].copy_from_slice(&size.to_le_bytes());
}
