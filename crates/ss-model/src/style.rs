//! Cell formatting.
//!
//! Excel stores formatting centrally and references it per cell, which is why a
//! workbook with uniform formatting costs almost nothing. We mirror that rather
//! than inlining formatting into cells: a [`StyleId`] is the `s` attribute, and
//! it indexes `cellXfs`.
//!
//! **The tables are index-addressed, and that governs every mutation here.** A
//! font, a fill, a border, and a cell format are all referred to by position, so
//! a new one may only ever be *appended*. Inserting a font at index 2 would
//! silently re-letter every cell in the workbook that used to point past it.
//! Nothing in this module removes or reorders an entry, and `original_*` records
//! how much of each table came out of the file so a writer knows exactly what it
//! has to add.
//!
//! What is deliberately *not* modeled: `family`, `charset`, and `scheme` on a
//! font, gradient fills, and the several attributes of `<xf>` that only affect
//! the formatting dialog. Dropping them is safe because we never rewrite an
//! entry the file wrote — the writer copies those through byte for byte and only
//! appends ones we authored, which by construction have nothing we did not model.

use std::collections::BTreeMap;

use crate::color::{Color, Theme};
use crate::numfmt::NumberFormat;

/// Index into the workbook's style table.
///
/// This is the `s` attribute on a cell, which indexes `cellXfs` in styles.xml.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct StyleId(pub u32);

impl StyleId {
    /// The workbook default, always index 0. A cell with this style is unformatted.
    pub const DEFAULT: StyleId = StyleId(0);
}

/// The first `numFmtId` a file is allowed to define. Everything below it is an
/// Excel built-in and is never written out.
pub const FIRST_CUSTOM_FORMAT_ID: u32 = 164;

/// Excel's default font size, in points.
pub const DEFAULT_FONT_SIZE: f64 = 11.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    /// Drawn at the bottom of the cell rather than under the glyphs. Rendered
    /// the same as its non-accounting twin; the distinction is typographic.
    SingleAccounting,
    DoubleAccounting,
}

impl Underline {
    pub fn from_xml(text: &str) -> Underline {
        match text {
            "double" => Underline::Double,
            "singleAccounting" => Underline::SingleAccounting,
            "doubleAccounting" => Underline::DoubleAccounting,
            "none" => Underline::None,
            _ => Underline::Single,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Underline::None => "none",
            Underline::Single => "single",
            Underline::Double => "double",
            Underline::SingleAccounting => "singleAccounting",
            Underline::DoubleAccounting => "doubleAccounting",
        }
    }

