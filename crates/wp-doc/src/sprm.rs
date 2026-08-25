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
use wp_model::prop::{
    Border, BorderStyle, Justify, ParaProps, RunProps, TabKind, TabLeader, TabStop, Toggle,
    Underline, UnderlineKind,
};
use wp_model::table::{CellMargins, CellVAlign, RowHeight, TableBorders, VMerge, Width};
use wp_model::units::{Eighth, HalfPoint, Twips};

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
/// forms where the first operand byte is a count — and for `sprmTDefTable` and
/// `sprmPChgTabs`, the two named exceptions to that rule: the former's count is
/// two bytes, not one, and the latter's count can be 255, meaning "work it out
/// from the two arrays inside".
fn operand_length(opcode: u16, rest: &[u8]) -> Option<usize> {
    match opcode >> 13 {
        0 | 1 => Some(1),
        2 | 4 | 5 => Some(2),
        3 => Some(4),
        7 => Some(3),
        // sprmTDefTable's `cb` is two bytes and already counts itself (it is
        // "the remainder, incremented by one"), so the total span is `cb + 1`.
        _ if opcode == 0xD608 => {
            let cb = u16::from_le_bytes([*rest.first()?, *rest.get(1)?]) as usize;
            Some(cb + 1)
        }
        // Variable: a length byte, then that many bytes.
        _ => {
            let count = rest.first().copied()? as usize;
            match (opcode, count) {
                // The one exception in the whole format.
                (0xC615, 255) => {
                    let deleted = *rest.get(1)? as usize;
                    let added = *rest.get(2 + deleted * 4)? as usize;
                    // PChgTabsAdd is cTabs, then 2 bytes/entry of positions and
                    // 1 byte/entry of tab descriptors — 3 bytes each, not 6.
                    Some(2 + deleted * 4 + 1 + added * 3)
                }
                _ => Some(1 + count),
            }
        }
    }
}

/// Applies a character `grpprl` to run properties.
///
/// `fonts` is the document's font table, because a run names its face by index
/// into that table and by nothing else; an empty table leaves every run saying
/// nothing about its face, which is what the layout falls back from.
pub fn apply_run(props: &mut RunProps, data: &[u8], fonts: &[std::sync::Arc<str>]) {
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
            // The three faces a run can name, one per script. The first is the
            // one Latin text is set in, and Word writes it for the other two as
            // well whenever the document has never been anything but Latin.
            0x4A4F => {
                if let Some(name) = face(sprm.u16(), fonts) {
                    props.fonts.ascii = Some(name.clone());
                    props.fonts.high_ansi = Some(name);
                }
            }
            0x4A50 => props.fonts.east_asian = face(sprm.u16(), fonts),
            0x4A51 => props.fonts.complex = face(sprm.u16(), fonts),
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

/// The name an `ftc` stands for, or nothing when the table does not reach.
///
/// A font the table does not name is left unstated rather than guessed at: the
/// style beneath has a better answer than any face picked here would be.
fn face(index: Option<u16>, fonts: &[std::sync::Arc<str>]) -> Option<std::sync::Arc<str>> {
    fonts
        .get(index? as usize)
        .filter(|name| !name.is_empty())
        .cloned()
}

/// The style index a character `grpprl` names, if it names one.
pub fn run_style(data: &[u8]) -> Option<u16> {
    walk(data)
        .into_iter()
        .find(|sprm| sprm.opcode == 0x4A30)
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
            // Direct tab stops (PChgTabsOperand) and a style's own tab stops
            // (PChgTabsPapxOperand, found only inside a style's UpxPapx) are
            // different operand shapes for the same idea — the former has a
            // close-distance array the latter doesn't — so both are handled
            // by the one shared parser, distinguished by `has_close`.
            0xC615 => add_tabs(&mut props.tabs, tab_stops(sprm.operand, true)),
            0xC60D => add_tabs(&mut props.tabs, tab_stops(sprm.operand, false)),
            _ => {}
        }
    }
}

/// Appends freshly parsed stops rather than replacing the list: a paragraph
/// (or a style) can state more than one `PChgTabs`-family sprm, and each one
/// adds to what came before it in the same `grpprl`. Merging a direct
/// paragraph's stops with its *style's* stops is a separate, later step
/// ([`ParaProps::layer`]), not this one.
fn add_tabs(tabs: &mut Option<Vec<TabStop>>, mut new: Vec<TabStop>) {
    if new.is_empty() {
        return;
    }
    match tabs {
        Some(existing) => existing.append(&mut new),
        None => *tabs = Some(new),
    }
}

