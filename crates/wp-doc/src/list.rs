//! The lists a `.doc` numbers its paragraphs from.
//!
//! Two tables in the table stream, and a paragraph reaches its numbering
//! through both. `PlfLst` holds the list *definitions* — one `LSTF` per list,
//! and after the whole array of them, the levels themselves, nine to a list or
//! one if the list says it is simple. `PlfLfo` holds the list *instances*: a
//! layer of indirection whose only job is to let two places in a document
//! number from the same definition and still restart independently. A
//! paragraph names an instance by `sprmPIlfo` and a level by `sprmPIlvl`, and
//! the pair is exactly what `<w:numPr>` says in the modern format.
//!
//! The one trap is that `lcbPlfLst` does not cover the levels: the count in the
//! FIB stops at the end of the `LSTF` array and the `LVL`s run on past it, so
//! the levels have to be read from the table stream itself rather than from the
//! slice the FIB describes. [MS-DOC] §2.9.164 says so in a sentence that is
//! easy to read straight past.
//!
//! Structures are [MS-DOC] §2.9.164 (`PlfLst`), §2.9.131 (`LSTF`), §2.9.129
//! (`LVL`), §2.9.130 (`LVLF`), §2.9.163 (`PlfLfo`), §2.9.126 (`LFO`), §2.9.127
//! (`LFOData`) and §2.9.128 (`LFOLVL`); the number formats are [MS-OSHARED]
//! §2.2.1.3.

use std::sync::Arc;
use wp_model::numbering::{AbstractNum, Level, LevelOverride, MultiLevel, Num, Numbering, Suffix};
use wp_model::prop::{Justify, RunProps};

/// How many levels a list that is not simple has.
const LEVELS: usize = 9;
/// `LSTF`, `LVLF` and `LFO` are all fixed-size records.
const LSTF: usize = 28;
const LVLF: usize = 28;
const LFO: usize = 16;

/// Everything the document's two list tables say, as the model spells it.
///
/// A file with no lists gives an empty [`Numbering`], which is the same thing a
/// `.docx` with no numbering part gives, so nothing downstream has to know
/// which kind of file it came from.
pub fn read(fib: &crate::Fib, table: &[u8], fonts: &[Arc<str>]) -> Numbering {
    let mut numbering = Numbering::new();
    definitions(fib, table, fonts, &mut numbering);
    instances(fib, table, fonts, &mut numbering);
    numbering
}

/// `PlfLst`: the definitions, keyed by the `lsid` an instance names them by.
fn definitions(fib: &crate::Fib, table: &[u8], fonts: &[Arc<str>], into: &mut Numbering) {
    let Some((at, _)) = fib.at(crate::fib::field::PLF_LST) else {
        return;
    };
    // Deliberately from the offset to the end of the stream, not the slice the
    // FIB's length describes: the levels live past the end of it.
    let Some(plf) = table.get(at..) else {
        return;
    };
    let count = i16(plf, 0).max(0) as usize;
    let Some(records) = plf.get(2..2 + count * LSTF) else {
        return;
    };

    let mut walk = 2 + count * LSTF;
    for entry in records.chunks_exact(LSTF) {
        let lsid = i32(entry, 0);
        let simple = entry[26] & 0x01 != 0;
        let mut definition = AbstractNum::new(lsid as u32);
        definition.multi_level = match simple {
            true => MultiLevel::Single,
            false => MultiLevel::Multi,
        };
        for index in 0..if simple { 1 } else { LEVELS } {
            let Some((level, next)) = level(plf, walk, index as u8, fonts) else {
                break;
            };
            definition.set_level(level);
            walk = next;
        }
        into.insert_abstract(definition);
    }
}

