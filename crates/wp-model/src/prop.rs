//! Run and paragraph properties — `<w:rPr>` and `<w:pPr>`.
//!
//! Three rules govern everything in this module, and all three are invisible
//! until they are wrong.
//!
//! **Absent is not false.** Every field is an `Option`, and `None` means
//! *inherit from the level above*, not *off*. A model that stores `bool` has
//! already destroyed the difference between "this style says no" and "this style
//! does not say", which is the difference between a heading that is bold and one
//! that is not.
//!
//! **A bare element is true.** `<w:b/>` means bold. `<w:b w:val="0"/>` means not
//! bold. So does `w:val="false"`, `"off"`, and — in files written before 2007 —
//! nothing at all. [`on_off`] is the one place that decides.
//!
//! **Some properties toggle rather than override.** Bold applied by a style on
//! top of a style that is already bold comes out *not* bold: the values XOR
//! through the style hierarchy, and only direct formatting is absolute. That is
//! why applying Word's Strong character style to a heading un-bolds it, and it
//! is the single most surprising rule in the format. See [`Toggles`].

use std::sync::Arc;

use crate::color::{Color, Highlight};
use crate::style::StyleId;
use crate::units::{HalfPoint, Line240, Twips};

/// Reads the `w:val` of an on/off element.
///
/// `None` for the attribute itself being absent means **true**, because
/// `<w:b/>` is how Word writes bold. This function is given whatever the
/// attribute held, so `on_off(None)` is the bare element.
pub fn on_off(val: Option<&str>) -> bool {
    match val {
        None => true,
        Some(text) => !matches!(text.trim(), "0" | "false" | "off"),
    }
}

/// The properties whose values XOR through the style hierarchy.
///
/// The list is closed — it is exactly the twelve elements ECMA-376 §17.7.3 names
/// — and nothing else in the format behaves this way. `w:rtl`, `w:noProof` and
/// the rest are ordinary on/off properties where the nearest value wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum Toggle {
    Bold = 1 << 0,
    BoldCs = 1 << 1,
    Italic = 1 << 2,
    ItalicCs = 1 << 3,
    Caps = 1 << 4,
    SmallCaps = 1 << 5,
    Strike = 1 << 6,
    DoubleStrike = 1 << 7,
    Outline = 1 << 8,
    Shadow = 1 << 9,
    Emboss = 1 << 10,
    Imprint = 1 << 11,
    /// `w:vanish` — hidden text. A toggle, which means hiding hidden text by
    /// applying a hidden style to it shows it again.
    Vanish = 1 << 12,
}

impl Toggle {
    pub const ALL: [Toggle; 13] = [
        Toggle::Bold,
        Toggle::BoldCs,
        Toggle::Italic,
        Toggle::ItalicCs,
        Toggle::Caps,
        Toggle::SmallCaps,
        Toggle::Strike,
        Toggle::DoubleStrike,
        Toggle::Outline,
        Toggle::Shadow,
        Toggle::Emboss,
        Toggle::Imprint,
        Toggle::Vanish,
    ];

    pub fn from_element(local_name: &str) -> Option<Toggle> {
        Some(match local_name {
            "b" => Toggle::Bold,
            "bCs" => Toggle::BoldCs,
            "i" => Toggle::Italic,
            "iCs" => Toggle::ItalicCs,
            "caps" => Toggle::Caps,
            "smallCaps" => Toggle::SmallCaps,
            "strike" => Toggle::Strike,
            "dstrike" => Toggle::DoubleStrike,
            "outline" => Toggle::Outline,
            "shadow" => Toggle::Shadow,
            "emboss" => Toggle::Emboss,
            "imprint" => Toggle::Imprint,
            "vanish" => Toggle::Vanish,
            _ => return None,
        })
    }

    pub const fn element(self) -> &'static str {
        match self {
            Toggle::Bold => "b",
            Toggle::BoldCs => "bCs",
            Toggle::Italic => "i",
            Toggle::ItalicCs => "iCs",
            Toggle::Caps => "caps",
            Toggle::SmallCaps => "smallCaps",
            Toggle::Strike => "strike",
            Toggle::DoubleStrike => "dstrike",
            Toggle::Outline => "outline",
            Toggle::Shadow => "shadow",
            Toggle::Emboss => "emboss",
            Toggle::Imprint => "imprint",
            Toggle::Vanish => "vanish",
        }
    }
}

/// The thirteen toggles of one `<w:rPr>`, as two bitfields.
///
/// `stated` records which the level said anything about; `on` records what it
/// said. Two words rather than thirteen `Option<bool>`s, which matters because a
/// [`RunProps`] exists per run and a long document has tens of thousands.
///
/// The representation is also what makes the layering rules one line each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Toggles {
    stated: u16,
    /// Invariant: no bit is set here that is not also set in `stated`.
    on: u16,
}

impl Toggles {
    pub const fn new() -> Toggles {
        Toggles { stated: 0, on: 0 }
    }

    pub fn get(self, toggle: Toggle) -> Option<bool> {
        let bit = toggle as u16;
        (self.stated & bit != 0).then_some(self.on & bit != 0)
    }