/// A tab stop's alignment and leader, packed into one byte — the `TBD`
/// structure. Bits 0-2 are `jc`, bits 3-5 are `tlc`, the top two are unused.
fn tab_descriptor(byte: u8) -> (TabKind, TabLeader) {
    let kind = match byte & 0x07 {
        1 => TabKind::Center,
        2 => TabKind::End,
        3 => TabKind::Decimal,
        4 => TabKind::Bar,
        _ => TabKind::Start,
    };
    let leader = match (byte >> 3) & 0x07 {
        1 => TabLeader::Dot,
        2 => TabLeader::Hyphen,
        3 | 4 => TabLeader::Underscore,
        5 => TabLeader::MiddleDot,
        _ => TabLeader::None,
    };
    (kind, leader)
}

/// Reads a `PChgTabsAdd` (`cTabs`, then that many `XAS` positions, then that
/// many `TBD` descriptor bytes) starting at `data`, returning the stops and
/// how many bytes were consumed.
fn tabs_add(data: &[u8]) -> (Vec<TabStop>, usize) {
    let Some(&count) = data.first() else {
        return (Vec::new(), 0);
    };
    let count = count as usize;
    let positions = data.get(1..1 + count * 2).unwrap_or_default();
    let descriptors = data
        .get(1 + count * 2..1 + count * 2 + count)
        .unwrap_or_default();
    let stops = positions
        .chunks_exact(2)
        .zip(descriptors)
        .map(|(pos, &tbd)| {
            let position = Twips(i16::from_le_bytes([pos[0], pos[1]]) as i32);
            let (kind, leader) = tab_descriptor(tbd);
            TabStop {
                position,
                kind,
                leader,
            }
        })
        .collect();
    (stops, 1 + count * 2 + count)
}

/// Reads a `cTabs`-then-`rgdxaDel` array (an XAS per entry) starting at
/// `data`, returning cleared stops and how many bytes were consumed. Used by
/// both `PChgTabsDelClose` (which has a trailing close-distance array this
/// does not read — the caller skips it separately) and the simpler
/// `PChgTabsDel` a style's own tab sprm uses.
fn tabs_del(data: &[u8]) -> (Vec<TabStop>, usize) {
    let Some(&count) = data.first() else {
        return (Vec::new(), 0);
    };
    let count = count as usize;
    let positions = data.get(1..1 + count * 2).unwrap_or_default();
    let stops = positions
        .chunks_exact(2)
        .map(|pos| TabStop {
            position: Twips(i16::from_le_bytes([pos[0], pos[1]]) as i32),
            kind: TabKind::Clear,
            leader: TabLeader::None,
        })
        .collect();
    (stops, 1 + count * 2)
}

/// Parses a `PChgTabs(Papx)?Operand`: a 1-byte `cb` (already consumed by the
/// sprm walk into `op`), a delete list, then an add list. `has_close` tells
/// the deletes apart — direct paragraph formatting's `PChgTabsDelClose` adds
/// a same-length close-distance array after `rgdxaDel` that a style's own
/// `PChgTabsDel` does not have.
///
/// Deleted positions come back as [`TabKind::Clear`] stops, which
/// `ParaProps`'s own merge already knows removes an inherited stop at that
/// position — so a delete here does not need special handling.
fn tab_stops(op: &[u8], has_close: bool) -> Vec<TabStop> {
    // `op` is the whole sprm operand, `cb` included as its first byte —
    // `operand_length` only sizes the span, it does not strip anything —
    // so the delete list starts one byte in, for the ordinary (`cb` < 255)
    // form and the rare `cb == 255` extended one alike (the latter's second
    // byte is where `cTabsDel` starts either way).
    let payload = op.get(1..).unwrap_or_default();
    let (mut dels, mut at) = tabs_del(payload);
    if has_close {
        // Skip the close-distance array (same length as rgdxaDel): it says
        // how wide a net each delete casts, which this reader does not need
        // — an exact-position delete is close enough for a legacy format.
        let count = payload.first().copied().unwrap_or(0) as usize;
        at += count * 2;
    }
    let (adds, _) = tabs_add(payload.get(at..).unwrap_or_default());
    dels.extend(adds);
    dels
}