    pub fn is_none(self) -> bool {
        self == Underline::None
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Font {
    pub name: String,
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
    pub underline: Underline,
    pub strike: bool,
    pub color: Color,
    /// Superscript or subscript, as `<vertAlign val="superscript"/>`.
    pub vert_align: Option<VertAlign>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VertAlign {
    Superscript,
    Subscript,
}

impl Default for Font {
    fn default() -> Self {
        Font {
            name: "Calibri".to_string(),
            size: DEFAULT_FONT_SIZE,
            bold: false,
            italic: false,
            underline: Underline::None,
            strike: false,
            color: Color::Auto,
            vert_align: None,
        }
    }
}

/// How a fill is painted.
///
/// The trap is [`Pattern::Solid`]: its visible colour is the *foreground*, not
/// the background. Reading `bgColor` for a solid fill gives white for almost
/// every shaded cell in every real workbook.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Pattern {
    #[default]
    None,
    Solid,
    /// One of the seventeen hatches. Kept by name and drawn as a blend, which is
    /// close enough at cell size and never wrong about which colours are in play.
    Named(String),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Fill {
    pub pattern: Pattern,
    pub fg: Color,
    pub bg: Color,
}

impl Fill {
    pub fn solid(color: Color) -> Fill {
        Fill {
            pattern: Pattern::Solid,
            fg: color,
            bg: Color::Auto,
        }
    }

    /// The colour actually painted, if any.
    pub fn shade(&self, theme: &Theme) -> Option<[u8; 3]> {
        match self.pattern {
            Pattern::None => None,
            Pattern::Solid => self.fg.resolve(theme),
            // A hatch is half one colour and half the other at any distance a
            // cell is read from.
            Pattern::Named(_) => match (self.fg.resolve(theme), self.bg.resolve(theme)) {
                (Some(f), Some(b)) => Some([
                    ((u16::from(f[0]) + u16::from(b[0])) / 2) as u8,
                    ((u16::from(f[1]) + u16::from(b[1])) / 2) as u8,
                    ((u16::from(f[2]) + u16::from(b[2])) / 2) as u8,
                ]),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            },
        }
    }

    pub fn is_none(&self) -> bool {
        self.pattern == Pattern::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderStyle {
    #[default]
    None,
    Hair,
    Thin,
    Medium,
    Thick,
    Double,
    Dotted,
    Dashed,
    DashDot,
    DashDotDot,
    MediumDashed,
    MediumDashDot,
    MediumDashDotDot,
    SlantDashDot,
}

impl BorderStyle {
    pub fn from_xml(text: &str) -> BorderStyle {
        match text {
            "hair" => BorderStyle::Hair,
            "thin" => BorderStyle::Thin,
            "medium" => BorderStyle::Medium,
            "thick" => BorderStyle::Thick,
            "double" => BorderStyle::Double,
            "dotted" => BorderStyle::Dotted,
            "dashed" => BorderStyle::Dashed,
            "dashDot" => BorderStyle::DashDot,
            "dashDotDot" => BorderStyle::DashDotDot,
            "mediumDashed" => BorderStyle::MediumDashed,
            "mediumDashDot" => BorderStyle::MediumDashDot,
            "mediumDashDotDot" => BorderStyle::MediumDashDotDot,
            "slantDashDot" => BorderStyle::SlantDashDot,
            _ => BorderStyle::None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            BorderStyle::None => "none",
            BorderStyle::Hair => "hair",
            BorderStyle::Thin => "thin",
            BorderStyle::Medium => "medium",
            BorderStyle::Thick => "thick",
            BorderStyle::Double => "double",
            BorderStyle::Dotted => "dotted",
            BorderStyle::Dashed => "dashed",
            BorderStyle::DashDot => "dashDot",
            BorderStyle::DashDotDot => "dashDotDot",
            BorderStyle::MediumDashed => "mediumDashed",
            BorderStyle::MediumDashDot => "mediumDashDot",
            BorderStyle::MediumDashDotDot => "mediumDashDotDot",
            BorderStyle::SlantDashDot => "slantDashDot",
        }
    }

    /// How wide to draw it, in logical pixels at 100% zoom.
    pub fn width(self) -> f32 {
        match self {
            BorderStyle::None => 0.0,
            BorderStyle::Hair => 0.5,
            BorderStyle::Thin | BorderStyle::Dotted | BorderStyle::Dashed => 1.0,
            BorderStyle::DashDot | BorderStyle::DashDotDot => 1.0,
            BorderStyle::Medium
            | BorderStyle::MediumDashed
            | BorderStyle::MediumDashDot
            | BorderStyle::MediumDashDotDot
            | BorderStyle::SlantDashDot
            | BorderStyle::Double => 2.0,
            BorderStyle::Thick => 3.0,
        }
    }

    /// The on/off run length for a dashed style, or `None` for a solid one.
    pub fn dash(self) -> Option<(f32, f32)> {
        match self {
            BorderStyle::Dotted => Some((1.0, 2.0)),
            BorderStyle::Hair => Some((1.0, 1.0)),
            BorderStyle::Dashed | BorderStyle::MediumDashed => Some((3.0, 2.0)),
            BorderStyle::DashDot | BorderStyle::MediumDashDot | BorderStyle::SlantDashDot => {
                Some((4.0, 2.0))
            }
            BorderStyle::DashDotDot | BorderStyle::MediumDashDotDot => Some((5.0, 2.0)),
            _ => None,
        }
    }

    pub fn is_none(self) -> bool {
        self == BorderStyle::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Edge {
    pub style: BorderStyle,
    pub color: Color,
}

impl Edge {
    pub fn new(style: BorderStyle) -> Edge {
        Edge {
            style,
            color: Color::Auto,
        }
    }

    pub fn is_none(self) -> bool {
        self.style.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Border {
    pub left: Edge,
    pub right: Edge,
    pub top: Edge,
    pub bottom: Edge,
    pub diagonal: Edge,
    pub diagonal_up: bool,
    pub diagonal_down: bool,
}

impl Border {
    pub fn all(style: BorderStyle) -> Border {
        let edge = Edge::new(style);
        Border {
            left: edge,
            right: edge,
            top: edge,
            bottom: edge,
            ..Default::default()
        }
    }

    pub fn is_none(&self) -> bool {
        self.left.is_none()
            && self.right.is_none()
            && self.top.is_none()
            && self.bottom.is_none()
            && self.diagonal.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HAlign {
    /// Numbers right, text left, booleans and errors centred. Resolved by the
    /// renderer because it depends on the value, not on the style.
    #[default]
    General,
    Left,
    Center,
    Right,
    Fill,
    Justify,
    CenterContinuous,
    Distributed,
}

impl HAlign {
    pub fn from_xml(text: &str) -> HAlign {
        match text {
            "left" => HAlign::Left,
            "center" => HAlign::Center,
            "right" => HAlign::Right,
            "fill" => HAlign::Fill,
            "justify" => HAlign::Justify,
            "centerContinuous" => HAlign::CenterContinuous,
            "distributed" => HAlign::Distributed,
            _ => HAlign::General,
        }
    }

    pub fn as_str(self) -> Option<&'static str> {
        match self {
            HAlign::General => None,
            HAlign::Left => Some("left"),
            HAlign::Center => Some("center"),
            HAlign::Right => Some("right"),
            HAlign::Fill => Some("fill"),
            HAlign::Justify => Some("justify"),
            HAlign::CenterContinuous => Some("centerContinuous"),
            HAlign::Distributed => Some("distributed"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VAlign {
    Top,
    Center,
    #[default]
    Bottom,
    Justify,
    Distributed,
}

impl VAlign {
    pub fn from_xml(text: &str) -> VAlign {
        match text {
            "top" => VAlign::Top,
            "center" => VAlign::Center,
            "justify" => VAlign::Justify,
            "distributed" => VAlign::Distributed,
            _ => VAlign::Bottom,
        }
    }

    pub fn as_str(self) -> Option<&'static str> {
        match self {
            VAlign::Bottom => None,
            VAlign::Top => Some("top"),
            VAlign::Center => Some("center"),
            VAlign::Justify => Some("justify"),
            VAlign::Distributed => Some("distributed"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Alignment {
    pub horizontal: HAlign,
    pub vertical: VAlign,
    pub wrap: bool,
    pub shrink: bool,
    /// In Excel's indent units; one is roughly three characters.
    pub indent: u32,
    /// Degrees anticlockwise, 0-90 and 91-180 (the second range meaning -1 to
    /// -90), or 255 for stacked vertical text. Kept as the file writes it.
    pub rotation: u32,
}

impl Alignment {
    pub fn is_default(&self) -> bool {
        *self == Alignment::default()
    }

    /// Degrees anticlockwise from horizontal, with Excel's 91-180 encoding
    /// unfolded into negatives. `None` for stacked text.
    pub fn degrees(&self) -> Option<f32> {
        match self.rotation {
            255 => None,
            r if r <= 90 => Some(r as f32),
            r if r <= 180 => Some(-((r - 90) as f32)),
            _ => Some(0.0),
        }
    }

    pub fn stacked(&self) -> bool {
        self.rotation == 255
    }
}

/// One entry of `cellXfs`: everything a cell's `s` attribute selects.
#[derive(Debug, Clone, PartialEq)]
pub struct CellFormat {
    /// The `numFmtId`, kept as the file wrote it so a save can put it back
    /// rather than inventing an equivalent one.
    pub num_fmt_id: u32,
    pub font: u32,
    pub fill: u32,
    pub border: u32,
    pub alignment: Alignment,
    /// The named style this one is based on: an index into `cellStyleXfs`.
    pub xf_id: u32,
    /// `quotePrefix="1"` — a leading apostrophe the user typed, which forces
    /// text. Not part of the value, so it has to live in the format.
    pub quote_prefix: bool,
    /// `<protection locked="0"/>` — whether protecting the sheet would stop
    /// this cell being edited.
    ///
    /// Locked by default, as every cell in a new workbook is: locking is the
    /// state a cell starts in, and unlocking the input cells is the deliberate
    /// act. It means nothing at all until the sheet is protected, which is why
    /// a workbook can be full of locked cells nobody has ever noticed.
    pub locked: bool,
}

impl Default for CellFormat {
    /// The unformatted cell — which is locked, because every cell is until
    /// somebody says otherwise. An `<xf>` with no `<protection>` child inherits
    /// exactly this, so the default is also what parsing starts from.
    fn default() -> Self {
        CellFormat {
            num_fmt_id: 0,
            font: 0,
            fill: 0,
            border: 0,
            alignment: Alignment::default(),
            xf_id: 0,
            quote_prefix: false,
            locked: true,
        }
    }
}

/// A named cell style, as the Styles gallery lists them.
#[derive(Debug, Clone)]
pub struct NamedStyle {
    pub name: String,
    /// Index into `cellStyleXfs`.
    pub xf_id: u32,
    /// Excel's identifier for a built-in style, e.g. 0 for Normal.
    pub builtin_id: Option<u32>,
    pub hidden: bool,
}

/// A *differential* format: only the parts of a look that a conditional
/// formatting rule overrides.
///
/// Every field is optional and that is the whole point — a rule that turns text
/// red must not also reset the cell's font, its size, or its fill.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Dxf {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<Underline>,
    pub strike: Option<bool>,
    pub color: Option<Color>,
    pub fill: Option<Fill>,
    pub border: Option<Border>,
    pub number_format: Option<String>,
}

impl Dxf {
    pub fn is_empty(&self) -> bool {
        *self == Dxf::default()
    }
}

/// A style pulled apart into something that can be edited.
///
/// The tables are index-addressed and append-only, so "make this cell bold" is
/// not a mutation: it is *look up, change a field, ask for the style that has
/// that look*. This type is the middle step.
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    pub number_format: String,
    pub font: Font,
    pub fill: Fill,
    pub border: Border,
    pub alignment: Alignment,
    pub quote_prefix: bool,
    pub locked: bool,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            number_format: "General".to_string(),
            font: Font::default(),
            fill: Fill::default(),
            border: Border::default(),
            alignment: Alignment::default(),
            quote_prefix: false,
            locked: true,
        }
    }
}

/// Everything `styles.xml` said, as a reader hands it over.
#[derive(Debug, Clone, Default)]
pub struct Parts {
    pub codes: BTreeMap<u32, String>,
    pub fonts: Vec<Font>,
    pub fills: Vec<Fill>,
    pub borders: Vec<Border>,
    pub cell_style_xfs: Vec<CellFormat>,
    pub cell_xfs: Vec<CellFormat>,
    pub named: Vec<NamedStyle>,
    pub dxfs: Vec<Dxf>,
    pub theme: Theme,
}

/// The formatting a workbook's cells can refer to.
#[derive(Debug, Clone)]
pub struct StyleTable {
    /// Distinct parsed number formats. Several styles usually share one.
    formats: Vec<NumberFormat>,
    /// Which entry of `formats` each [`StyleId`] uses.
    by_style: Vec<u32>,
    /// Custom format codes by id, as read and as added since.
    codes: BTreeMap<u32, String>,
    entries: Vec<CellFormat>,
    fonts: Vec<Font>,
    fills: Vec<Fill>,
    borders: Vec<Border>,
    cell_style_xfs: Vec<CellFormat>,
    named: Vec<NamedStyle>,
    dxfs: Vec<Dxf>,
    theme: Theme,
    /// How many of each table came out of the file. Everything past these is
    /// ours to write.
    original: Counts,
    /// General, returned for any style the table does not cover.
    fallback: NumberFormat,
    default_font: Font,
    default_fill: Fill,
    default_border: Border,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub fonts: usize,
    pub fills: usize,
    pub borders: usize,
    pub cell_xfs: usize,
}

impl Default for StyleTable {
    fn default() -> Self {
        StyleTable::from_parts(Parts::default())
    }
}

impl StyleTable {
    /// Builds a table from number formats alone.
    ///
    /// The font, fill, and border tables come out seeded exactly as a new
    /// workbook's do — one font, *two* fills, one border — so a model built this
    /// way lines up index for index with the package `write::blank` authors.
    pub fn build(codes: &BTreeMap<u32, String>, style_format_ids: &[u32]) -> Self {
        StyleTable::from_parts(Parts {
            codes: codes.clone(),
            cell_xfs: style_format_ids
                .iter()
                .map(|id| CellFormat {
                    num_fmt_id: *id,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })
    }

    /// Builds from everything styles.xml said.
    pub fn from_parts(parts: Parts) -> Self {
        let Parts {
            codes,
            mut fonts,
            mut fills,
            mut borders,
            cell_style_xfs,
            cell_xfs,
            named,
            dxfs,
            theme,
        } = parts;

        if fonts.is_empty() {
            fonts.push(Font::default());
        }
        if fills.is_empty() {
            // Index 1 is reserved for gray125 in every workbook Excel writes,
            // and a file whose fills table is shorter than two is rejected.
            fills.push(Fill::default());
            fills.push(Fill {
                pattern: Pattern::Named("gray125".to_string()),
                ..Default::default()
            });
        }
        if borders.is_empty() {
            borders.push(Border::default());
        }

        let mut table = StyleTable {
            formats: Vec::new(),
            by_style: Vec::with_capacity(cell_xfs.len()),
            codes,
            original: Counts {
                fonts: fonts.len(),
                fills: fills.len(),
                borders: borders.len(),
                cell_xfs: cell_xfs.len(),
            },
            entries: cell_xfs,
            fonts,
            fills,
            borders,
            cell_style_xfs,
            named,
            dxfs,
            theme,
            fallback: NumberFormat::general(),
            default_font: Font::default(),
            default_fill: Fill::default(),
            default_border: Border::default(),
        };

        let mut seen: BTreeMap<u32, u32> = BTreeMap::new();
        for index in 0..table.entries.len() {
            let id = table.entries[index].num_fmt_id;
            let slot = match seen.get(&id) {
                Some(slot) => *slot,
                None => {
                    let code = table.code_for(id).to_string();
                    table.formats.push(NumberFormat::parse(&code));
                    let slot = table.formats.len() as u32 - 1;
                    seen.insert(id, slot);
                    slot
                }
            };
            table.by_style.push(slot);
        }
        table
    }

    /// The format code an id names, resolving Excel's built-ins.
    ///
    /// Most formats are never written down: `numFmtId="14"` is a date and the
    /// file says nothing more. A table that only honours codes it can see shows
    /// every date in every document as a five-digit serial.
    fn code_for(&self, id: u32) -> &str {
        self.codes
            .get(&id)
            .map(String::as_str)
            .or_else(|| NumberFormat::builtin(id))
            .unwrap_or("General")
    }

    /// The `numFmtId` a style names, for a writer putting styles.xml back.
    pub fn format_id(&self, style: StyleId) -> u32 {
        self.entries
            .get(style.0 as usize)
            .map_or(0, |xf| xf.num_fmt_id)
    }

    /// Custom format codes by id — everything a writer has to emit as `<numFmt>`.
    pub fn codes(&self) -> &BTreeMap<u32, String> {
        &self.codes
    }

    /// The number format a cell should display through.
    ///
    /// A style we have never heard of gets General rather than an error — a cell
    /// must always show something, and a file with a dangling style index is
    /// still a file the user wants to read.
    pub fn number_format(&self, style: StyleId) -> &NumberFormat {
        self.by_style
            .get(style.0 as usize)
            .and_then(|slot| self.formats.get(*slot as usize))
            .unwrap_or(&self.fallback)
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn format_of(&self, style: StyleId) -> Option<&CellFormat> {
        self.entries.get(style.0 as usize)
    }

    pub fn font(&self, style: StyleId) -> &Font {
        self.format_of(style)
            .and_then(|xf| self.fonts.get(xf.font as usize))
            .unwrap_or(&self.default_font)
    }

    pub fn fill(&self, style: StyleId) -> &Fill {
        self.format_of(style)
            .and_then(|xf| self.fills.get(xf.fill as usize))
            .unwrap_or(&self.default_fill)
    }

    pub fn border(&self, style: StyleId) -> &Border {
        self.format_of(style)
            .and_then(|xf| self.borders.get(xf.border as usize))
            .unwrap_or(&self.default_border)
    }

    pub fn alignment(&self, style: StyleId) -> Alignment {
        self.format_of(style)
            .map(|xf| xf.alignment)
            .unwrap_or_default()
    }

    pub fn dxf(&self, index: u32) -> Option<&Dxf> {
        self.dxfs.get(index as usize)
    }

    pub fn dxfs(&self) -> &[Dxf] {
        &self.dxfs
    }

    /// Adds a differential format and returns the index that names it.
    ///
    /// Deduplicated, because the same override arriving twice — a conditional
    /// format copied to another range, a table given the style of its
    /// neighbour — should not grow the table it is written back from.
    pub fn add_dxf(&mut self, dxf: Dxf) -> u32 {
        match self.dxfs.iter().position(|existing| *existing == dxf) {
            Some(index) => index as u32,
            None => {
                self.dxfs.push(dxf);
                self.dxfs.len() as u32 - 1
            }
        }
    }

    pub fn named_styles(&self) -> &[NamedStyle] {
        &self.named
    }

    /// The style a named entry applies, if the workbook has it.
    ///
    /// A named style points into `cellStyleXfs`, which a cell's `s` attribute
    /// does *not* index — so applying one means finding or appending a `cellXfs`
    /// entry with the same look.
    pub fn look_of_named(&self, name: &str) -> Option<Look> {
        let named = self
            .named
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))?;
        let xf = self.cell_style_xfs.get(named.xf_id as usize)?;
        Some(self.look_of(xf))
    }

    fn look_of(&self, xf: &CellFormat) -> Look {
        Look {
            number_format: self.code_for(xf.num_fmt_id).to_string(),
            font: self
                .fonts
                .get(xf.font as usize)
                .cloned()
                .unwrap_or_default(),
            fill: self
                .fills
                .get(xf.fill as usize)
                .cloned()
                .unwrap_or_default(),
            border: self
                .borders
                .get(xf.border as usize)
                .copied()
                .unwrap_or_default(),
            alignment: xf.alignment,
            quote_prefix: xf.quote_prefix,
            locked: xf.locked,
        }
    }

    /// A style's formatting, pulled out so it can be edited.
    pub fn look(&self, style: StyleId) -> Look {
        match self.format_of(style) {
            Some(xf) => self.look_of(xf),
            None => Look::default(),
        }
    }

    /// The style with this look, reusing an existing one or appending.
    pub fn style_for(&mut self, look: &Look) -> StyleId {
        let num_fmt_id = self.format_id_for(&look.number_format);
        let font = position(&mut self.fonts, &look.font);
        let fill = position(&mut self.fills, &look.fill);
        let border = position(&mut self.borders, &look.border);

        let wanted = CellFormat {
            num_fmt_id,
            font,
            fill,
            border,
            alignment: look.alignment,
            xf_id: 0,
            quote_prefix: look.quote_prefix,
            locked: look.locked,
        };
        if let Some(index) = self.entries.iter().position(|xf| {
            xf.num_fmt_id == wanted.num_fmt_id
                && xf.font == wanted.font
                && xf.fill == wanted.fill
                && xf.border == wanted.border
                && xf.alignment == wanted.alignment
                && xf.quote_prefix == wanted.quote_prefix
                && xf.locked == wanted.locked
        }) {
            return StyleId(index as u32);
        }

        self.entries.push(wanted);
        let code = self.code_for(num_fmt_id).to_string();
        let slot =
            match (0..self.entries.len() - 1).find(|i| self.entries[*i].num_fmt_id == num_fmt_id) {
                Some(existing) => self.by_style[existing],
                None => {
                    self.formats.push(NumberFormat::parse(&code));
                    self.formats.len() as u32 - 1
                }
            };
        self.by_style.push(slot);
        StyleId(self.entries.len() as u32 - 1)
    }

    /// Derives a style from an existing one by changing part of its look.
    ///
    /// This is what every formatting command is: bold is `restyle(s, |l|
    /// l.font.bold = true)`. The original style is untouched, because other
    /// cells point at it.
    pub fn restyle(&mut self, style: StyleId, edit: impl FnOnce(&mut Look)) -> StyleId {
        let mut look = self.look(style);
        edit(&mut look);
        self.style_for(&look)
    }

    /// A style that displays through `code`, reusing an existing one if there is
    /// a match and adding one otherwise.
    ///
    /// This is what typing a date into an empty cell needs. Excel does the same
    /// thing: the value it stores is the serial `45306`, and what makes the cell
    /// read as a date is a style pointing at a date format.
    /// Derived from the workbook default rather than from nothing, so a date
    /// typed into a workbook whose base font is Cambria stays Cambria. Building
    /// it from `Look::default()` would append a Calibri font to every file
    /// whose default is anything else.
    pub fn style_for_format(&mut self, code: &str) -> StyleId {
        self.restyle(StyleId::DEFAULT, |look| {
            look.number_format = code.to_string()
        })
    }

    /// The `numFmtId` for a code: an existing declaration, an Excel built-in, or
    /// a freshly allocated custom id.
    fn format_id_for(&mut self, code: &str) -> u32 {
        if let Some(id) = self
            .codes
            .iter()
            .find(|(_, existing)| existing.as_str() == code)
            .map(|(id, _)| *id)
        {
            return id;
        }
        if let Some(id) =
            (0..FIRST_CUSTOM_FORMAT_ID).find(|id| NumberFormat::builtin(*id) == Some(code))
        {
            return id;
        }
        let next = self
            .codes
            .keys()
            .copied()
            .filter(|id| *id >= FIRST_CUSTOM_FORMAT_ID)
            .max()
            .map_or(FIRST_CUSTOM_FORMAT_ID, |max| max + 1);
        self.codes.insert(next, code.to_string());
        next
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    // --- What a writer needs. Everything past `original` is ours to append. ---

    pub fn original(&self) -> Counts {
        self.original
    }

    pub fn fonts(&self) -> &[Font] {
        &self.fonts
    }

    pub fn fills(&self) -> &[Fill] {
        &self.fills
    }

    pub fn borders(&self) -> &[Border] {
        &self.borders
    }

    pub fn entries(&self) -> &[CellFormat] {
        &self.entries
    }
}

/// The index of `wanted` in `table`, appending it if it is not there.
///
/// Append-only on purpose: these tables are addressed by position.
fn position<T: Clone + PartialEq>(table: &mut Vec<T>, wanted: &T) -> u32 {
    match table.iter().position(|existing| existing == wanted) {
        Some(index) => index as u32,
        None => {
            table.push(wanted.clone());
            table.len() as u32 - 1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numfmt::FormatValue;

    #[test]
    fn default_style_is_index_zero() {
        assert_eq!(StyleId::default(), StyleId::DEFAULT);
        assert_eq!(StyleId::DEFAULT.0, 0);
    }

    #[test]
    fn builtin_ids_resolve_without_the_file_saying_anything() {
        // Style 1 is `numFmtId="14"`, a date, and the file carries no code for it.
        let table = StyleTable::build(&BTreeMap::new(), &[0, 14]);
        let shown = |style: u32| {
            table
                .number_format(StyleId(style))
                .format(FormatValue::Number(45352.0))
                .text
        };
        assert_eq!(shown(0), "45352", "General shows the serial");
        assert_eq!(shown(1), "03-01-24", "id 14 is a date");
    }

    #[test]
    fn custom_codes_win_over_the_builtin_table() {
        let codes = BTreeMap::from([(164, "0.000".to_string())]);
        let table = StyleTable::build(&codes, &[164]);
        assert_eq!(
            table
                .number_format(StyleId(0))
                .format(FormatValue::Number(1.5))
                .text,
            "1.500"
        );
    }

    #[test]
    fn styles_sharing_a_format_share_one_parse() {
        let table = StyleTable::build(&BTreeMap::new(), &[14, 14, 14, 0]);
        assert_eq!(table.len(), 4);
        assert_eq!(table.formats.len(), 2, "one parse per distinct format");
    }

    #[test]
    fn asking_for_a_format_reuses_a_style_that_already_has_it() {
        let mut table = StyleTable::build(&BTreeMap::new(), &[0, 14]);
        let before = table.len();
        assert_eq!(table.style_for_format("General"), StyleId(0));
        assert_eq!(table.len(), before, "no new style for one that exists");
    }

    #[test]
    fn a_new_format_gets_a_custom_id_a_writer_can_emit() {
        let mut table = StyleTable::build(&BTreeMap::new(), &[0]);
        let style = table.style_for_format("0.000");
        assert_eq!(
            table
                .number_format(style)
                .format(FormatValue::Number(1.5))
                .text,
            "1.500"
        );
        assert_eq!(table.format_id(style), FIRST_CUSTOM_FORMAT_ID);
        assert_eq!(
            table
                .codes()
                .get(&FIRST_CUSTOM_FORMAT_ID)
                .map(String::as_str),
            Some("0.000")
        );

        // A second style for the same code shares the id rather than inventing one.
        let again = table.style_for_format("0.000");
        assert_eq!(again, style);
    }

    #[test]
    fn a_builtin_code_is_recognized_rather_than_redefined() {
        // `mm-dd-yy` is built-in id 14. Writing it as a custom format would be
        // legal but would make our files differ from Excel's for no reason.
        let mut table = StyleTable::build(&BTreeMap::new(), &[0]);
        let style = table.style_for_format("mm-dd-yy");
        assert_eq!(table.format_id(style), 14);
        assert!(table.codes().is_empty());
    }

    #[test]
    fn a_dangling_style_index_falls_back_to_general() {
        // Files do contain these. Showing the value is better than showing an
        // error the user cannot act on.
        let table = StyleTable::build(&BTreeMap::new(), &[0]);
        assert_eq!(
            table
                .number_format(StyleId(99))
                .format(FormatValue::Number(1.5))
                .text,
            "1.5"
        );
        assert_eq!(table.font(StyleId(99)), &Font::default());
    }

    #[test]
    fn bolding_a_cell_appends_a_font_and_a_format_and_moves_nothing() {
        let mut table = StyleTable::build(&BTreeMap::new(), &[0, 14]);
        let before = table.original();
        let bold = table.restyle(StyleId(1), |look| look.font.bold = true);

        assert_eq!(bold, StyleId(2), "appended, never inserted");
        assert!(table.font(bold).bold);
        assert_eq!(
            table.format_id(bold),
            14,
            "the date format came along with it"
        );
        assert_eq!(table.fonts().len(), 2);
        assert_eq!(
            table.original(),
            before,
            "what the file had does not change when we add"
        );
        // The style it was derived from is untouched: other cells point at it.
        assert!(!table.font(StyleId(1)).bold);
    }

    #[test]
    fn asking_twice_for_the_same_look_gives_the_same_style() {
        let mut table = StyleTable::build(&BTreeMap::new(), &[0]);
        let first = table.restyle(StyleId::DEFAULT, |l| l.font.italic = true);
        let second = table.restyle(StyleId::DEFAULT, |l| l.font.italic = true);
        assert_eq!(first, second);
        assert_eq!(table.len(), 2, "one style added, not two");
        assert_eq!(table.fonts().len(), 2, "and one font");
    }

    #[test]
    fn unbolding_finds_its_way_back_to_the_style_it_came_from() {
        let mut table = StyleTable::build(&BTreeMap::new(), &[0]);
        let bold = table.restyle(StyleId::DEFAULT, |l| l.font.bold = true);
        let plain = table.restyle(bold, |l| l.font.bold = false);
        assert_eq!(plain, StyleId::DEFAULT, "not a third style");
    }

    #[test]
    fn a_new_workbooks_tables_line_up_with_the_package_we_author() {
        // `write::blank` writes one font, two fills, and one border. If the model
        // seeded a different number, the first appended fill would land on an
        // index the file already used and recolour cells nobody touched.
        let table = StyleTable::build(&BTreeMap::new(), &[0]);
        assert_eq!(table.original().fonts, 1);
        assert_eq!(table.original().fills, 2, "index 1 is reserved for gray125");
        assert_eq!(table.original().borders, 1);
        assert_eq!(table.original().cell_xfs, 1);
    }

    #[test]
    fn a_solid_fill_shows_its_foreground_not_its_background() {
        // The single most common formatting bug in xlsx readers: for
        // `patternType="solid"` the visible colour is `fgColor`. Reading
        // `bgColor` gives white for almost every shaded cell in every workbook.
        let fill = Fill {
            pattern: Pattern::Solid,
            fg: Color::rgb(0xFF, 0xEB, 0x9C),
            bg: Color::rgb(0xFF, 0xFF, 0xFF),
        };
        assert_eq!(
            fill.shade(&Theme::default()),
            Some([0xFF, 0xEB, 0x9C]),
            "the foreground is what is painted"
        );
    }

    #[test]
    fn alignment_unfolds_excels_rotation_encoding() {
        // 91-180 means -1 to -90 degrees. Read literally, a cell rotated 45
        // degrees down would be drawn rotated 135 degrees up.
        let rotated = |r: u32| Alignment {
            rotation: r,
            ..Default::default()
        };
        assert_eq!(rotated(45).degrees(), Some(45.0));
        assert_eq!(rotated(135).degrees(), Some(-45.0));
        assert_eq!(rotated(255).degrees(), None, "stacked, not rotated");
        assert!(rotated(255).stacked());
    }

    #[test]
    fn a_named_style_resolves_through_the_table_a_cell_does_not_index() {
        let mut parts = Parts {
            cell_xfs: vec![CellFormat::default()],
            cell_style_xfs: vec![
                CellFormat::default(),
                CellFormat {
                    font: 1,
                    ..Default::default()
                },
            ],
            fonts: vec![
                Font::default(),
                Font {
                    bold: true,
                    size: 15.0,
                    ..Font::default()
                },
            ],
            ..Default::default()
        };
        parts.named.push(NamedStyle {
            name: "Heading 1".to_string(),
            xf_id: 1,
            builtin_id: Some(16),
            hidden: false,
        });
        let mut table = StyleTable::from_parts(parts);

        let look = table.look_of_named("heading 1").expect("named style");
        assert!(look.font.bold);
        let applied = table.style_for(&look);
        assert_eq!(applied, StyleId(1), "appended to cellXfs, where `s` points");
        assert_eq!(table.font(applied).size, 15.0);
    }
}
