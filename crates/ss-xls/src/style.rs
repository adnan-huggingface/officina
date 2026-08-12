//! `FONT`, `FORMAT`, `XF` and `PALETTE` into the model's style table.
//!
//! BIFF packs a cell's whole appearance into twenty bytes of bitfields, where
//! xlsx spends four separate elements on it. The mapping is mechanical but has
//! two edges worth naming.
//!
//! **Font index 4 does not exist.** The `FONT` records are numbered 0, 1, 2, 3,
//! 5, 6, … — the fourth slot was used by Excel 5 for something else and the
//! numbering was never closed up. An `XF` that says `ifnt = 7` means the sixth
//! font record. Read as a plain index, every cell in a workbook with more than
//! four fonts is drawn in the wrong one, and the wrongness is one place off, so
//! it looks like a rendering bug rather than an indexing one.
//!
//! **There is one XF list, not two.** xlsx keeps named-style formats in
//! `cellStyleXfs` and cell formats in `cellXfs`, and a cell's `s` indexes only
//! the second. BIFF interleaves both kinds in one list and a cell's `ixfe`
//! indexes the whole of it, so the list is handed over as `cell_xfs` intact and
//! `StyleId` is `ixfe` unchanged.

use std::collections::BTreeMap;

use ss_model::style::{
    Alignment, Border, BorderStyle, CellFormat, Edge, Fill, Font, HAlign, Parts, Pattern,
    Underline, VAlign, VertAlign,
};
use ss_model::Color;

use crate::record::{u16_at, u32_at};

#[derive(Default)]
pub(crate) struct Styles {
    fonts: Vec<Font>,
    fills: Vec<Fill>,
    borders: Vec<Border>,
    xfs: Vec<CellFormat>,
    codes: BTreeMap<u32, String>,
    /// The workbook's own fifty-six colours, when it overrides the standard
    /// ones. Indices below 8 are fixed and are never in here.
    palette: Vec<[u8; 3]>,
}

impl Styles {
    pub fn new() -> Styles {
        Styles {
            // Index 0 must be the empty fill and index 1 the reserved gray125,
            // the same two every workbook starts with, or a fill added later
            // lands on a number the model reserves.
            fills: vec![Fill::default(), gray125()],
            borders: vec![Border::default()],
            ..Default::default()
        }
    }

    pub fn palette(&mut self, data: &[u8]) {
        let Some(count) = u16_at(data, 0) else { return };
        self.palette = (0..count as usize)
            .filter_map(|i| {
                let at = 2 + i * 4;
                Some([*data.get(at)?, *data.get(at + 1)?, *data.get(at + 2)?])
            })
            .collect();
    }

    pub fn format(&mut self, data: &[u8]) {
        let (Some(id), Some((code, _))) = (u16_at(data, 0), crate::string::long(data, 2)) else {
            return;
        };
        self.codes.insert(id as u32, code);
    }

    pub fn font(&mut self, data: &[u8]) {
        let height = u16_at(data, 0).unwrap_or(200);
        let flags = u16_at(data, 2).unwrap_or(0);
        let color = u16_at(data, 4).unwrap_or(0x7FFF);
        let weight = u16_at(data, 6).unwrap_or(400);
        let script = u16_at(data, 8).unwrap_or(0);
        let underline = data.get(10).copied().unwrap_or(0);
        let name = crate::string::short(data, 14)
            .map(|(text, _)| text)
            .unwrap_or_default();

        self.fonts.push(Font {
            name: if name.is_empty() {
                "Arial".to_string()
            } else {
                name
            },
            // Twips. A 10-point font is stored as 200.
            size: height as f64 / 20.0,
            bold: weight >= 600,
            italic: flags & 0x02 != 0,
            underline: match underline {
                0x01 => Underline::Single,
                0x02 => Underline::Double,
                0x21 => Underline::SingleAccounting,
                0x22 => Underline::DoubleAccounting,
                _ => Underline::None,
            },
            strike: flags & 0x08 != 0,
            color: self.color(color),
            vert_align: match script {
                1 => Some(VertAlign::Superscript),
                2 => Some(VertAlign::Subscript),
                _ => None,
            },
        });
    }