/// Decodes a `Brc80`/`Brc80MayBeNil`: 4 bytes shared by paragraph, table and
/// cell borders alike.
///
/// **Saying nothing and saying "none" are different answers.** A cell whose
/// `TC80` is all zeroes on a side has not spoken, and the table's own rule —
/// `sprmTTableBorders80`, which is where an ordinary grid comes from — runs
/// there. All bits set is `Brc80MayBeNil`'s way of saying the cell *has*
/// spoken and wants no rule, which is what stops the line under a letterhead's
/// title from being drawn across the middle of it. Answering `None` to both
/// leaves the table's rule showing through a border the file struck out.
fn brc80(bytes: &[u8]) -> Option<Border> {
    let bytes: [u8; 4] = bytes.get(..4)?.try_into().ok()?;
    if bytes == [0xFF, 0xFF, 0xFF, 0xFF] {
        return Some(Border {
            style: BorderStyle::None,
            size: None,
            space: None,
            color: None,
            shadow: false,
        });
    }
    let width = bytes[0];
    let style = brc_type(bytes[1]);
    if style == BorderStyle::None {
        return None;
    }
    let color = ico(bytes[2]);
    let space = bytes[3] & 0x1F;
    Some(Border {
        style,
        size: Some(Eighth(width.max(2) as i32)),
        space: Some(space),
        color,
        shadow: bytes[3] & 0x20 != 0,
    })
}

/// `BrcType` — the border line style, matched against the handful this
/// project's `BorderStyle` names directly. Everything else (including the
/// ~180 Word 97 clip-art borders) draws as a plain line, the same fallback
/// `BorderStyle::from_val` already uses for OOXML's unrecognised names.
fn brc_type(value: u8) -> BorderStyle {
    match value {
        0x00 => BorderStyle::None,
        0x01 | 0x05 => BorderStyle::Single,
        0x03 => BorderStyle::Double,
        0x06 => BorderStyle::Dotted,
        0x07 | 0x16 => BorderStyle::Dashed,
        0x08 => BorderStyle::DotDash,
        0x09 => BorderStyle::DotDotDash,
        0x0A => BorderStyle::Triple,
        0x14 => BorderStyle::Wave,
        0x15 => BorderStyle::DoubleWave,
        _ => BorderStyle::Art,
    }
}

/// One table row's geometry, read from the row mark's own `grpprl` — that is
/// where a `.doc` states `sprmTDefTable` and `sprmTTableBorders80`, per
/// [MS-DOC] Overview of Tables: "The properties of each row mark MUST define
/// the cells for that table row."
#[derive(Debug, Default, Clone)]
pub struct TableRow {
    /// Column boundaries from `sprmTDefTable`'s `rgdxaCenter`, left edge
    /// first, as the file states them: positions measured from the text
    /// margin rather than widths. **Rows of one table need not share them** —
    /// that is one of the two ways a `.doc` spells a horizontal merge — so the
    /// table's grid is the union of every row's, and a cell spans however many
    /// of the union's columns its own two boundaries enclose.
    pub boundaries: Option<Vec<i32>>,
    /// One entry per column, from `sprmTDefTable`'s `rgTc80`.
    pub cells: Vec<CellDef>,
    /// The row's own uniform border, from `sprmTTableBorders80` — table
    /// level, not per cell (see `wp-layout`'s cell-then-table precedence).
    pub borders: Option<TableBorders>,
    /// Half the gap between two columns, from `sprmTDxaGapHalf`. It is both
    /// the padding inside every cell and the amount the first boundary sits
    /// to the left of where the table's text begins, which is why a table
    /// whose `rgdxaCenter` starts at -108 is nonetheless flush with the margin.
    pub gap_half: Option<Twips>,
    /// `sprmTDyaRowHeight`: positive is "at least", negative is "exactly", and
    /// zero is "whatever the content needs".
    pub height: Option<RowHeight>,
    /// `sprmTCellPaddingDefault` — the table's own cell margins, which a `.doc`
    /// states per row alongside everything else about the row.
    pub padding: Option<CellMargins>,
}