    /// What a level's value means once nothing further can change it.
    pub fn is_on(self, toggle: Toggle) -> bool {
        self.on & toggle as u16 != 0
    }

    pub fn set(&mut self, toggle: Toggle, on: bool) {
        let bit = toggle as u16;
        self.stated |= bit;
        if on {
            self.on |= bit;
        } else {
            self.on &= !bit;
        }
    }

    pub fn clear(&mut self, toggle: Toggle) {
        let bit = toggle as u16;
        self.stated &= !bit;
        self.on &= !bit;
    }

    pub fn is_empty(self) -> bool {
        self.stated == 0
    }

    /// Layers a *style's* toggles over these — the XOR rule.
    ///
    /// Bold over bold is not bold. A `false` in a style is the identity, because
    /// XOR with false changes nothing; that is not an oversight but the rule,
    /// and it is why a style cannot un-bold what another style bolded. Only
    /// direct formatting can do that.
    pub fn layer_style(&mut self, over: Toggles) {
        // The `on` invariant is what lets this be a bare XOR: bits `over` did
        // not state are zero there, and XOR with zero keeps ours.
        self.on ^= over.on;
        self.stated |= over.stated;
    }

    /// Layers *direct formatting* over these — the nearest value wins outright.
    pub fn layer_direct(&mut self, over: Toggles) {
        self.on = (self.on & !over.stated) | over.on;
        self.stated |= over.stated;
    }
}

/// How one level of formatting sits on top of another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    /// A style in the hierarchy: everything overrides except the toggles, which
    /// XOR.
    Style,
    /// Direct formatting on the run or paragraph: everything overrides.
    Direct,
}

/// `<w:rFonts>` — up to four faces at once, chosen per character by script.
///
/// A run does not have *a* font. It has an ASCII font, a high-ANSI font, an East
/// Asian font and a complex-script font, and which one draws a given character
/// depends on that character's code point and on `w:hint`. Modelling this as one
/// name is the mistake that makes CJK text in a Latin document render in the
/// wrong face.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Fonts {
    pub ascii: Option<Arc<str>>,
    pub high_ansi: Option<Arc<str>>,
    pub east_asian: Option<Arc<str>>,
    pub complex: Option<Arc<str>>,
    /// `w:asciiTheme="minorHAnsi"` and friends. Modern Word writes the theme
    /// reference *and* a cached name; the reference is the one that is right
    /// after a theme change, so it wins where both are present.
    pub ascii_theme: Option<ThemeFont>,
    pub high_ansi_theme: Option<ThemeFont>,
    pub east_asian_theme: Option<ThemeFont>,
    pub complex_theme: Option<ThemeFont>,
    /// `w:hint` — which of the four to use for characters the code point does
    /// not settle. Ambiguous punctuation in a CJK document is the case.
    pub hint: Option<Script>,
}

impl Fonts {
    pub fn is_empty(&self) -> bool {
        *self == Fonts::default()
    }

    fn layer(&mut self, over: &Fonts) {
        fn take(slot: &mut Option<Arc<str>>, over: &Option<Arc<str>>) {
            if over.is_some() {
                slot.clone_from(over);
            }
        }
        take(&mut self.ascii, &over.ascii);
        take(&mut self.high_ansi, &over.high_ansi);
        take(&mut self.east_asian, &over.east_asian);
        take(&mut self.complex, &over.complex);
        self.ascii_theme = over.ascii_theme.or(self.ascii_theme);
        self.high_ansi_theme = over.high_ansi_theme.or(self.high_ansi_theme);
        self.east_asian_theme = over.east_asian_theme.or(self.east_asian_theme);
        self.complex_theme = over.complex_theme.or(self.complex_theme);
        self.hint = over.hint.or(self.hint);
    }
}

/// A reference into the theme's font scheme rather than a font name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeFont {
    MajorAscii,
    MajorHighAnsi,
    MajorEastAsian,
    MajorComplex,
    MinorAscii,
    MinorHighAnsi,
    MinorEastAsian,
    MinorComplex,
}

impl ThemeFont {
    pub fn from_val(text: &str) -> Option<ThemeFont> {
        Some(match text {
            "majorAscii" => ThemeFont::MajorAscii,
            "majorHAnsi" => ThemeFont::MajorHighAnsi,
            "majorEastAsia" => ThemeFont::MajorEastAsian,
            "majorBidi" => ThemeFont::MajorComplex,
            "minorAscii" => ThemeFont::MinorAscii,
            "minorHAnsi" => ThemeFont::MinorHighAnsi,
            "minorEastAsia" => ThemeFont::MinorEastAsian,
            "minorBidi" => ThemeFont::MinorComplex,
            _ => return None,
        })
    }

    /// Headings use the major scheme, body text the minor one.
    pub const fn is_major(self) -> bool {
        matches!(
            self,
            ThemeFont::MajorAscii
                | ThemeFont::MajorHighAnsi
                | ThemeFont::MajorEastAsian
                | ThemeFont::MajorComplex
        )
    }
}

