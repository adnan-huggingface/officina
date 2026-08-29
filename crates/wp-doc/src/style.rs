//! The stylesheet: what the style indices in the rest of the file mean.
//!
//! Every paragraph and every run in a `.doc` names its style by number — an
//! `istd`, an index into this table. Without it a document says "style 7" and
//! nothing more, and a heading is indistinguishable from body text.
//!
//! **A style's own formatting is a variant record.** What follows the name is a
//! `grLPUpxSw` whose members depend on which kind of style it is: a paragraph
//! style holds its paragraph properties and then its character ones, a
//! character style holds only the second, and a table or numbering style holds
//! a shape this does not read. Each member is a counted `grpprl` padded to an
//! even length, and a reader that forgets the padding walks into the middle of
//! the next one and writes a face onto a style that never asked for it.
//!
//! **A style stands on the one it is based on.** `istdBase` is another index
//! into this same table, forward as often as back, so the chain can only be
//! joined up once every entry has been read.

use crate::fib::{u16 as read_u16, Fib};
use crate::{fkp, sprm};
use wp_model::style::{Style, StyleKind, StyleTable};

/// Reads the style names into a table, indexed the way the file indexes them.
///
/// The returned vector maps `istd` to the interned id, because the two numbering
/// schemes are not the same and confusing them silently mis-styles a document.
pub fn read(
    fib: &Fib,
    table: &[u8],
    fonts: &[std::sync::Arc<str>],
) -> (StyleTable, Vec<Option<wp_model::style::StyleId>>) {
    let mut styles = StyleTable::new();
    let mut by_istd = Vec::new();
    // Where each style says it stands, kept until every index has a style to
    // point at. `NIL` means the style stands on nothing.
    let mut bases: Vec<u16> = Vec::new();
    let Some(stsh) = fib.slice(table, crate::fib::field::STSHF) else {
        return (styles, by_istd);
    };
    let header = read_u16(stsh, 0) as usize;
    let count = read_u16(stsh, 2) as usize;
    // How many bytes of the fixed part of each entry this file writes. Word 97
    // writes ten; a later version may write more, and the name is after it.
    let base = read_u16(stsh, 4) as usize;

    // `STSHI.rgftcStandardChpStsh` — the three faces (Latin, East Asian,
    // complex) a run falls back to when neither it nor its style names one.
    // A `.doc` has no `docDefaults`; this is where the same answer lives, and
    // without it every document Word wrote after 2007 was laid in Times New
    // Roman rather than in the Calibri it says here.
    if header >= 14 {
        let standard = |at: usize| {
            fonts
                .get(read_u16(stsh, at) as usize)
                .filter(|name| !name.is_empty())
                .cloned()
        };
        let mut defaults = wp_model::style::DocDefaults::default();
        let latin = standard(14);
        defaults.run.fonts.ascii = latin.clone();
        defaults.run.fonts.high_ansi = latin;
        defaults.run.fonts.east_asian = standard(16);
        defaults.run.fonts.complex = standard(18);
        styles.set_doc_defaults(defaults);
    }

    let mut at = 2 + header;
    for _ in 0..count {
        let length = read_u16(stsh, at) as usize;
        let entry = match stsh.get(at + 2..at + 2 + length) {
            Some(entry) => entry,
            None => break,
        };
        at += 2 + length;
        // A zero-length entry is a hole: the index is used up but there is no
        // style there, which is why this pushes `None` rather than skipping.
        if length == 0 || entry.len() < base + 2 {
            by_istd.push(None);
            bases.push(NIL);
            continue;
        }
        // Bits 4..16 of the second word are the style group: paragraph,
        // character, table or numbering.
        let kind = match (read_u16(entry, 2) & 0x000F) as u8 {
            2 => StyleKind::Character,
            3 => StyleKind::Table,
            4 => StyleKind::Numbering,
            _ => StyleKind::Paragraph,
        };
        let name = name_at(entry, base);
        if name.is_empty() {
            by_istd.push(None);
            bases.push(NIL);
            continue;
        }
        let id = styles.intern(&name, kind);
        let mut style = Style::new(name.as_str(), kind);
        style.name = Some(name.into());
        // The paragraph half comes first for a paragraph style and is absent
        // for a character one; the character half follows either way.
        let (para, run) = match kind {
            StyleKind::Paragraph => {
                let para = upx(entry, base, 0);
                (para, upx(entry, base, 1))
            }
            StyleKind::Character => (None, upx(entry, base, 0)),
            _ => (None, None),
        };
        if let Some(grpprl) = para {
            // A `UpxPapx` begins with the style's own index, which is not a
            // property; `split_istd` is the same reader a direct exception uses.
            let (_, grpprl) = fkp::split_istd(grpprl);
            sprm::apply_para(&mut style.para, grpprl);
        }
        if let Some(grpprl) = run {
            sprm::apply_run(&mut style.run, grpprl, fonts);
        }
        styles.insert(style);
        by_istd.push(Some(id));
        bases.push(read_u16(entry, 2) >> 4);
    }

    // Now that every index has a style, the chain can be joined up. A style
    // based on itself, or on an index with no style, stands on nothing.
    for (istd, base) in bases.iter().enumerate() {
        let (Some(Some(id)), Some(Some(parent))) = (by_istd.get(istd), by_istd.get(*base as usize))
        else {
            continue;
        };
        if id != parent {
            if let Some(style) = styles.get_mut(*id) {
                style.based_on = Some(*parent);
            }
        }
    }
    (styles, by_istd)
}