/// One `LVL`, and where the one after it starts.
fn level(plf: &[u8], at: usize, index: u8, fonts: &[Arc<str>]) -> Option<(Level, usize)> {
    let lvlf = plf.get(at..at + LVLF)?;
    let flags = lvlf[5];
    // `cbGrpprlChpx` then `cbGrpprlPapx`, after the four bytes of
    // `dxaIndentSav` and the four that are unused — but the two groups
    // themselves follow in the other order, the paragraph's first.
    let chpx_len = lvlf[24] as usize;
    let papx_len = lvlf[25] as usize;
    let papx = plf.get(at + LVLF..at + LVLF + papx_len)?;
    let chpx = plf.get(at + LVLF + papx_len..at + LVLF + papx_len + chpx_len)?;

    let xst_at = at + LVLF + papx_len + chpx_len;
    let cch = u16(plf, xst_at) as usize;
    let chars = plf.get(xst_at + 2..xst_at + 2 + cch * 2)?;

    let mut level = Level::new(index);
    level.start = i32(lvlf, 0);
    level.format = format(lvlf[4]);
    level.text = number_text(chars, &lvlf[6..15]);
    level.justify = match flags & 0x03 {
        1 => Justify::Center,
        2 => Justify::End,
        _ => Justify::Start,
    };
    level.legal = flags & 0x04 != 0;
    // `fNoRestart` off is Word's default — any shallower level restarts this
    // one — which the model spells as saying nothing. On, `ilvlRestartLim` is
    // the first level that does *not* restart it, so the deepest one that does
    // is the level above, which is the same number counted from one.
    level.restart = (flags & 0x08 != 0).then(|| lvlf[26]);
    level.suffix = match lvlf[15] {
        1 => Suffix::Space,
        2 => Suffix::Nothing,
        _ => Suffix::Tab,
    };
    crate::sprm::apply_para(&mut level.para, papx);
    let mut run = RunProps::default();
    crate::sprm::apply_run(&mut run, chpx, fonts);
    level.run = run;
    Some((level, xst_at + 2 + cch * 2))
}

/// The label, with each placeholder written the way the model wants it.
///
/// A `.doc` puts the level a placeholder stands for straight into the string as
/// a character whose *value* is the zero-based level, and says where those
/// characters are in `rgbxchNums` — one-based offsets, in order, stopping at
/// the first zero unless all nine are used. The model wants `%1` through `%9`,
/// counted from one. A bullet has no placeholders at all.
fn number_text(chars: &[u8], places: &[u8]) -> Arc<str> {
    let units: Vec<u16> = chars
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let mut holes = [false; 64];
    for &place in places {
        if place == 0 {
            break;
        }
        if let Some(slot) = holes.get_mut(place as usize - 1) {
            *slot = true;
        }
    }
    let mut out = String::with_capacity(units.len() + 4);
    for (index, &unit) in units.iter().enumerate() {
        match holes.get(index).copied().unwrap_or(false) && unit < LEVELS as u16 {
            true => {
                out.push('%');
                out.push(char::from(b'1' + unit as u8));
            }
            false => out.push(char::from_u32(unit as u32).unwrap_or('\u{fffd}')),
        }
    }
    out.into()
}

/// An `MSONFC` as the name the model reads.
///
/// The two the specification calls out are the ones that carry no sequence:
/// 0xFF numbers nothing and 0x17 draws a bullet. The rest are a straight
/// mapping onto `ST_NumberFormat`, and one this does not name arrives as
/// itself so that a Japanese or Hebrew list is preserved rather than flattened
/// to decimal.
fn format(nfc: u8) -> wp_model::numbering::NumFormat {
    let name = match nfc {
        0x00 => "decimal",
        0x01 => "upperRoman",
        0x02 => "lowerRoman",
        0x03 => "upperLetter",
        0x04 => "lowerLetter",
        0x05 => "ordinal",
        0x06 => "cardinalText",
        0x07 => "ordinalText",
        0x09 => "chicago",
        0x0A => "ideographDigital",
        0x0B => "japaneseCounting",
        0x0E => "decimalFullWidth",
        0x10 => "japaneseLegal",
        0x11 => "japaneseDigitalTenThousand",
        0x12 => "decimalEnclosedCircle",
        0x16 => "decimalZero",
        0x17 => "bullet",
        0x1A => "decimalEnclosedFullstop",
        0x1B => "decimalEnclosedParen",
        0x2D => "hebrew1",
        0x2E => "arabicAlpha",
        0x2F => "hebrew2",
        0x30 => "arabicAbjad",
        0x3A => "russianLower",
        0x3B => "russianUpper",
        0xFF => "none",
        _ => return wp_model::numbering::NumFormat::Other(format!("msonfc{nfc}").into()),
    };
    wp_model::numbering::NumFormat::from_val(name)
}