/// Which of a run's four faces a character is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    Ascii,
    HighAnsi,
    EastAsian,
    Complex,
}

impl Script {
    pub fn from_val(text: &str) -> Option<Script> {
        Some(match text {
            "default" => Script::Ascii,
            "eastAsia" => Script::EastAsian,
            "cs" => Script::Complex,
            _ => return None,
        })
    }
}

/// `<w:u>` — eighteen line styles, of which Word's UI offers most.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnderlineKind {
    #[default]
    None,
    Single,
    Double,
    Thick,
    Dotted,
    DottedHeavy,
    Dash,
    DashedHeavy,
    DashLong,
    DashLongHeavy,
    DotDash,
    DashDotHeavy,
    DotDotDash,
    DashDotDotHeavy,
    Wave,
    WavyHeavy,
    WavyDouble,
    /// Underlines the words and not the spaces between them.
    Words,
}

impl UnderlineKind {
    pub fn from_val(text: &str) -> Option<UnderlineKind> {
        Some(match text {
            "none" => UnderlineKind::None,
            "single" => UnderlineKind::Single,
            "double" => UnderlineKind::Double,
            "thick" => UnderlineKind::Thick,
            "dotted" => UnderlineKind::Dotted,
            "dottedHeavy" => UnderlineKind::DottedHeavy,
            "dash" => UnderlineKind::Dash,
            "dashedHeavy" => UnderlineKind::DashedHeavy,
            "dashLong" => UnderlineKind::DashLong,
            "dashLongHeavy" => UnderlineKind::DashLongHeavy,
            "dotDash" => UnderlineKind::DotDash,
            "dashDotHeavy" => UnderlineKind::DashDotHeavy,
            "dotDotDash" => UnderlineKind::DotDotDash,
            "dashDotDotHeavy" => UnderlineKind::DashDotDotHeavy,
            "wave" => UnderlineKind::Wave,
            "wavyHeavy" => UnderlineKind::WavyHeavy,
            "wavyDouble" => UnderlineKind::WavyDouble,
            "words" => UnderlineKind::Words,
            _ => return None,
        })
    }

    pub const fn draws(self) -> bool {
        !matches!(self, UnderlineKind::None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Underline {
    pub kind: UnderlineKind,
    /// Absent means the run's own colour, which is not the same as `auto`.
    pub color: Option<Color>,
}

/// `<w:vertAlign>` — superscript and subscript, which are a *position* rather
/// than a size, and Word shrinks the glyphs itself rather than storing a size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VertAlign {
    #[default]
    Baseline,
    Superscript,
    Subscript,
}

impl VertAlign {
    pub fn from_val(text: &str) -> Option<VertAlign> {
        Some(match text {
            "baseline" => VertAlign::Baseline,
            "superscript" => VertAlign::Superscript,
            "subscript" => VertAlign::Subscript,
            _ => return None,
        })
    }
}

/// One run's formatting, as one level states it.
///
/// This is *not* a resolved appearance — see [`crate::style::Resolved`]. Every
/// field absent is the normal case: most runs in a real document carry an empty
/// `<w:rPr>` or none at all and take everything from their paragraph's style.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunProps {
    /// `<w:rStyle>` — the character style, which sits between the paragraph
    /// style and direct formatting.
    pub style: Option<StyleId>,
    pub fonts: Fonts,
    pub toggles: Toggles,
    /// `<w:sz>`, in half-points.
    pub size: Option<HalfPoint>,
    /// `<w:szCs>`. A complex-script run takes its size from here, and a document
    /// that sets only one of the two renders Arabic at the wrong size.
    pub size_complex: Option<HalfPoint>,
    pub color: Option<Color>,
    pub underline: Option<Underline>,
    pub highlight: Option<Highlight>,
    pub vert_align: Option<VertAlign>,
    /// `<w:spacing>` on a run: extra space *between characters*, in twips. The
    /// same element name as a paragraph's line spacing, meaning something
    /// unrelated.
    pub letter_spacing: Option<Twips>,
    /// `<w:w>` — horizontal glyph scale as a whole percentage. 100 is normal.
    pub scale: Option<u16>,
    /// `<w:position>` — raised or lowered from the baseline, in half-points.
    pub raise: Option<HalfPoint>,
    /// `<w:kern>` — the size *at or above which* kerning is applied, not an
    /// amount. Zero is off.
    pub kern: Option<HalfPoint>,
    pub shading: Option<Shading>,
    /// `<w:bdr>` — a border around the run itself, not the paragraph.
    pub border: Option<Border>,
    /// `<w:rtl>`. Not a toggle: the nearest value wins.
    pub rtl: Option<bool>,
    pub no_proof: Option<bool>,
    /// `<w:lang>` — three of them, one per script, and the spell checker and the
    /// line breaker both need it.
    pub lang: Option<Lang>,
}

/// `<w:lang>` — language tags per script family.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Lang {
    pub value: Option<Arc<str>>,
    pub east_asian: Option<Arc<str>>,
    pub complex: Option<Arc<str>>,
}