/// One cell's definition, out of `sprmTDefTable`'s `rgTc80`.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct CellDef {
    pub borders: TableBorders,
    /// `TC80.tcgrf`'s `fVertMerge` and `fVertRestart`. A cell that continues
    /// the one above holds no content of its own, and drawing it as an ordinary
    /// empty cell is what puts a white box under a letterhead.
    pub v_merge: Option<VMerge>,
    /// `fMerged` — Word 97's horizontal merge, where the cells stay in the row
    /// and a flag says to draw them as one. Word 2000 and later spell the same
    /// thing by giving the row fewer and wider columns instead, so a reader
    /// that wants both has to understand both.
    pub merged_left: bool,
    pub v_align: CellVAlign,
}

/// Table-domain sprms (`sgc` 5) mixed into a row-mark paragraph's `grpprl`
/// alongside its paragraph-domain ones. `sprm::walk` is domain-agnostic —
/// length comes from `spra` alone — so these survive intact regardless of
/// what else is in the list; only `sprmTDefTable` and `sprmTTableBorders80`
/// are read here (see `crates/wp-doc/src/lib.rs`'s scope note on table
/// geometry for what is deliberately left out).
pub fn table_row(data: &[u8]) -> TableRow {
    let mut row = TableRow::default();
    for sprm in walk(data) {
        match sprm.opcode {
            0xD608 => def_table(&mut row, sprm.operand),
            0xD605 => row.borders = Some(table_borders_80(sprm.operand)),
            0x9602 => row.gap_half = sprm.i16().map(|half| Twips(half as i32)),
            0x9407 => row.height = sprm.i16().map(row_height),
            0xD634 => cell_padding_default(&mut row, sprm.operand),
            _ => {}
        }
    }
    row
}

/// `sprmTDyaRowHeight`, whose sign carries the rule that `<w:trHeight>` spells
/// out in an attribute of its own.
fn row_height(value: i16) -> RowHeight {
    match value {
        0 => RowHeight::Auto,
        exact if exact < 0 => RowHeight::Exact(Twips(-(exact as i32))),
        at_least => RowHeight::AtLeast(Twips(at_least as i32)),
    }
}

/// `sprmTCellPaddingDefault`'s operand: `cb`(1) + the range of cells it covers
/// (2, ignored — the *default* is the table's) + `grfbrc`(1, which sides this
/// one states) + `ftsWidth`(1) + `wWidth`(2). Word writes it twice over, once
/// for the pair of sides it leaves at nothing and once for the pair it pads.
fn cell_padding_default(row: &mut TableRow, op: &[u8]) {
    let Some(sides) = op.get(3).copied() else {
        return;
    };
    // `ftsWidth` 3 is twips. Auto, percent and nil are not paddings this can
    // honour, and guessing one moves the text in every cell of the table.
    if op.get(4).copied() != Some(3) {
        return;
    }
    let Some(bytes) = op.get(5..7) else {
        return;
    };
    let width = Width::Fixed(Twips(i16::from_le_bytes([bytes[0], bytes[1]]) as i32));
    let margins = row.padding.get_or_insert_with(CellMargins::default);
    for (bit, side) in [
        (0x01, &mut margins.top),
        (0x02, &mut margins.start),
        (0x04, &mut margins.bottom),
        (0x08, &mut margins.end),
    ] {
        if sides & bit != 0 {
            *side = Some(width);
        }
    }
}

