//! Sprms: the property opcodes a `.doc` states its formatting in.
//!
//! A sprm is a 16-bit opcode and an operand whose *size is encoded in the
//! opcode* — three bits of it, `spra`, say whether what follows is one byte, two,
//! four, three, or a counted run. That matters more than any individual opcode:
//! a reader that does not know how long a sprm is cannot find the next one, and
//! a single wrong length turns the rest of the list into noise. So the walk is
//! driven by `spra` and the opcodes it does not know are stepped over rather
//! than guessed at.
//!
//! Toggles are the other trap, and the same one `.docx` has: a value of 128
//! means "whatever the style says" and 129 means "the opposite of what the style
//! says". Reading them as booleans makes every styled document wrong.

use wp_model::color::Highlight;
use wp_model::prop::{Justify, ParaProps, RunProps, Toggle, Underline, UnderlineKind};
use wp_model::units::{HalfPoint, Twips};

/// One property, as it appears in a `grpprl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sprm<'a> {
    pub opcode: u16,
    pub operand: &'a [u8],
}

impl Sprm<'_> {
    /// The one-byte operand, as a toggle.
    ///
    /// `None` for "as the style says", which is not the same as false.
    fn toggle(&self) -> Option<bool> {
        match self.operand.first().copied()? {
            0 => Some(false),
            1 => Some(true),
            // 128 is "whatever the style says", which is not a value at all.
            128 => None,
            // 129 is "the opposite of what the style says" — and this is what
            // Word actually writes when a user presses Ctrl+B, because bold *is*
            // a toggle. Resolving it properly needs the style's own value, which
            // this reader does not have (see `style`), so it resolves against
            // the default: off inverted is on. A run made bold inside a heading
            // that is already bold therefore comes out bold rather than plain,
            // which is the rarer of the two mistakes and the less surprising.
            129 => Some(true),
            _ => None,
        }
    }

    fn u8(&self) -> Option<u8> {
        self.operand.first().copied()
    }

    fn i16(&self) -> Option<i16> {
        let bytes = self.operand.get(..2)?;
        Some(i16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u16(&self) -> Option<u16> {
        self.i16().map(|value| value as u16)
    }
}

/// Walks a `grpprl`, a list of sprms packed end to end.
pub fn walk(mut data: &[u8]) -> Vec<Sprm<'_>> {
    let mut out = Vec::new();
    while data.len() >= 2 {
        let opcode = u16::from_le_bytes([data[0], data[1]]);
        let rest = &data[2..];
        let Some(length) = operand_length(opcode, rest) else {
            break;
        };
        if rest.len() < length {
            break;
        }
        out.push(Sprm {
            opcode,
            operand: &rest[..length],
        });
        data = &rest[length..];
    }
    out
}

/// How many bytes of operand follow an opcode.
///
/// The top three bits are the whole answer, except for the two variable-length
/// forms where the first operand byte is a count — and for `sprmPChgTabs`, whose
/// count can be 255, meaning "work it out from the two arrays inside".
fn operand_length(opcode: u16, rest: &[u8]) -> Option<usize> {
    match opcode >> 13 {
        0 | 1 => Some(1),
        2 | 4 | 5 => Some(2),
        3 => Some(4),
        7 => Some(3),
        // Variable: a length byte, then that many bytes.
        _ => {
            let count = rest.first().copied()? as usize;
            match (opcode, count) {
                // The one exception in the whole format.
                (0xC615, 255) => {
                    let deleted = *rest.get(1)? as usize;
                    let added = *rest.get(2 + deleted * 4)? as usize;
                    Some(2 + deleted * 4 + 1 + added * 6)
                }
                _ => Some(1 + count),
            }
        }
    }
}

/// Applies a character `grpprl` to run properties.
pub fn apply_run(props: &mut RunProps, data: &[u8]) {
    for sprm in walk(data) {
        match sprm.opcode {
            // The toggles, in the order the format lists them.
            0x0835 => toggle(props, Toggle::Bold, sprm.toggle()),
            0x0836 => toggle(props, Toggle::Italic, sprm.toggle()),
            0x0837 => toggle(props, Toggle::Strike, sprm.toggle()),
            0x0838 => toggle(props, Toggle::Outline, sprm.toggle()),
            0x0839 => toggle(props, Toggle::Shadow, sprm.toggle()),
            0x083A => toggle(props, Toggle::SmallCaps, sprm.toggle()),
            0x083B => toggle(props, Toggle::Caps, sprm.toggle()),
            0x083C => toggle(props, Toggle::Vanish, sprm.toggle()),
            // Size, in half-points, exactly as `.docx` states it.
            0x4A43 => props.size = sprm.u16().map(|value| HalfPoint(value as i32)),
            0x2A3E => {
                props.underline = sprm.u8().map(|value| Underline {
                    kind: underline(value),
                    color: None,
                })
            }
            0x2A42 => props.color = sprm.u8().and_then(ico),
            0x2A0C => props.highlight = sprm.u8().and_then(highlight),
            // A font is an index into the font table; the name is resolved by
            // the caller, which is the only thing that has the table.
            0x4A4F..=0x4A51 => {}
            _ => {}
        }
    }
}