impl RunProps {
    pub fn new() -> RunProps {
        RunProps::default()
    }

    pub fn is_empty(&self) -> bool {
        *self == RunProps::default()
    }

    /// Lays `over` on top of these, by the rules of `layer`.
    ///
    /// Only the toggles distinguish the two layers. Everything else overrides in
    /// both, which is the ordinary "nearest wins" and needs no special case.
    pub fn layer(&mut self, over: &RunProps, layer: Layer) {
        match layer {
            Layer::Style => self.toggles.layer_style(over.toggles),
            Layer::Direct => self.toggles.layer_direct(over.toggles),
        }
        // A character style reference is not inherited *into* a resolution — it
        // names the next level to visit, and the caller has already visited it.
        if over.style.is_some() {
            self.style = over.style;
        }
        self.fonts.layer(&over.fonts);
        self.size = over.size.or(self.size);
        self.size_complex = over.size_complex.or(self.size_complex);
        self.color = over.color.or(self.color);
        self.underline = over.underline.or(self.underline);
        self.highlight = over.highlight.or(self.highlight);
        self.vert_align = over.vert_align.or(self.vert_align);
        self.letter_spacing = over.letter_spacing.or(self.letter_spacing);
        self.scale = over.scale.or(self.scale);
        self.raise = over.raise.or(self.raise);
        self.kern = over.kern.or(self.kern);
        self.shading = over.shading.or(self.shading);
        self.border = over.border.or(self.border);
        self.rtl = over.rtl.or(self.rtl);
        self.no_proof = over.no_proof.or(self.no_proof);
        if over.lang.is_some() {
            self.lang.clone_from(&over.lang);
        }
    }

    pub fn bold(&self) -> bool {
        self.toggles.is_on(Toggle::Bold)
    }

    pub fn italic(&self) -> bool {
        self.toggles.is_on(Toggle::Italic)
    }

    /// Hidden text. Drawn only when the document is showing formatting marks,
    /// and — being a toggle — hideable by an odd number of levels only.
    pub fn hidden(&self) -> bool {
        self.toggles.is_on(Toggle::Vanish)
    }
}

/// `<w:jc>` — where a line's slack goes.
///
/// Word 2007 renamed `left` and `right` to `start` and `end` so that the names
/// mean the same thing in a right-to-left paragraph. Both spellings occur, often
/// in the same document, and they are the same value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Justify {
    #[default]
    Start,
    Center,
    End,
    /// `both` — justified, with the last line left alone.
    Both,
    /// `distribute` — justified, *including* the last line. East Asian layout
    /// uses it; it is not a synonym for `both`.
    Distribute,
}

impl Justify {
    pub fn from_val(text: &str) -> Option<Justify> {
        Some(match text {
            "left" | "start" => Justify::Start,
            "center" => Justify::Center,
            "right" | "end" => Justify::End,
            "both" | "justify" => Justify::Both,
            "distribute" => Justify::Distribute,
            _ => return None,
        })
    }
}

/// `<w:ind>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Indent {
    /// `w:start` / `w:left`.
    pub start: Option<Twips>,
    /// `w:end` / `w:right`.
    pub end: Option<Twips>,
    /// `w:firstLine` — the first line starts further in.
    pub first_line: Option<Twips>,
    /// `w:hanging` — the first line starts further *out*. Stored positive, as
    /// the file writes it.
    pub hanging: Option<Twips>,
}

impl Indent {
    /// How far the first line is offset from the paragraph's left edge, signed.
    ///
    /// The file may carry both attributes and `w:hanging` wins — that is Word's
    /// rule, not a tie-break we invented, and a reader that lets `firstLine` win
    /// turns every bulleted list inside out.
    pub fn first_line_offset(&self) -> Twips {
        match (self.hanging, self.first_line) {
            (Some(h), _) => Twips(-h.0),
            (None, Some(f)) => f,
            (None, None) => Twips(0),
        }
    }

    fn layer(&mut self, over: &Indent) {
        self.start = over.start.or(self.start);
        self.end = over.end.or(self.end);
        // The pair is stated together: a level that gives a first line indent
        // clears an inherited hanging one, or the two would combine into a
        // measurement neither level asked for.
        if over.first_line.is_some() || over.hanging.is_some() {
            self.first_line = over.first_line;
            self.hanging = over.hanging;
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Indent::default()
    }
}

/// `<w:spacing w:line=… w:lineRule=…>` — one attribute, three meanings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineSpacing {
    /// `auto` — a multiple of the line's natural height, in 240ths.
    Multiple(Line240),
    /// `atLeast` — twips, and the line grows past it for a tall glyph.
    AtLeast(Twips),
    /// `exact` — twips, and a tall glyph is clipped rather than growing the
    /// line. This is what makes a document with a pasted large font lose the
    /// tops of its letters.
    Exact(Twips),
}

impl Default for LineSpacing {
    fn default() -> Self {
        LineSpacing::Multiple(Line240::SINGLE)
    }
}