/// `PlfLfo`: the instances, numbered from one because that is how a paragraph
/// names them.
fn instances(fib: &crate::Fib, table: &[u8], fonts: &[Arc<str>], into: &mut Numbering) {
    let Some(plf) = fib.slice(table, crate::fib::field::PLF_LFO) else {
        return;
    };
    let count = u32(plf, 0) as usize;
    let Some(records) = plf.get(4..4 + count * LFO) else {
        return;
    };
    // The overrides follow the whole array of LFOs, one variable-length record
    // each, so they can only be walked in step with it.
    let mut walk = 4 + count * LFO;
    for (index, entry) in records.chunks_exact(LFO).enumerate() {
        let lsid = i32(entry, 0);
        // An instance whose definition is not in the file numbers nothing:
        // the model answers `None` for a definition it does not hold, which is
        // the honest answer and better than a number from the wrong list.
        let mut instance = Num::new(index as u32 + 1, lsid as u32);
        let (overrides, next) = overrides(plf, walk, entry[12] as usize, fonts);
        instance.overrides = overrides;
        walk = next;
        into.insert_num(instance);
    }
}

/// One `LFOData`, and where the one after it starts.
fn overrides(
    plf: &[u8],
    at: usize,
    count: usize,
    fonts: &[Arc<str>],
) -> (Vec<LevelOverride>, usize) {
    // `cp` says where the first paragraph in the list is, which nothing here
    // needs; the overrides follow it.
    let mut walk = at + 4;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let Some(header) = plf.get(walk..walk + 8) else {
            return (out, walk);
        };
        let start = i32(header, 0);
        let bits = u32(header, 4);
        let index = (bits & 0x0F) as u8;
        let has_start = bits & 0x10 != 0;
        let formatted = bits & 0x20 != 0;
        walk += 8;
        let mut level = None;
        if formatted {
            match self::level(plf, walk, index, fonts) {
                Some((lvl, next)) => {
                    level = Some(Box::new(lvl));
                    walk = next;
                }
                None => return (out, walk),
            }
        }
        out.push(LevelOverride {
            index,
            start: has_start.then_some(start),
            level,
        });
    }
    (out, walk)
}

fn u16(data: &[u8], at: usize) -> u16 {
    match data.get(at..at + 2) {
        Some(bytes) => u16::from_le_bytes([bytes[0], bytes[1]]),
        None => 0,
    }
}

fn i16(data: &[u8], at: usize) -> i16 {
    u16(data, at) as i16
}

fn u32(data: &[u8], at: usize) -> u32 {
    match data.get(at..at + 4) {
        Some(bytes) => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        None => 0,
    }
}