/// Sets a toggle, or clears it when the file says "as the style says".
fn toggle(props: &mut RunProps, which: Toggle, value: Option<bool>) {
    match value {
        Some(on) => props.toggles.set(which, on),
        None => props.toggles.clear(which),
    }
}

/// The style index a character `grpprl` names, if it names one.
pub fn run_style(data: &[u8]) -> Option<u16> {
    walk(data)
        .into_iter()
        .find(|sprm| sprm.opcode == 0x4A30)
        .and_then(|sprm| sprm.u16())
}

/// The font index a character `grpprl` names.
pub fn run_font(data: &[u8]) -> Option<u16> {
    walk(data)
        .into_iter()
        .find(|sprm| matches!(sprm.opcode, 0x4A4F..=0x4A51))
        .and_then(|sprm| sprm.u16())
}

/// Applies a paragraph `grpprl` to paragraph properties.
pub fn apply_para(props: &mut ParaProps, data: &[u8]) {
    for sprm in walk(data) {
        match sprm.opcode {
            0x2403 => props.justify = sprm.u8().and_then(justify),
            0x840F => props.indent.start = sprm.i16().map(|value| Twips(value as i32)),
            0x840E => props.indent.end = sprm.i16().map(|value| Twips(value as i32)),
            0x8411 => {
                // One number does both jobs: negative is a hanging indent.
                let value = sprm.i16().unwrap_or(0);
                match value < 0 {
                    true => props.indent.hanging = Some(Twips(-value as i32)),
                    false => props.indent.first_line = Some(Twips(value as i32)),
                }
            }
            0xA413 => props.spacing.before = sprm.u16().map(|value| Twips(value as i32)),
            0xA414 => props.spacing.after = sprm.u16().map(|value| Twips(value as i32)),
            0x6412 => {
                // Four bytes: the amount, then whether it is a multiple of a
                // line or an exact measurement.
                let amount = sprm.i16().unwrap_or(0);
                let multiple = sprm
                    .operand
                    .get(2..4)
                    .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]) != 0)
                    .unwrap_or(false);
                props.spacing.line = Some(match (multiple, amount < 0) {
                    (true, _) => {
                        wp_model::prop::LineSpacing::Multiple(wp_model::Line240(amount as i32))
                    }
                    // A negative exact value means "exactly", positive "at least".
                    (false, true) => wp_model::prop::LineSpacing::Exact(Twips(-amount as i32)),
                    (false, false) => wp_model::prop::LineSpacing::AtLeast(Twips(amount as i32)),
                });
            }
            0x2407 => props.keep_next = sprm.toggle(),
            0x2406 => props.keep_lines = sprm.toggle(),
            0x2405 => props.page_break_before = sprm.toggle(),
            0x2404 => props.widow_control = sprm.toggle(),
            _ => {}
        }
    }
}

/// The style index a paragraph `grpprl` names.
pub fn para_style(data: &[u8]) -> Option<u16> {
    walk(data)
        .into_iter()
        .find(|sprm| sprm.opcode == 0x4600)
        .and_then(|sprm| sprm.u16())
}

/// Whether a paragraph `grpprl` says the paragraph is in a table, and whether it
/// is the row mark that ends one.
pub fn table_flags(data: &[u8]) -> (bool, bool) {
    let mut in_table = false;
    let mut row_end = false;
    for sprm in walk(data) {
        match sprm.opcode {
            0x2416 => in_table = sprm.u8().unwrap_or(0) != 0,
            0x2417 => row_end = sprm.u8().unwrap_or(0) != 0,
            _ => {}
        }
    }
    (in_table, row_end)
}

fn justify(value: u8) -> Option<Justify> {
    Some(match value {
        0 => Justify::Start,
        1 => Justify::Center,
        2 => Justify::End,
        3 => Justify::Both,
        4 => Justify::Distribute,
        _ => return None,
    })
}

/// The marker-pen palette, which is named colours rather than a colour picker.
fn highlight(index: u8) -> Option<Highlight> {
    Some(match index {
        0 => return None,
        1 => Highlight::Black,
        2 => Highlight::Blue,
        3 => Highlight::Cyan,
        4 => Highlight::Green,
        5 => Highlight::Magenta,
        6 => Highlight::Red,
        7 => Highlight::Yellow,
        8 => Highlight::White,
        9 => Highlight::DarkBlue,
        10 => Highlight::DarkCyan,
        11 => Highlight::DarkGreen,
        12 => Highlight::DarkMagenta,
        13 => Highlight::DarkRed,
        14 => Highlight::DarkYellow,
        15 => Highlight::DarkGray,
        16 => Highlight::LightGray,
        _ => return None,
    })
}