/// `<w:spacing>` on a paragraph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Spacing {
    pub before: Option<Twips>,
    pub after: Option<Twips>,
    /// `w:beforeAutospacing` — HTML-ish automatic spacing, which overrides the
    /// number beside it rather than adding to it.
    pub before_auto: Option<bool>,
    pub after_auto: Option<bool>,
    pub line: Option<LineSpacing>,
}

impl Spacing {
    fn layer(&mut self, over: &Spacing) {
        self.before = over.before.or(self.before);
        self.after = over.after.or(self.after);
        self.before_auto = over.before_auto.or(self.before_auto);
        self.after_auto = over.after_auto.or(self.after_auto);
        self.line = over.line.or(self.line);
    }

    pub fn is_empty(&self) -> bool {
        *self == Spacing::default()
    }
}

/// One tab stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabStop {
    pub position: Twips,
    pub kind: TabKind,
    pub leader: TabLeader,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabKind {
    #[default]
    Start,
    Center,
    End,
    /// Aligns on the decimal separator — which is the *locale's*, not a period.
    Decimal,
    /// Draws a vertical line and does not advance the text at all.
    Bar,
    /// Removes an inherited stop at this position. Not a stop: a deletion, and a
    /// reader that treats it as one leaves a tab where the paragraph style
    /// removed it.
    Clear,
}

impl TabKind {
    pub fn from_val(text: &str) -> Option<TabKind> {
        Some(match text {
            "left" | "start" => TabKind::Start,
            "center" => TabKind::Center,
            "right" | "end" => TabKind::End,
            "decimal" => TabKind::Decimal,
            "bar" => TabKind::Bar,
            "clear" => TabKind::Clear,
            // `num` is a legacy list-tab and behaves as a left stop.
            "num" => TabKind::Start,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabLeader {
    #[default]
    None,
    Dot,
    Hyphen,
    Underscore,
    Heavy,
    MiddleDot,
}

impl TabLeader {
    pub fn from_val(text: &str) -> Option<TabLeader> {
        Some(match text {
            "none" => TabLeader::None,
            "dot" => TabLeader::Dot,
            "hyphen" => TabLeader::Hyphen,
            "underscore" => TabLeader::Underscore,
            "heavy" => TabLeader::Heavy,
            "middleDot" => TabLeader::MiddleDot,
            _ => return None,
        })
    }

    /// The character a table of contents fills its line with.
    pub const fn glyph(self) -> Option<char> {
        Some(match self {
            TabLeader::None => return None,
            TabLeader::Dot => '.',
            TabLeader::Hyphen => '-',
            TabLeader::Underscore | TabLeader::Heavy => '_',
            TabLeader::MiddleDot => '·',
        })
    }
}

/// A border on any of the four edges of a paragraph, a table, a cell or a run.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Border {
    pub style: BorderStyle,
    /// In eighths of a point — `w:sz`, and not the `w:sz` that is a font size.
    pub size: Option<crate::units::Eighth>,
    /// `w:space` — the gap between the border and the text, in **points**, and
    /// a whole number. The one measurement in the format that is plain points.
    pub space: Option<u8>,
    pub color: Option<Color>,
    pub shadow: bool,
}

/// The border line styles that are drawn as lines.
///
/// ECMA lists about 180 more, nearly all of them clip-art borders from Word 97
/// (`w:val="apples"` draws a row of apples). They are read as
/// [`BorderStyle::Art`] with the name kept, so the writer puts back exactly what
/// it found and the layout draws a plain line in the space they occupy.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BorderStyle {
    /// `none` and `nil` are both "no border", and both occur.
    #[default]
    None,
    Single,
    Thick,
    Double,
    Dotted,
    Dashed,
    DotDash,
    DotDotDash,
    Triple,
    Wave,
    DoubleWave,
    /// One of the clip-art borders, or anything else unrecognised. Drawn as a
    /// single line of the stated width.
    Art,
}

impl BorderStyle {
    pub fn from_val(text: &str) -> BorderStyle {
        match text {
            "none" | "nil" => BorderStyle::None,
            "single" => BorderStyle::Single,
            "thick" => BorderStyle::Thick,
            "double" => BorderStyle::Double,
            "dotted" => BorderStyle::Dotted,
            "dashed" | "dashSmallGap" => BorderStyle::Dashed,
            "dotDash" => BorderStyle::DotDash,
            "dotDotDash" => BorderStyle::DotDotDash,
            "triple" => BorderStyle::Triple,
            "wave" => BorderStyle::Wave,
            "doubleWave" => BorderStyle::DoubleWave,
            _ => BorderStyle::Art,
        }
    }

    pub const fn draws(self) -> bool {
        !matches!(self, BorderStyle::None)
    }
}

/// A paragraph's own borders. `between` is drawn only where two consecutive
/// paragraphs share the same border settings, which is what makes a boxed
/// multi-paragraph quotation look like one box.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ParaBorders {
    pub top: Option<Border>,
    pub start: Option<Border>,
    pub bottom: Option<Border>,
    pub end: Option<Border>,
    pub between: Option<Border>,
    pub bar: Option<Border>,
}

impl ParaBorders {
    pub fn is_empty(&self) -> bool {
        *self == ParaBorders::default()
    }
}

/// `<w:shd>` — a fill, a pattern, and the pattern's own colour.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Shading {
    pub pattern: ShadingPattern,
    /// The background. `auto` here means the page, not black.
    pub fill: Option<Color>,
    /// The foreground the *pattern* is drawn in. Irrelevant for `clear` and
    /// `solid`, which is why nearly every real document leaves it `auto`.
    pub color: Option<Color>,
}