/// `TDefTableOperand`: `cb`(2) + `NumberOfColumns`(1) + `rgdxaCenter`
/// (columns+1 `XAS` positions) + `rgTc80` (one 20-byte `TC80` per column,
/// short columns left borderless). `op` is the whole sprm operand, `cb`
/// included — unlike the generic variable-length sprms, `operand_length`
/// does not strip anything for this one either, so `NumberOfColumns` starts
/// two bytes in.
fn def_table(row: &mut TableRow, op: &[u8]) {
    let Some(&columns) = op.get(2) else {
        return;
    };
    let columns = columns as usize;
    let Some(centers) = op.get(3..3 + (columns + 1) * 2) else {
        return;
    };
    row.boundaries = Some(
        centers
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as i32)
            .collect(),
    );
    let tc80s = op.get(3 + (columns + 1) * 2..).unwrap_or_default();
    row.cells = tc80s
        .chunks(20)
        .filter(|chunk| chunk.len() == 20)
        .map(|tc80| {
            let tcgrf = u16::from_le_bytes([tc80[0], tc80[1]]);
            CellDef {
                borders: TableBorders {
                    top: brc80(&tc80[4..8]),
                    start: brc80(&tc80[8..12]),
                    bottom: brc80(&tc80[12..16]),
                    end: brc80(&tc80[16..20]),
                    inside_h: None,
                    inside_v: None,
                },
                // `fVertMerge` with `fVertRestart` starts a merge; `fVertMerge`
                // alone continues the one above. The model spells it the same
                // way, so nothing downstream has to invert it.
                v_merge: match (tcgrf & 0x0020 != 0, tcgrf & 0x0040 != 0) {
                    (false, _) => None,
                    (true, true) => Some(VMerge::Restart),
                    (true, false) => Some(VMerge::Continue),
                },
                merged_left: tcgrf & 0x0002 != 0,
                v_align: match (tcgrf >> 7) & 0x03 {
                    1 => CellVAlign::Center,
                    2 => CellVAlign::Bottom,
                    _ => CellVAlign::Top,
                },
            }
        })
        .collect();
}

