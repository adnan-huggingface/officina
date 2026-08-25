//! The font table: what a `ftc` is the name of.
//!
//! Nothing in a `.doc` names a face. A run says `sprmCRgFtc0 = 3` and means
//! whatever the fourth entry of `SttbfFfn` says, so a reader that skips this
//! table renders an entire document in whatever face it falls back to — which
//! is a different document, not a plainer one.

use crate::fib::{u16 as read_u16, Fib};
use std::sync::Arc;

/// Where the name starts inside one `FFN`, counted from its own first byte.
///
/// That first byte is `cbFfnM1`, which is also the string table's own count of
/// the entry, so the record is kept whole rather than sliced past it — the two
/// readings of the same byte are what an off-by-one here would confuse.
///
/// The header between is the same size for every version this crate opens. Word
/// 6 and 95 wrote a shorter one, and are refused before this is ever reached.
const NAME_AT: usize = 40;

/// Reads the font names, indexed the way the file indexes them.
///
/// An entry that cannot be read is kept as an empty name rather than dropped:
/// the index is the identity, so a hole must still take up its place.
pub fn read(fib: &Fib, table: &[u8]) -> Vec<Arc<str>> {
    fib.slice(table, crate::fib::field::STTBF_FFN)
        .map(walk)
        .unwrap_or_default()
}

/// The names in one `SttbfFfn`.
fn walk(sttb: &[u8]) -> Vec<Arc<str>> {
    // `SttbfFfn` is a plain STTB whose strings are counted in bytes, but Word
    // has written the extended marker in front of one before now, and reading
    // the marker as a count gives sixty-five thousand fonts.
    let at = match read_u16(sttb, 0) == 0xFFFF {
        true => 2,
        false => 0,
    };
    let count = read_u16(sttb, at) as usize;
    let mut names = Vec::with_capacity(count.min(1024));
    // `cbExtra` follows the count and is zero for this table, but the two bytes
    // are there either way.
    let mut cursor = at + 4;
    for _ in 0..count {
        let Some(&length) = sttb.get(cursor) else {
            break;
        };
        let Some(entry) = sttb.get(cursor..cursor + 1 + length as usize) else {
            break;
        };
        cursor += 1 + length as usize;
        names.push(name_of(entry).into());
    }
    names
}

/// The name inside one `FFN`, which is UTF-16 and ends at the first zero.
///
/// An alternate name may follow the first, for a font the document asks to be
/// substituted; only the name the runs mean is read, and the terminator is what
/// separates the two.
fn name_of(entry: &[u8]) -> String {
    let Some(bytes) = entry.get(NAME_AT..) else {
        return String::new();
    };
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names, as plain strings, because an `Arc<str>` does not compare
    /// against a literal.
    fn named(sttb: &[u8]) -> Vec<String> {
        walk(sttb).iter().map(|name| name.to_string()).collect()
    }

    /// One `FFN`: its own length, a header this does not read, and the name.
    fn ffn(names: &[&str]) -> Vec<u8> {
        let mut entry = vec![0u8; NAME_AT];
        for name in names {
            for unit in name.encode_utf16() {
                entry.extend_from_slice(&unit.to_le_bytes());
            }
            entry.extend_from_slice(&0u16.to_le_bytes());
        }
        entry[0] = (entry.len() - 1) as u8;
        entry
    }

    #[test]
    fn a_name_ends_at_its_terminator_and_not_at_the_entry() {
        // An alternate name follows the first inside the same entry, for a face
        // the document asks to be substituted; reading to the end of the entry
        // glues the two together into one face nobody has.
        assert_eq!(name_of(&ffn(&["Arial", "Helvetica"])), "Arial");
    }

    #[test]
    fn the_entries_come_out_in_the_order_the_indices_count_them() {
        let mut sttb = Vec::new();
        sttb.extend_from_slice(&2u16.to_le_bytes());
        sttb.extend_from_slice(&0u16.to_le_bytes());
        sttb.extend(ffn(&["Times New Roman"]));
        sttb.extend(ffn(&["Arial"]));
        assert_eq!(named(&sttb), ["Times New Roman", "Arial"]);
    }

    #[test]
    fn the_extended_marker_is_a_marker_and_not_a_count_of_sixty_five_thousand() {
        let mut sttb = vec![0xFF, 0xFF];
        sttb.extend_from_slice(&1u16.to_le_bytes());
        sttb.extend_from_slice(&0u16.to_le_bytes());
        sttb.extend(ffn(&["Arial"]));
        assert_eq!(named(&sttb), ["Arial"]);
    }

    #[test]
    fn an_entry_that_runs_past_the_table_stops_the_walk_rather_than_panicking() {
        let mut sttb = Vec::new();
        sttb.extend_from_slice(&2u16.to_le_bytes());
        sttb.extend_from_slice(&0u16.to_le_bytes());
        sttb.extend(ffn(&["Arial"]));
        sttb.push(200);
        assert_eq!(named(&sttb), ["Arial"]);
        assert_eq!(name_of(&[0u8; 4]), "");
    }
}