impl Shading {
    /// The colour actually painted behind the text.
    ///
    /// `solid` inverts the two: its `w:color` is the fill and its `w:fill` is
    /// ignored. This is the same trap as a spreadsheet's differential fill using
    /// `bgColor` where a solid one uses `fgColor`, and it costs a whole
    /// document's shading when it is missed.
    pub fn background(&self) -> Option<Color> {
        match self.pattern {
            ShadingPattern::Clear => self.fill,
            ShadingPattern::Solid => self.color.or(self.fill),
            // A percentage pattern blends the two; the fill is the closer
            // approximation and the pattern is not drawn.
            _ => self.fill,
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Shading::default()
    }
}

/// The shading patterns, folded to the three that change what is painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShadingPattern {
    /// No pattern — the fill shows through. The overwhelmingly common value.
    #[default]
    Clear,
    /// The *foreground* colour fills the area. See [`Shading::background`].
    Solid,
    /// `pct5` … `pct95`, held as the percentage so a renderer can blend rather
    /// than guess.
    Percent(u8),
    /// One of the twenty-odd hatchings — `horzStripe`, `diagCross`, `thinZigZag`
    /// and the rest. Kept as a kind so the writer restores the name, and drawn
    /// as its fill.
    Hatch,
}

impl ShadingPattern {
    pub fn from_val(text: &str) -> ShadingPattern {
        match text {
            "clear" | "nil" => ShadingPattern::Clear,
            "solid" => ShadingPattern::Solid,
            other => match other.strip_prefix("pct").and_then(|n| n.parse::<u8>().ok()) {
                Some(percent) => ShadingPattern::Percent(percent.min(100)),
                None => ShadingPattern::Hatch,
            },
        }
    }
}

/// Vertical alignment of the glyphs on a line — `<w:textAlignment>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Auto,
    Top,
    Center,
    Baseline,
    Bottom,
}

impl TextAlign {
    pub fn from_val(text: &str) -> Option<TextAlign> {
        Some(match text {
            "auto" => TextAlign::Auto,
            "top" => TextAlign::Top,
            "center" => TextAlign::Center,
            "baseline" => TextAlign::Baseline,
            "bottom" => TextAlign::Bottom,
            _ => return None,
        })
    }
}

/// `<w:numPr>` — which list a paragraph is in, and how deep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumRef {
    /// `w:numId`. **Zero means "not in a list"** and is how a style's numbering
    /// is cancelled for one paragraph — it is not an id.
    pub num_id: u32,
    /// `w:ilvl`, zero through eight.
    pub level: u8,
}

impl NumRef {
    pub const fn is_numbered(self) -> bool {
        self.num_id != 0
    }
}

/// One paragraph's formatting, as one level states it.
///
/// Boxed where a field is both large and rare: a paragraph carrying tabs,
/// borders or a section break is unusual, and a document has a great many
/// paragraphs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParaProps {
    /// `<w:pStyle>`.
    pub style: Option<StyleId>,
    pub numbering: Option<NumRef>,
    pub justify: Option<Justify>,
    pub indent: Indent,
    pub spacing: Spacing,
    pub keep_next: Option<bool>,
    pub keep_lines: Option<bool>,
    pub page_break_before: Option<bool>,
    pub widow_control: Option<bool>,
    /// `<w:contextualSpacing>` — suppress the space between paragraphs of the
    /// same style, which is what makes a list look like a list.
    pub contextual_spacing: Option<bool>,
    pub suppress_line_numbers: Option<bool>,
    /// Right-to-left paragraph direction.
    pub bidi: Option<bool>,
    /// `<w:outlineLvl>` — 0..=8, or absent for body text. What a table of
    /// contents and the navigation pane are built from.
    pub outline_level: Option<u8>,
    pub tabs: Option<Vec<TabStop>>,
    pub borders: Option<Box<ParaBorders>>,
    pub shading: Option<Shading>,
    pub text_align: Option<TextAlign>,
    /// `<w:rPr>` inside `<w:pPr>` — the formatting of the **paragraph mark
    /// itself**, which is a real character with a real size, and which is what
    /// the next paragraph inherits when the user presses Enter at the end.
    pub mark: Option<Box<RunProps>>,
}

impl ParaProps {
    pub fn new() -> ParaProps {
        ParaProps::default()
    }

    pub fn is_empty(&self) -> bool {
        *self == ParaProps::default()
    }