    pub fn xf(&mut self, data: &[u8]) {
        let ifnt = u16_at(data, 0).unwrap_or(0);
        let ifmt = u16_at(data, 2).unwrap_or(0);
        // Byte 4 carries the protection bits: locked, hidden, and whether this
        // XF is a style rather than a cell format.
        let locked = u16_at(data, 4).unwrap_or(0) & 0x0001 != 0;
        let align = data.get(6).copied().unwrap_or(0);
        let rotation = data.get(7).copied().unwrap_or(0);
        let indent = data.get(8).copied().unwrap_or(0);
        let edges = u32_at(data, 10).unwrap_or(0);
        let more = u32_at(data, 14).unwrap_or(0);
        let colors = u16_at(data, 18).unwrap_or(0);

        let border = Border {
            left: self.edge((edges & 0xF) as u8, ((edges >> 16) & 0x7F) as u16),
            right: self.edge(((edges >> 4) & 0xF) as u8, ((edges >> 23) & 0x7F) as u16),
            top: self.edge(((edges >> 8) & 0xF) as u8, (more & 0x7F) as u16),
            bottom: self.edge(((edges >> 12) & 0xF) as u8, ((more >> 7) & 0x7F) as u16),
            diagonal: self.edge(((more >> 21) & 0xF) as u8, ((more >> 14) & 0x7F) as u16),
            // The two diagonal bits sit at the very top of the first word.
            diagonal_up: edges & 0x8000_0000 != 0,
            diagonal_down: edges & 0x4000_0000 != 0,
        };

        let pattern = ((more >> 26) & 0x3F) as u8;
        let fill = Fill {
            pattern: self.pattern(pattern),
            // In a solid fill the *foreground* is the visible colour, and BIFF
            // stores it in the same place either way.
            fg: self.color(colors & 0x7F),
            bg: self.color((colors >> 7) & 0x7F),
        };

        let alignment = Alignment {
            horizontal: match align & 0x07 {
                1 => HAlign::Left,
                2 => HAlign::Center,
                3 => HAlign::Right,
                4 => HAlign::Fill,
                5 => HAlign::Justify,
                6 => HAlign::CenterContinuous,
                7 => HAlign::Distributed,
                _ => HAlign::General,
            },
            vertical: match (align >> 4) & 0x07 {
                0 => VAlign::Top,
                1 => VAlign::Center,
                3 => VAlign::Justify,
                4 => VAlign::Distributed,
                _ => VAlign::Bottom,
            },
            wrap: align & 0x08 != 0,
            shrink: indent & 0x10 != 0,
            indent: (indent & 0x0F) as u32,
            // The same encoding xlsx uses: 1-90 anticlockwise, 91-180 meaning
            // 1-90 clockwise, 255 stacked.
            rotation: rotation as u32,
        };

        let fill = position(&mut self.fills, fill);
        let border = position(&mut self.borders, border);
        self.xfs.push(CellFormat {
            num_fmt_id: ifmt as u32,
            font: self.font_slot(ifnt),
            fill,
            border,
            alignment,
            xf_id: 0,
            quote_prefix: false,
            locked,
        });
    }

    /// The font an `ifnt` selects, working around the missing fourth slot.
    fn font_slot(&self, ifnt: u16) -> u32 {
        let index = if ifnt >= 4 {
            ifnt as u32 - 1
        } else {
            ifnt as u32
        };
        // Every `FONT` record precedes every `XF`, so a reference past the end
        // is the file being wrong rather than this reader being early.
        index.min(self.fonts.len().saturating_sub(1) as u32)
    }