/// `TableBordersOperand80`: `cb`(1, always 0x18) + 6 `Brc80MayBeNil` in the
/// fixed order top/left/bottom/right/insideH/insideV. `op` is the whole sprm
/// operand, `cb` included, so the borders start one byte in.
fn table_borders_80(op: &[u8]) -> TableBorders {
    TableBorders {
        top: op.get(1..5).and_then(brc80),
        start: op.get(5..9).and_then(brc80),
        bottom: op.get(9..13).and_then(brc80),
        end: op.get(13..17).and_then(brc80),
        inside_h: op.get(17..21).and_then(brc80),
        inside_v: op.get(21..25).and_then(brc80),
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
///
/// **Automatic is a colour and not a silence.** A run that says `auto` has
/// overridden whatever its style asked for; answering "nothing stated" instead
/// leaves a table of contents in the blue its hyperlink style wanted and Word
/// does not draw.
fn ico(index: u8) -> Option<wp_model::Color> {
    let rgb = match index {
        0 => return Some(wp_model::Color::Auto),
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
        apply_run(&mut props, &[0x35, 0x08, 128], &[]);
        assert_eq!(props.toggles.get(Toggle::Bold), None);
        apply_run(&mut props, &[0x35, 0x08, 1], &[]);
        assert_eq!(props.toggles.get(Toggle::Bold), Some(true));
    }

    #[test]
    fn a_toggle_of_129_is_what_word_writes_for_ctrl_b() {
        // Every `.doc` this project has looked at spells direct bold as 129,
        // not 1. A reader that only understands 1 shows no formatting at all.
        let mut props = RunProps::default();
        apply_run(&mut props, &[0x35, 0x08, 129], &[]);
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
    fn colour_index_zero_is_automatic_rather_than_unstated() {
        let mut props = RunProps::default();
        apply_run(&mut props, &[0x42, 0x2A, 0x00], &[]);
        assert_eq!(
            props.color,
            Some(wp_model::Color::Auto),
            "a run that says automatic has overridden its style, not kept quiet"
        );
        apply_run(&mut props, &[0x42, 0x2A, 0x06], &[]);
        assert_eq!(props.color, Some(wp_model::Color::Rgb([0xFF, 0x00, 0x00])));
    }

    #[test]
    fn a_direct_tab_stop_lands_at_its_stated_position() {
        // sprmPChgTabs: cb=5, zero deletes, one add — End-aligned, dot leader,
        // at 5000 twips (0x1388 LE).
        let mut props = ParaProps::default();
        apply_para(
            &mut props,
            &[0x15, 0xC6, 0x05, 0x00, 0x01, 0x88, 0x13, 0x0A],
        );
        assert_eq!(
            props.tabs,
            Some(vec![TabStop {
                position: Twips(5000),
                kind: TabKind::End,
                leader: TabLeader::Dot,
            }])
        );
    }

    #[test]
    fn a_styles_own_tab_stop_uses_the_papx_shaped_operand() {
        // sprmPChgTabsPapx: cb=4, one delete at 1440 twips (0x05A0 LE), no
        // adds. The Papx form has no close-distance array, unlike the direct
        // sprmPChgTabs — a delete here still comes back as a Clear stop.
        let mut props = ParaProps::default();
        apply_para(&mut props, &[0x0D, 0xC6, 0x04, 0x01, 0xA0, 0x05, 0x00]);
        assert_eq!(
            props.tabs,
            Some(vec![TabStop {
                position: Twips(1440),
                kind: TabKind::Clear,
                leader: TabLeader::None,
            }])
        );
    }

    #[test]
    fn brc80_reads_width_type_colour_and_space() {
        // 8 eighths of a point (1pt), single line, red, no space.
        assert_eq!(
            brc80(&[8, 0x01, 0x06, 0x00]),
            Some(Border {
                style: BorderStyle::Single,
                size: Some(Eighth(8)),
                space: Some(0),
                color: Some(wp_model::Color::Rgb([0xFF, 0x00, 0x00])),
                shadow: false,
            })
        );
    }

    #[test]
    fn brc80_all_bits_set_is_a_border_that_says_it_is_not_there() {
        // Not `None`: the cell has struck the rule out, and answering "nothing
        // stated" lets the table's own rule run where the file said it must
        // not.
        assert_eq!(
            brc80(&[0xFF, 0xFF, 0xFF, 0xFF]),
            Some(Border {
                style: BorderStyle::None,
                size: None,
                space: None,
                color: None,
                shadow: false,
            })
        );
    }

    #[test]
    fn brc80_all_zeroes_is_a_cell_that_has_not_spoken() {
        assert_eq!(brc80(&[0, 0x00, 0x00, 0x00]), None);
    }

    #[test]
    fn sprm_t_def_table_gives_the_grid_and_each_columns_border() {
        // One column, 0..2000 twips, every side a single 1pt red border —
        // followed by an unrelated sprm, to prove the two-byte `cb` (the one
        // exception besides sprmPChgTabs) steps the walk correctly rather
        // than corrupting everything after it.
        let brc = [8u8, 0x01, 0x06, 0x00];
        let mut tc80 = vec![0u8, 0, 0xD0, 0x07]; // tcgrf, wWidth
        tc80.extend(brc.repeat(4)); // brcTop, brcLeft, brcBottom, brcRight
        let mut operand = vec![0x01]; // NumberOfColumns
        operand.extend([0x00, 0x00, 0xD0, 0x07]); // rgdxaCenter: 0, 2000
        operand.extend(&tc80);
        let cb = (operand.len() + 1) as u16;
        let mut data = vec![0x08, 0xD6]; // sprmTDefTable
        data.extend(cb.to_le_bytes());
        data.extend(&operand);
        data.extend([0x17, 0x24, 0x01]); // sprmPFTtp, unrelated, must survive

        let sprms = walk(&data);
        assert_eq!(
            sprms.len(),
            2,
            "the def-table sprm must not eat its neighbour"
        );
        assert_eq!(sprms[1].opcode, 0x2417);

        let row = table_row(&data);
        assert_eq!(
            row.boundaries,
            Some(vec![0, 2000]),
            "the boundaries are kept as the file states them"
        );
        assert_eq!(row.cells.len(), 1);
        let border = Border {
            style: BorderStyle::Single,
            size: Some(Eighth(8)),
            space: Some(0),
            color: Some(wp_model::Color::Rgb([0xFF, 0x00, 0x00])),
            shadow: false,
        };
        assert_eq!(row.cells[0].borders.top, Some(border));
        assert_eq!(row.cells[0].borders.end, Some(border));
    }

    #[test]
    fn sprm_t_table_borders_80_is_the_rows_own_border() {
        let brc = [8u8, 0x01, 0x06, 0x00];
        let nil = [0xFFu8, 0xFF, 0xFF, 0xFF];
        let mut data = vec![0x05, 0xD6, 0x18]; // sprmTTableBorders80, cb=0x18
        data.extend(brc); // top
        data.extend(nil); // left, bottom, right, insideH, insideV
        data.extend(nil);
        data.extend(nil);
        data.extend(nil);
        data.extend(nil);

        let row = table_row(&data);
        assert_eq!(
            row.borders.unwrap().top,
            Some(Border {
                style: BorderStyle::Single,
                size: Some(Eighth(8)),
                space: Some(0),
                color: Some(wp_model::Color::Rgb([0xFF, 0x00, 0x00])),
                shadow: false,
            })
        );
    }
}
