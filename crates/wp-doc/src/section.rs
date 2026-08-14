//! Page setup: the paper the document was written on.
//!
//! Without this every `.doc` opens as US Letter, and a document written on A4 —
//! which is most documents written outside North America — reflows on the first
//! line and paginates differently for the whole of its length. It is a small
//! amount of reading for a large amount of being wrong.
//!
//! The section table is a `Plcfsed`: character positions, then one entry each
//! saying where the section's properties are in `WordDocument`. Only the first
//! section is read — a `.doc` with several is laid out on the first one's paper,
//! which is stated rather than hidden.

use crate::fib::{u16 as read_u16, u32 as read_u32, Fib};
use crate::sprm;
use wp_model::section::SectionProps;
use wp_model::units::Twips;

/// Reads the first section's page setup, if the file states one.
pub fn read(fib: &Fib, table: &[u8], stream: &[u8]) -> Option<SectionProps> {
    let plc = fib.slice(table, crate::fib::field::PLCFSED)?;
    // n+1 character positions, then n twelve-byte entries.
    if plc.len() < 4 + 12 {
        return None;
    }
    let count = (plc.len() - 4) / 16;
    if count == 0 {
        return None;
    }
    let base = (count + 1) * 4;
    // Bytes 2..6 of an entry are the offset of the section's `Sepx`.
    let at = read_u32(plc, base + 2) as usize;
    // 0xFFFFFFFF means "no properties": the section is the default, which is
    // not the same as a section whose properties happen to be empty.
    if at == 0xFFFF_FFFF {
        return None;
    }
    let length = read_u16(stream, at) as usize;
    let grpprl = stream.get(at + 2..at + 2 + length)?;

    let mut props = SectionProps::default();
    apply(grpprl, &mut props).then_some(props)
}

/// Applies a section `grpprl`. `true` if it said anything about the paper.
fn apply(grpprl: &[u8], props: &mut SectionProps) -> bool {
    let mut stated = false;
    for found in sprm::walk(grpprl) {
        let value = || -> i32 {
            found
                .operand
                .get(..2)
                .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]) as i32)
                .unwrap_or(0)
        };
        match found.opcode {
            // Paper.
            0xB01F => {
                props.page.width = Twips(value());
                stated = true;
            }
            0xB020 => {
                props.page.height = Twips(value());
                stated = true;
            }
            // Margins. Left and right are unsigned; top and bottom are signed,
            // and a negative one means the header may run into the body.
            0xB021 => props.margins.start = Twips(value().abs()),
            0xB022 => props.margins.end = Twips(value().abs()),
            0x9023 => props.margins.top = Twips(value()),
            0x9024 => props.margins.bottom = Twips(value()),
            0xB017 => props.margins.header = Twips(value().abs()),
            0xB018 => props.margins.footer = Twips(value().abs()),
            0x301D => {
                // The width and height a `.doc` states are already the printed
                // ones, as they are in a `.docx`; the orientation is carried so
                // the page setup dialog and a later writer say the same thing.
                props.page.orientation = match found.operand.first().copied() {
                    Some(2) => wp_model::section::Orientation::Landscape,
                    _ => wp_model::section::Orientation::Portrait,
                };
                stated = true;
            }
            _ => {}
        }
    }
    stated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a4_is_read_rather_than_assumed_to_be_letter() {
        // 11906 by 16838 twips is A4, and it is what most of the world writes on.
        let grpprl = [
            0x1F, 0xB0, 0x82, 0x2E, // width 11906
            0x20, 0xB0, 0xC6, 0x41, // height 16838
            0x21, 0xB0, 0x40, 0x06, // left margin 1600
        ];
        let mut props = SectionProps::default();
        assert!(apply(&grpprl, &mut props));
        assert_eq!(props.page.width, Twips(11906));
        assert_eq!(props.page.height, Twips(16838));
        assert_eq!(props.margins.start, Twips(1600));
    }

    #[test]
    fn a_section_that_says_nothing_about_paper_leaves_the_default_alone() {
        // Otherwise every document that only sets, say, its columns would come
        // out on a page of zero by zero.
        let mut props = SectionProps::default();
        assert!(!apply(&[0x23, 0x90, 0x40, 0x06], &mut props));
        assert_eq!(props.margins.top, Twips(1600), "but the margin was read");
    }

    #[test]
    fn a_negative_top_margin_is_kept_as_it_is() {
        // A header that sits in the margin writes one, and clamping it to zero
        // moves the body down the page.
        let mut props = SectionProps::default();
        apply(&[0x23, 0x90, 0x9C, 0xFF], &mut props);
        assert_eq!(props.margins.top, Twips(-100));
    }
}