fn i32(data: &[u8], at: usize) -> i32 {
    u32(data, at) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One `LVL` whose level text is `xst`, with no formatting groups.
    fn lvl(
        nfc: u8,
        flags: u8,
        follow: u8,
        restart_lim: u8,
        places: [u8; 9],
        xst: &[u16],
    ) -> Vec<u8> {
        let mut out = vec![1, 0, 0, 0, nfc, flags];
        out.extend_from_slice(&places);
        out.push(follow);
        out.extend_from_slice(&[0; 8]); // dxaIndentSav and the unused four
        out.extend_from_slice(&[0, 0]); // cbGrpprlChpx, cbGrpprlPapx
        out.push(restart_lim);
        out.push(0); // grfhic
        assert_eq!(out.len(), LVLF);
        out.extend_from_slice(&(xst.len() as u16).to_le_bytes());
        for unit in xst {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out
    }

    #[test]
    fn a_level_is_read_from_the_bytes_word_writes_it_in() {
        // `1.` at the top of a decimal list, tab after it. The placeholder is
        // the character at one-based offset 1 of the level text, and its value
        // is the zero-based level it stands for.
        let bytes = lvl(
            0x00,
            0x00,
            0,
            0,
            [1, 0, 0, 0, 0, 0, 0, 0, 0],
            &[0x0000, 0x002E],
        );
        let (level, next) = super::level(&bytes, 0, 0, &[]).expect("a level");
        assert_eq!(level.start, 1);
        assert_eq!(level.format, wp_model::numbering::NumFormat::Decimal);
        assert_eq!(&*level.text, "%1.");
        assert_eq!(level.suffix, Suffix::Tab);
        assert_eq!(level.justify, Justify::Start);
        assert_eq!(
            level.restart, None,
            "Word's default restarts on any shallower level"
        );
        assert_eq!(
            next,
            bytes.len(),
            "and the next level starts where this one ends"
        );
    }

    #[test]
    fn a_deeper_levels_label_names_every_level_above_it() {
        // `%1.%2.` — two placeholders, at the first and third characters, for
        // the zero-based levels 0 and 1.
        let bytes = lvl(
            0x00,
            0x00,
            1,
            0,
            [1, 3, 0, 0, 0, 0, 0, 0, 0],
            &[0x0000, 0x002E, 0x0001, 0x002E],
        );
        let (level, _) = super::level(&bytes, 0, 1, &[]).expect("a level");
        assert_eq!(&*level.text, "%1.%2.");
        assert_eq!(level.suffix, Suffix::Space);
    }

    #[test]
    fn a_bullet_level_has_a_glyph_where_a_number_would_be() {
        // `msonfcBullet`, whose level text is the bullet itself and holds no
        // placeholder at all — so a character that happens to sit where a
        // placeholder would must still be drawn.
        let bytes = lvl(0x17, 0x00, 2, 0, [0; 9], &[0x00B7]);
        let (level, _) = super::level(&bytes, 0, 0, &[]).expect("a level");
        assert_eq!(level.format, wp_model::numbering::NumFormat::Bullet);
        assert_eq!(&*level.text, "\u{00B7}");
        assert_eq!(level.suffix, Suffix::Nothing);
    }

    #[test]
    fn a_level_that_says_it_never_restarts_says_so_in_two_places() {
        // `fNoRestart` on with `ilvlRestartLim` zero: nothing above it restarts
        // it, which the model spells as a restart level of zero.
        let bytes = lvl(0x00, 0x08, 0, 0, [1, 0, 0, 0, 0, 0, 0, 0, 0], &[0x0000]);
        let (level, _) = super::level(&bytes, 0, 2, &[]).expect("a level");
        assert_eq!(level.restart, Some(0));
    }

    #[test]
    fn the_justification_of_a_level_is_the_low_two_bits_of_its_flags() {
        for (bits, want) in [(0, Justify::Start), (1, Justify::Center), (2, Justify::End)] {
            let bytes = lvl(0x00, bits, 0, 0, [0; 9], &[0x002E]);
            let (level, _) = super::level(&bytes, 0, 0, &[]).expect("a level");
            assert_eq!(level.justify, want, "jc {bits}");
        }
    }

    #[test]
    fn the_number_formats_word_writes_are_the_names_the_model_reads() {
        use wp_model::numbering::NumFormat;
        assert_eq!(format(0x00), NumFormat::Decimal);
        assert_eq!(format(0x01), NumFormat::UpperRoman);
        assert_eq!(format(0x04), NumFormat::LowerLetter);
        assert_eq!(format(0x16), NumFormat::DecimalZero);
        assert_eq!(format(0x17), NumFormat::Bullet);
        assert_eq!(format(0xFF), NumFormat::None);
        // One this does not name arrives as itself rather than as decimal, so
        // a Japanese list is preserved instead of flattened.
        assert!(matches!(format(0x21), NumFormat::Other(_)));
    }
}