/// `istdNil` — what a style that stands on nothing says its base is.
const NIL: u16 = 0x0FFF;

/// An `Xstz`: a count of characters, then that many UTF-16 units, then a
/// terminating zero that is not counted.
fn name_at(entry: &[u8], at: usize) -> String {
    let count = read_u16(entry, at) as usize;
    let start = at + 2;
    let Some(bytes) = entry.get(start..start + count * 2) else {
        return String::new();
    };
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

/// The `nth` counted `grpprl` of a style's `grLPUpxSw`, the tail of the `STD`
/// entry that follows the name.
///
/// **Each member is padded to an even length**, and the padding is not counted
/// by the length in front of it. Stepping by the stated length alone lands one
/// byte early on the next member, whose count is then read out of the middle of
/// a property.
///
/// `name_at` (used to build the style's name) deliberately does not count the
/// name's own terminating zero, so `grLPUpxSw` starts two bytes further on than
/// the name's counted length alone would suggest.
fn upx(entry: &[u8], base: usize, nth: usize) -> Option<&[u8]> {
    let count = read_u16(entry, base) as usize;
    let mut start = base + 2 + count * 2 + 2;
    for _ in 0..nth {
        let length = read_u16(entry, start) as usize;
        start = start + 2 + length + length % 2;
    }
    let length = read_u16(entry, start) as usize;
    entry.get(start + 2..start + 2 + length)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_counted_in_characters_and_terminated_as_well() {
        let mut entry = vec![0u8; 10];
        entry.extend_from_slice(&3u16.to_le_bytes());
        for character in "One".encode_utf16() {
            entry.extend_from_slice(&character.to_le_bytes());
        }
        entry.extend_from_slice(&0u16.to_le_bytes());
        assert_eq!(name_at(&entry, 10), "One");
    }

    #[test]
    fn a_paragraph_styles_own_tab_stop_comes_out_of_its_papx() {
        // The STD entry: stdfBase (10 bytes, contents irrelevant here — this
        // helper is only ever called for a style already known to be a
        // paragraph style), then the name, then grLPUpxSw's one LPUpxPapx.
        let mut entry = vec![0u8; 10];
        entry.extend_from_slice(&3u16.to_le_bytes());
        for character in "One".encode_utf16() {
            entry.extend_from_slice(&character.to_le_bytes());
        }
        entry.extend_from_slice(&0u16.to_le_bytes()); // uncounted terminator

        // UpxPapx: istd(2) + grpprlPapx. The grpprl is one sprmPChgTabsPapx:
        // cb=4, one delete at 1440 twips (0x05A0 LE), no adds.
        let mut upx_papx = vec![0x00, 0x00];
        upx_papx.extend([0x0D, 0xC6, 0x04, 0x01, 0xA0, 0x05, 0x00]);
        entry.extend_from_slice(&(upx_papx.len() as u16).to_le_bytes());
        entry.extend_from_slice(&upx_papx);

        let papx = upx(&entry, 10, 0).expect("a papx follows the name");
        let (_, grpprl) = fkp::split_istd(papx);
        let mut props = wp_model::prop::ParaProps::default();
        sprm::apply_para(&mut props, grpprl);
        assert_eq!(
            props.tabs,
            Some(vec![wp_model::prop::TabStop {
                position: wp_model::units::Twips(1440),
                kind: wp_model::prop::TabKind::Clear,
                leader: wp_model::prop::TabLeader::None,
            }])
        );
    }

    #[test]
    fn the_character_half_is_found_past_the_padding_the_first_half_does_not_count() {
        // An odd-length UpxPapx is followed by a byte of padding that its own
        // count does not include. Stepping by the count alone reads the last
        // byte of the papx as the length of the chpx.
        let mut entry = vec![0u8; 10];
        entry.extend_from_slice(&1u16.to_le_bytes());
        entry.extend_from_slice(&0x41u16.to_le_bytes()); // "A"
        entry.extend_from_slice(&0u16.to_le_bytes()); // uncounted terminator

        // istd(2) + sprmPJc (0x2403), which makes the whole thing odd.
        let papx = [0x00u8, 0x00, 0x03, 0x24, 0x02];
        entry.extend_from_slice(&(papx.len() as u16).to_le_bytes());
        entry.extend_from_slice(&papx);
        entry.push(0); // the padding

        // sprmCHps (0x4A43): sixteen half-points.
        let chpx = [0x43u8, 0x4A, 0x10, 0x00];
        entry.extend_from_slice(&(chpx.len() as u16).to_le_bytes());
        entry.extend_from_slice(&chpx);

        let mut props = wp_model::prop::RunProps::default();
        sprm::apply_run(&mut props, upx(&entry, 10, 1).expect("a chpx"), &[]);
        assert_eq!(props.size, Some(wp_model::units::HalfPoint(16)));
    }

    #[test]
    fn a_name_that_runs_past_the_entry_is_empty_rather_than_a_panic() {
        let entry = vec![0u8; 12];
        let mut short = entry.clone();
        short[10] = 200;
        assert_eq!(name_at(&short, 10), "");
    }
}