    fn edge(&self, style: u8, color: u16) -> Edge {
        let style = match style {
            1 => BorderStyle::Thin,
            2 => BorderStyle::Medium,
            3 => BorderStyle::Dashed,
            4 => BorderStyle::Dotted,
            5 => BorderStyle::Thick,
            6 => BorderStyle::Double,
            7 => BorderStyle::Hair,
            8 => BorderStyle::MediumDashed,
            9 => BorderStyle::DashDot,
            10 => BorderStyle::MediumDashDot,
            11 => BorderStyle::DashDotDot,
            12 => BorderStyle::MediumDashDotDot,
            13 => BorderStyle::SlantDashDot,
            _ => BorderStyle::None,
        };
        Edge {
            style,
            color: if style.is_none() {
                Color::Auto
            } else {
                self.color(color)
            },
        }
    }

    fn pattern(&self, code: u8) -> Pattern {
        match code {
            0 => Pattern::None,
            1 => Pattern::Solid,
            other => Pattern::Named(
                match other {
                    2 => "mediumGray",
                    3 => "darkGray",
                    4 => "lightGray",
                    5 => "darkHorizontal",
                    6 => "darkVertical",
                    7 => "darkDown",
                    8 => "darkUp",
                    9 => "darkGrid",
                    10 => "darkTrellis",
                    11 => "lightHorizontal",
                    12 => "lightVertical",
                    13 => "lightDown",
                    14 => "lightUp",
                    15 => "lightGrid",
                    16 => "lightTrellis",
                    17 => "gray125",
                    18 => "gray0625",
                    _ => "gray125",
                }
                .to_string(),
            ),
        }
    }

    /// A colour index into a colour.
    ///
    /// Left as [`Color::Indexed`] when the workbook uses the standard palette,
    /// because that is what the model already resolves and what a save to xlsx
    /// should write back. Resolved to RGB only when the file overrode the
    /// palette, since an index then means something no other reader would agree
    /// with.
    fn color(&self, icv: u16) -> Color {
        // 0x7FFF is "automatic" — the window text colour, which is not black
        // and must not be resolved to it.
        if icv == 0x7FFF || icv == 0x0040 || icv == 0x0041 {
            return Color::Auto;
        }
        if icv >= 8 {
            if let Some(rgb) = self.palette.get(icv as usize - 8) {
                return Color::rgb(rgb[0], rgb[1], rgb[2]);
            }
        }
        Color::Indexed(icv as u32)
    }

    pub fn into_parts(self) -> Parts {
        Parts {
            codes: self.codes,
            fonts: self.fonts,
            fills: self.fills,
            borders: self.borders,
            cell_style_xfs: Vec::new(),
            cell_xfs: self.xfs,
            named: Vec::new(),
            dxfs: Vec::new(),
            // No theme: a BIFF workbook has none, and every colour in it is an
            // index into the palette instead.
            theme: ss_model::color::Theme::default(),
        }
    }
}

/// Excel's reserved second fill, present in every workbook whether used or not.
fn gray125() -> Fill {
    Fill {
        pattern: Pattern::Named("gray125".to_string()),
        fg: Color::Auto,
        bg: Color::Auto,
    }
}

/// Where `value` is in `list`, appending it if it is not there yet.
fn position<T: PartialEq>(list: &mut Vec<T>, value: T) -> u32 {
    match list.iter().position(|existing| *existing == value) {
        Some(index) => index as u32,
        None => {
            list.push(value);
            list.len() as u32 - 1
        }
    }
}

/// `DATEMODE`: whether day zero is 1900 or 1904.
pub(crate) fn date_1904(data: &[u8]) -> bool {
    u16_at(data, 0).unwrap_or(0) != 0
}

/// `SCL`: zoom as a fraction, e.g. 90/100.
pub(crate) fn zoom(data: &[u8]) -> Option<f64> {
    let numerator = u16_at(data, 0)? as f64;
    let denominator = u16_at(data, 2)? as f64;
    (denominator > 0.0 && numerator > 0.0).then(|| numerator / denominator)
}
