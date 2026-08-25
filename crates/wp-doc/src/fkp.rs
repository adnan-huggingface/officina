//! The bin tables: how a `.doc` says which formatting applies where.
//!
//! Formatting is not stored with the text. It is in 512-byte pages —
//! *formatted disk pages*, a name that gives away what a `.doc` is: a paged
//! memory image written to disk. Each page holds a sorted list of byte offsets
//! into `WordDocument` and, at the *back* of the page growing forwards, the
//! property lists themselves. The last byte of the page says how many runs it
//! holds, and everything else is found from that.
//!
//! To find the formatting of a character: its character position → its byte
//! offset (through the piece table) → the page (through the bin table) → the run
//! within the page → the property list. Four indirections, and the only one that
//! is obvious from the file is the last.

use crate::fib::{u32 as read_u32, Fib};
use crate::sprm;

/// The size of one page, which is not negotiable anywhere in the format.
const PAGE: usize = 512;

/// A run of bytes that share formatting, and the properties they share.
#[derive(Debug, Clone)]
pub struct Exception {
    /// Byte offsets into the `WordDocument` stream.
    pub from: usize,
    pub to: usize,
    /// The `grpprl` — a list of sprms, unparsed, because what it means depends
    /// on whether these are character or paragraph properties.
    pub grpprl: Vec<u8>,
}

/// Every character-property exception in the document, in file order.
pub fn characters(fib: &Fib, table: &[u8], stream: &[u8]) -> Vec<Exception> {
    pages(
        fib,
        table,
        stream,
        crate::fib::field::PLCFBTE_CHPX,
        chpx_page,
    )
}

/// Every paragraph-property exception.
pub fn paragraphs(fib: &Fib, table: &[u8], stream: &[u8]) -> Vec<Exception> {
    pages(
        fib,
        table,
        stream,
        crate::fib::field::PLCFBTE_PAPX,
        papx_page,
    )
}

/// Walks the bin table and reads each page it names.
fn pages(
    fib: &Fib,
    table: &[u8],
    stream: &[u8],
    field: usize,
    read: fn(&[u8]) -> Vec<Exception>,
) -> Vec<Exception> {
    let Some(plc) = fib.slice(table, field) else {
        return Vec::new();
    };
    // A `Plcfbte` is n+1 byte offsets then n page numbers, four bytes each.
    if plc.len() < 8 {
        return Vec::new();
    }
    let count = (plc.len() - 4) / 8;
    let base = (count + 1) * 4;
    let mut out = Vec::new();
    for index in 0..count {
        // Only the low 22 bits are the page number; the rest is a flag word in
        // some versions and reserved in others.
        let page = (read_u32(plc, base + index * 4) & 0x003F_FFFF) as usize;
        let at = page * PAGE;
        let Some(bytes) = stream.get(at..at + PAGE) else {
            continue;
        };
        out.extend(read(bytes));
    }
    out.sort_by_key(|exception| exception.from);
    out
}

/// A character-property page.
///
/// `rgfc[crun + 1]` offsets, then one byte per run saying where in the page its
/// property list is — halved, because a page is 512 bytes and a byte only
/// reaches 255.
fn chpx_page(page: &[u8]) -> Vec<Exception> {
    let count = page[PAGE - 1] as usize;
    let mut out = Vec::new();
    for index in 0..count {
        let from = read_u32(page, index * 4) as usize;
        let to = read_u32(page, (index + 1) * 4) as usize;
        let word = page[(count + 1) * 4 + index] as usize * 2;
        let grpprl = match word {
            // Zero means "no exception here": the run is whatever its style says.
            0 => Vec::new(),
            at => {
                let length = *page.get(at).unwrap_or(&0) as usize;
                page.get(at + 1..at + 1 + length)
                    .unwrap_or_default()
                    .to_vec()
            }
        };
        out.push(Exception { from, to, grpprl });
    }
    out
}