    /// Lays `over` on top of these.
    ///
    /// There is no toggle rule here: ECMA's toggle list is entirely run
    /// properties, so `keepNext` on a style over `keepNext` on its parent is
    /// simply on. The `layer` argument is taken anyway, because the paragraph
    /// mark's run properties inside carry the rule with them.
    pub fn layer(&mut self, over: &ParaProps, layer: Layer) {
        if over.style.is_some() {
            self.style = over.style;
        }
        self.numbering = over.numbering.or(self.numbering);
        self.justify = over.justify.or(self.justify);
        self.indent.layer(&over.indent);
        self.spacing.layer(&over.spacing);
        self.keep_next = over.keep_next.or(self.keep_next);
        self.keep_lines = over.keep_lines.or(self.keep_lines);
        self.page_break_before = over.page_break_before.or(self.page_break_before);
        self.widow_control = over.widow_control.or(self.widow_control);
        self.contextual_spacing = over.contextual_spacing.or(self.contextual_spacing);
        self.suppress_line_numbers = over.suppress_line_numbers.or(self.suppress_line_numbers);
        self.bidi = over.bidi.or(self.bidi);
        self.outline_level = over.outline_level.or(self.outline_level);
        if let Some(tabs) = &over.tabs {
            self.merge_tabs(tabs);
        }
        if over.borders.is_some() {
            self.borders.clone_from(&over.borders);
        }
        self.shading = over.shading.or(self.shading);
        self.text_align = over.text_align.or(self.text_align);
        match (&mut self.mark, &over.mark) {
            (_, None) => {}
            (None, Some(over)) => self.mark = Some(over.clone()),
            (Some(mine), Some(over)) => mine.layer(over, layer),
        }
    }