fn underline(value: u8) -> UnderlineKind {
    match value {
        0 => UnderlineKind::None,
        1 => UnderlineKind::Single,
        2 => UnderlineKind::Words,
        3 => UnderlineKind::Double,
        4 => UnderlineKind::Dotted,
        6 => UnderlineKind::Thick,
        7 => UnderlineKind::Dash,
        9 => UnderlineKind::DotDash,
        10 => UnderlineKind::DotDotDash,
        11 => UnderlineKind::Wave,
        _ => UnderlineKind::Single,
    }
}

/// The seventeen colours a `.doc` can name a colour by index.
///
/// There is no palette to look them up in: the numbers *are* the colours, and
/// they are the same sixteen every version of Word has had.
fn ico(index: u8) -> Option<wp_model::Color> {
    let rgb = match index {
        // 0 is `auto`, which is not a colour but an instruction.
        0 => return None,
        1 => [0x00, 0x00, 0x00],
        2 => [0x00, 0x00, 0xFF],
        3 => [0x00, 0xFF, 0xFF],
        4 => [0x00, 0xFF, 0x00],
        5 => [0xFF, 0x00, 0xFF],
        6 => [0xFF, 0x00, 0x00],
        7 => [0xFF, 0xFF, 0x00],
        8 => [0xFF, 0xFF, 0xFF],
        9 => [0x00, 0x00, 0x80],
        10 => [0x00, 0x80, 0x80],
        11 => [0x00, 0x80, 0x00],
        12 => [0x80, 0x00, 0x80],
        13 => [0x80, 0x00, 0x00],
        14 => [0x80, 0x80, 0x00],
        15 => [0x80, 0x80, 0x80],
        16 => [0xC0, 0xC0, 0xC0],
        _ => return None,
    };
    Some(wp_model::Color::Rgb(rgb))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_length_of_a_sprm_comes_out_of_its_opcode() {
        // A reader that cannot find the end of one sprm cannot find the start of
        // the next, and the rest of the list becomes noise.
        let data = [
            0x35, 0x08, 0x01, // bold on, one byte
            0x43, 0x4A, 0x30, 0x00, // size 24 half-points, two bytes
        ];
        let sprms = walk(&data);
        assert_eq!(sprms.len(), 2);
        assert_eq!(sprms[0].opcode, 0x0835);
        assert_eq!(sprms[1].operand, &[0x30, 0x00]);
    }

    #[test]
    fn an_unknown_sprm_is_stepped_over_rather_than_guessed_at() {
        let data = [
            0x00, 0x60, 0x99, 0x00, 0x11, 0x22, // an unknown four-byte one
            0x35, 0x08, 0x01, // bold, which must still be found
        ];
        let sprms = walk(&data);
        assert_eq!(sprms.len(), 2);
        assert_eq!(sprms[1].opcode, 0x0835);
    }

    #[test]
    fn a_toggle_of_128_means_whatever_the_style_says() {
        // Read as a boolean it means "off", and every styled run comes out wrong.
        let mut props = RunProps::default();
        apply_run(&mut props, &[0x35, 0x08, 128]);
        assert_eq!(props.toggles.get(Toggle::Bold), None);
        apply_run(&mut props, &[0x35, 0x08, 1]);
        assert_eq!(props.toggles.get(Toggle::Bold), Some(true));
    }

    #[test]
    fn a_toggle_of_129_is_what_word_writes_for_ctrl_b() {
        // Every `.doc` this project has looked at spells direct bold as 129,
        // not 1. A reader that only understands 1 shows no formatting at all.
        let mut props = RunProps::default();
        apply_run(&mut props, &[0x35, 0x08, 129]);
        assert_eq!(props.toggles.get(Toggle::Bold), Some(true));
    }

    #[test]
    fn a_negative_first_line_indent_is_a_hanging_one() {
        let mut props = ParaProps::default();
        apply_para(&mut props, &[0x11, 0x84, 0x1C, 0xFF]);
        assert_eq!(props.indent.hanging, Some(Twips(228)));
        assert_eq!(props.indent.first_line, None);
    }

    #[test]
    fn alignment_and_spacing_come_through() {
        let mut props = ParaProps::default();
        apply_para(
            &mut props,
            &[
                0x03, 0x24, 0x01, 0x13, 0xA4, 0x2C, 0x01, 0x14, 0xA4, 0xF0, 0x00,
            ],
        );
        assert_eq!(props.justify, Some(Justify::Center));
        assert_eq!(props.spacing.before, Some(Twips(300)));
        assert_eq!(props.spacing.after, Some(Twips(240)));
    }

    #[test]
    fn colour_index_zero_is_an_instruction_rather_than_a_colour() {
        let mut props = RunProps::default();
        apply_run(&mut props, &[0x42, 0x2A, 0x00]);
        assert_eq!(props.color, None, "auto is not a colour");
        apply_run(&mut props, &[0x42, 0x2A, 0x06]);
        assert_eq!(props.color, Some(wp_model::Color::Rgb([0xFF, 0x00, 0x00])));
    }
}