/// A paragraph-property page.
///
/// Thirteen bytes per run rather than one, and the property list has *two*
/// spellings of its own length: a byte count that is doubled, or — when that
/// byte is zero, because the list is longer than 254 bytes — the next byte
/// instead. The first two bytes of the list are the style index, not a sprm.
fn papx_page(page: &[u8]) -> Vec<Exception> {
    let count = page[PAGE - 1] as usize;
    let mut out = Vec::new();
    for index in 0..count {
        let from = read_u32(page, index * 4) as usize;
        let to = read_u32(page, (index + 1) * 4) as usize;
        let at = *page.get((count + 1) * 4 + index * 13).unwrap_or(&0) as usize * 2;
        let grpprl = match at {
            0 => Vec::new(),
            at => {
                let first = *page.get(at).unwrap_or(&0) as usize;
                let (start, length) = match first {
                    0 => (at + 2, *page.get(at + 1).unwrap_or(&0) as usize * 2),
                    // The count is in words and does not include its own byte.
                    cb => (at + 1, cb * 2 - 1),
                };
                page.get(start..start + length).unwrap_or_default().to_vec()
            }
        };
        out.push(Exception { from, to, grpprl });
    }
    out
}

/// The style index at the front of a paragraph exception, and the sprms after it.
///
/// A `PapxInFkp` begins with a two-byte `istd`. Reading it as a sprm gives a
/// property nobody asked for and loses the style.
pub fn split_istd(grpprl: &[u8]) -> (Option<u16>, &[u8]) {
    match grpprl.len() >= 2 {
        true => (
            Some(u16::from_le_bytes([grpprl[0], grpprl[1]])),
            &grpprl[2..],
        ),
        false => (None, grpprl),
    }
}

/// The exception covering a byte offset, if there is one.
pub fn at(exceptions: &[Exception], offset: usize) -> Option<&Exception> {
    let index = exceptions
        .binary_search_by(|exception| match () {
            _ if exception.to <= offset => std::cmp::Ordering::Less,
            _ if exception.from > offset => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        })
        .ok()?;
    exceptions.get(index)
}

/// Applies a paragraph exception, style index first.
pub fn para_props(exception: Option<&Exception>) -> (Option<u16>, wp_model::prop::ParaProps) {
    let mut props = wp_model::prop::ParaProps::default();
    let Some(exception) = exception else {
        return (None, props);
    };
    let (istd, rest) = split_istd(&exception.grpprl);
    sprm::apply_para(&mut props, rest);
    (istd, props)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a character page with one run whose properties say "bold".
    fn page() -> Vec<u8> {
        let mut page = vec![0u8; PAGE];
        page[0..4].copy_from_slice(&1000u32.to_le_bytes());
        page[4..8].copy_from_slice(&1100u32.to_le_bytes());
        // The property list lives at byte 100, so the run's word offset is 50.
        page[8] = 50;
        page[100] = 3;
        page[101..104].copy_from_slice(&[0x35, 0x08, 0x01]);
        page[PAGE - 1] = 1;
        page
    }

    #[test]
    fn a_run_finds_its_properties_at_the_back_of_the_page() {
        let exceptions = chpx_page(&page());
        assert_eq!(exceptions.len(), 1);
        assert_eq!((exceptions[0].from, exceptions[0].to), (1000, 1100));
        let mut props = wp_model::prop::RunProps::default();
        sprm::apply_run(&mut props, &exceptions[0].grpprl, &[]);
        assert_eq!(props.toggles.get(wp_model::prop::Toggle::Bold), Some(true));
    }

    #[test]
    fn a_run_with_no_exception_is_whatever_its_style_says() {
        // A zero offset is not an offset to a zero-length list; it means there
        // is nothing here at all.
        let mut page = page();
        page[8] = 0;
        assert!(chpx_page(&page)[0].grpprl.is_empty());
    }

    #[test]
    fn the_exception_covering_an_offset_is_found_by_range() {
        let exceptions = vec![
            Exception {
                from: 0,
                to: 100,
                grpprl: vec![1],
            },
            Exception {
                from: 100,
                to: 200,
                grpprl: vec![2],
            },
        ];
        assert_eq!(at(&exceptions, 150).map(|e| e.grpprl[0]), Some(2));
        assert_eq!(at(&exceptions, 99).map(|e| e.grpprl[0]), Some(1));
        assert!(at(&exceptions, 500).is_none());
    }

    #[test]
    fn a_paragraph_exception_begins_with_its_style_rather_than_a_property() {
        let (istd, rest) = split_istd(&[0x07, 0x00, 0x03, 0x24, 0x01]);
        assert_eq!(istd, Some(7));
        assert_eq!(rest, &[0x03, 0x24, 0x01]);
    }
}