    /// Tab stops accumulate across levels rather than replacing each other, and
    /// a `clear` stop deletes an inherited one at the same position.
    ///
    /// A paragraph that adds one stop to its style's three has four, not one.
    /// Replacing the list — the obvious reading — silently drops every stop a
    /// style set, which is most visible in a table of contents, where the right
    /// aligned page number lands in the middle of the line.
    fn merge_tabs(&mut self, over: &[TabStop]) {
        let mut stops = self.tabs.take().unwrap_or_default();
        for stop in over {
            stops.retain(|existing| existing.position != stop.position);
            if stop.kind != TabKind::Clear {
                stops.push(*stop);
            }
        }
        stops.sort_by_key(|stop| stop.position);
        self.tabs = (!stops.is_empty()).then_some(stops);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_element_is_true_and_the_three_spellings_of_false_are_false() {
        assert!(on_off(None));
        assert!(on_off(Some("1")));
        assert!(on_off(Some("true")));
        assert!(on_off(Some("on")));
        assert!(!on_off(Some("0")));
        assert!(!on_off(Some("false")));
        assert!(!on_off(Some("off")));
    }

    #[test]
    fn bold_over_bold_is_not_bold() {
        // Word's Strong character style applied to a Heading — both bold —
        // leaves the text unbolded. It is the format's most surprising rule and
        // the reason toggles are not ordinary properties.
        let mut heading = Toggles::new();
        heading.set(Toggle::Bold, true);
        let mut strong = Toggles::new();
        strong.set(Toggle::Bold, true);

        let mut resolved = heading;
        resolved.layer_style(strong);
        assert_eq!(resolved.get(Toggle::Bold), Some(false));

        // A third bold level turns it back on. Parity, not precedence.
        resolved.layer_style(strong);
        assert_eq!(resolved.get(Toggle::Bold), Some(true));
    }

    #[test]
    fn direct_formatting_is_absolute_where_a_style_only_toggles() {
        let mut heading = Toggles::new();
        heading.set(Toggle::Bold, true);

        let mut off = Toggles::new();
        off.set(Toggle::Bold, false);

        // Through a style, `false` is the identity — XOR cannot turn anything
        // off, which is why a style is unable to un-bold a bold style.
        let mut by_style = heading;
        by_style.layer_style(off);
        assert_eq!(by_style.get(Toggle::Bold), Some(true));

        // Ctrl+B does it, because direct formatting overrides outright.
        let mut by_hand = heading;
        by_hand.layer_direct(off);
        assert_eq!(by_hand.get(Toggle::Bold), Some(false));
    }

    #[test]
    fn a_toggle_nobody_stated_stays_unstated() {
        // "Absent" has to survive layering, or the resolver cannot tell a style
        // that says nothing about italics from one that says no.
        let mut base = Toggles::new();
        base.set(Toggle::Bold, true);
        let mut over = Toggles::new();
        over.set(Toggle::Italic, true);
        base.layer_style(over);
        assert_eq!(base.get(Toggle::Bold), Some(true));
        assert_eq!(base.get(Toggle::Italic), Some(true));
        assert_eq!(base.get(Toggle::Caps), None);
        assert_eq!(base.get(Toggle::Vanish), None);
    }

    #[test]
    fn every_toggle_has_its_own_bit_and_its_own_element_name() {
        let mut seen = 0u16;
        for toggle in Toggle::ALL {
            let bit = toggle as u16;
            assert_eq!(seen & bit, 0, "{} reuses a bit", toggle.element());
            seen |= bit;
            assert_eq!(Toggle::from_element(toggle.element()), Some(toggle));
        }
        assert_eq!(Toggle::from_element("rtl"), None, "rtl is not a toggle");
        assert_eq!(Toggle::from_element("noProof"), None);
    }

    #[test]
    fn a_hanging_indent_beats_a_first_line_one() {
        let both = Indent {
            first_line: Some(Twips(720)),
            hanging: Some(Twips(360)),
            ..Indent::default()
        };
        assert_eq!(both.first_line_offset(), Twips(-360));

        let first = Indent {
            first_line: Some(Twips(720)),
            ..Indent::default()
        };
        assert_eq!(first.first_line_offset(), Twips(720));
        assert_eq!(Indent::default().first_line_offset(), Twips(0));
    }

    #[test]
    fn stating_a_first_line_indent_clears_an_inherited_hanging_one() {
        let mut base = Indent {
            start: Some(Twips(720)),
            hanging: Some(Twips(360)),
            ..Indent::default()
        };
        base.layer(&Indent {
            first_line: Some(Twips(240)),
            ..Indent::default()
        });
        assert_eq!(base.start, Some(Twips(720)), "the left edge is untouched");
        assert_eq!(base.hanging, None);
        assert_eq!(base.first_line_offset(), Twips(240));
    }

    #[test]
    fn tab_stops_accumulate_and_a_clear_stop_deletes_one() {
        let stop = |position: i32, kind: TabKind| TabStop {
            position: Twips(position),
            kind,
            leader: TabLeader::None,
        };
        let mut style = ParaProps {
            tabs: Some(vec![stop(1440, TabKind::Start), stop(4320, TabKind::End)]),
            ..ParaProps::default()
        };
        style.layer(
            &ParaProps {
                tabs: Some(vec![
                    stop(2880, TabKind::Center),
                    stop(4320, TabKind::Clear),
                ]),
                ..ParaProps::default()
            },
            Layer::Direct,
        );
        let tabs = style.tabs.unwrap();
        assert_eq!(tabs.len(), 2, "one added, one cleared");
        assert_eq!(tabs[0].position, Twips(1440));
        assert_eq!(tabs[1].position, Twips(2880));
        assert_eq!(tabs[1].kind, TabKind::Center);
    }

    #[test]
    fn solid_shading_paints_its_foreground_and_clear_paints_its_fill() {
        let clear = Shading {
            pattern: ShadingPattern::Clear,
            fill: Some(Color::Rgb([0xD9, 0xD9, 0xD9])),
            color: Some(Color::Auto),
        };
        assert_eq!(clear.background(), Some(Color::Rgb([0xD9, 0xD9, 0xD9])));

        let solid = Shading {
            pattern: ShadingPattern::Solid,
            fill: Some(Color::Auto),
            color: Some(Color::Rgb([0x00, 0x00, 0x00])),
        };
        assert_eq!(solid.background(), Some(Color::BLACK));
    }

    #[test]
    fn shading_percentages_are_read_as_percentages() {
        assert_eq!(
            ShadingPattern::from_val("pct15"),
            ShadingPattern::Percent(15)
        );
        assert_eq!(ShadingPattern::from_val("clear"), ShadingPattern::Clear);
        assert_eq!(
            ShadingPattern::from_val("diagStripe"),
            ShadingPattern::Hatch
        );
    }

    #[test]
    fn left_and_start_are_one_value() {
        assert_eq!(Justify::from_val("left"), Some(Justify::Start));
        assert_eq!(Justify::from_val("start"), Some(Justify::Start));
        assert_eq!(Justify::from_val("right"), Some(Justify::End));
        assert_eq!(Justify::from_val("end"), Some(Justify::End));
        // `distribute` justifies the last line too, so it is not `both`.
        assert_ne!(Justify::from_val("distribute"), Justify::from_val("both"));
    }

    #[test]
    fn a_num_id_of_zero_takes_a_paragraph_out_of_its_lists() {
        assert!(!NumRef {
            num_id: 0,
            level: 0
        }
        .is_numbered());
        assert!(NumRef {
            num_id: 1,
            level: 0
        }
        .is_numbered());
    }

    #[test]
    fn run_properties_layer_nearest_wins_apart_from_the_toggles() {
        let mut base = RunProps {
            size: Some(HalfPoint(20)),
            color: Some(Color::BLACK),
            ..RunProps::default()
        };
        base.toggles.set(Toggle::Bold, true);
        base.fonts.ascii = Some("Calibri".into());

        let mut over = RunProps {
            size: Some(HalfPoint(28)),
            ..RunProps::default()
        };
        over.toggles.set(Toggle::Bold, true);
        over.fonts.east_asian = Some("MS Mincho".into());

        base.layer(&over, Layer::Style);
        assert_eq!(base.size, Some(HalfPoint(28)));
        assert_eq!(base.color, Some(Color::BLACK), "unstated is untouched");
        assert_eq!(base.fonts.ascii.as_deref(), Some("Calibri"));
        assert_eq!(base.fonts.east_asian.as_deref(), Some("MS Mincho"));
        assert!(!base.bold(), "two bold levels cancel");
    }
}
