//! The stylesheet: what the style indices in the rest of the file mean.
//!
//! Every paragraph and every run in a `.doc` names its style by number — an
//! `istd`, an index into this table. Without it a document says "style 7" and
//! nothing more, and a heading is indistinguishable from body text.
//!
//! **Only the names and the shape of the chain are read here.** A style's own
//! formatting is a `grpprl` in a variant record whose layout depends on a count
//! of "upxes" that differs between paragraph, character and table styles, and
//! getting it wrong writes properties onto the wrong style. So the definitions
//! are left out and said to be left out: a `.doc` opened here knows *which*
//! style each paragraph is in, and takes its formatting from the direct
//! properties the file states alongside.

use crate::fib::{u16 as read_u16, Fib};
use crate::{fkp, sprm};
use wp_model::style::{Style, StyleKind, StyleTable};

/// Reads the style names into a table, indexed the way the file indexes them.
///
/// The returned vector maps `istd` to the interned id, because the two numbering
/// schemes are not the same and confusing them silently mis-styles a document.
pub fn read(fib: &Fib, table: &[u8]) -> (StyleTable, Vec<Option<wp_model::style::StyleId>>) {
    let mut styles = StyleTable::new();
    let mut by_istd = Vec::new();
    let Some(stsh) = fib.slice(table, crate::fib::field::STSHF) else {
        return (styles, by_istd);
    };
    let header = read_u16(stsh, 0) as usize;
    let count = read_u16(stsh, 2) as usize;
    // How many bytes of the fixed part of each entry this file writes. Word 97
    // writes ten; a later version may write more, and the name is after it.
    let base = read_u16(stsh, 4) as usize;

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
            continue;
        }
        let id = styles.intern(&name, kind);
        let mut style = Style::new(name.as_str(), kind);
        style.name = Some(name.into());
        if kind == StyleKind::Paragraph {
            if let Some(grpprl) = para_upx(entry, base) {
                sprm::apply_para(&mut style.para, grpprl);
            }
        }
        styles.insert(style);
        by_istd.push(Some(id));
    }
    (styles, by_istd)
}

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

/// A paragraph style's own formatting: the `grpprlPapx` inside the first
/// `LPUpxPapx` of its `grLPUpxSw`, the tail of the `STD` entry that follows
/// the name. `StkParaGRLPUPX` guarantees `lpUpxPapx` is always the first
/// member for a paragraph style, regardless of `cupx` (2, or 3 if the style
/// is revision-marked) — so no branching on `cupx` is needed to find it.
///
/// `name_at` (used to build the style's name) deliberately does not count
/// the name's own terminating zero, so `grLPUpxSw` starts two bytes further
/// on than the name's counted length alone would suggest.
fn para_upx(entry: &[u8], base: usize) -> Option<&[u8]> {
    let count = read_u16(entry, base) as usize;
    let start = base + 2 + count * 2 + 2;
    let cb_upx = read_u16(entry, start) as usize;
    // UpxPapx is `istd`(2) + `grpprlPapx` — the same shape `split_istd`
    // already parses for a direct-formatting PAPX exception.
    let papx = entry.get(start + 2..start + 2 + cb_upx)?;
    let (_, grpprl) = fkp::split_istd(papx);
    Some(grpprl)
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

        let grpprl = para_upx(&entry, 10).expect("a papx follows the name");
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
    fn a_name_that_runs_past_the_entry_is_empty_rather_than_a_panic() {
        let entry = vec![0u8; 12];
        let mut short = entry.clone();
        short[10] = 200;
        assert_eq!(name_at(&short, 10), "");
    }
}
