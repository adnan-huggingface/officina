//! The five units a Word document measures in.
//!
//! A `.docx` does not have a length type. It has five, and they are not
//! distinguished by anything in the file — `w:sz` is a font size in *half-points*
//! on a `<w:rPr>` and a border width in *eighths of a point* on a `<w:top>`, the
//! same attribute name with a factor of four between them. `w:w` is twips on a
//! `<w:tcW w:type="dxa">` and fiftieths of a percent on a `<w:tcW w:type="pct">`.
//!
//! Calx learned this the expensive way with `f32` screen positions against `f64`
//! grid positions (see `LEARNINGS.md` §4). So every length here is a distinct
//! type that cannot be added to another, conversion happens through named
//! methods, and the layout engine works in points — one conversion, at one
//! boundary.
//!
//! All of them are signed. Word allows a negative indent (that is what a hanging
//! indent into the margin is), a negative spacing, and a negative position, and
//! an unsigned type would turn each of those into a very large positive one.

use std::fmt;

/// Points per inch. The one constant everything else is defined against.
pub const POINTS_PER_INCH: f64 = 72.0;

/// A twentieth of a point, and the unit most of a document is written in:
/// page size, margins, indents, paragraph spacing, tab stops, table widths.
///
/// Spelled `dxa` when a `w:type` attribute has to name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Twips(pub i32);

/// Half a point, and the unit a *font size* is written in — `<w:sz w:val="22"/>`
/// is 11pt, which is why every default in a Word file looks doubled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct HalfPoint(pub i32);

/// An eighth of a point, and the unit a *border width* is written in. Also
/// spelled `w:sz`, on an element that is nowhere near a `<w:rPr>`.
///
/// Word's UI offers ½pt through 6pt, which is 4 through 48 here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Eighth(pub i32);

/// English Metric Unit: 914,400 per inch, 12,700 per point. DrawingML's unit,
/// so it is what an inline picture's extent is written in and nothing else is.
///
/// `i64` rather than `i32` because that is what the format's own type is, and
/// because an `i32` runs out at about 2,300 inches — reachable by a drawing
/// canvas, and silently wrong rather than clamped when it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Emu(pub i64);

/// A fiftieth of a percent: 5000 is 100%. What a `w:type="pct"` width means, and
/// what `<w:tblW>` most often carries.
///
/// The name is deliberate. `Percent(50)` would read as fifty percent and mean
/// one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Pct50(pub i32);

/// A 240th of a line, which is how `<w:spacing w:line="…" w:lineRule="auto">`
/// spells multiple line spacing: 240 is single, 360 is one-and-a-half, 480 is
/// double.
///
/// Under the other two line rules — `exact` and `atLeast` — the *same attribute*
/// is twips instead. [`crate::prop::LineSpacing`] keeps the rule beside the
/// number so the two cannot be confused at the point of use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Line240(pub i32);

impl Twips {
    pub const PER_POINT: i32 = 20;
    pub const PER_INCH: i32 = 1440;

    /// The page size Word starts a US Letter document at: 12240 x 15840.
    pub const LETTER_WIDTH: Twips = Twips(12240);
    pub const LETTER_HEIGHT: Twips = Twips(15840);
    /// A4, which is what most of the world outside the US opens with.
    pub const A4_WIDTH: Twips = Twips(11906);
    pub const A4_HEIGHT: Twips = Twips(16838);
    /// Word's default margin on all four sides: one inch.
    pub const INCH: Twips = Twips(1440);

    pub fn from_points(points: f64) -> Twips {
        Twips((points * Self::PER_POINT as f64).round() as i32)
    }

    pub fn from_inches(inches: f64) -> Twips {
        Twips((inches * Self::PER_INCH as f64).round() as i32)
    }

    pub fn points(self) -> f64 {
        self.0 as f64 / Self::PER_POINT as f64
    }

    pub fn inches(self) -> f64 {
        self.0 as f64 / Self::PER_INCH as f64
    }
}

impl HalfPoint {
    /// Word's default body size, in the file's own units: 22 half-points.
    pub const DEFAULT: HalfPoint = HalfPoint(22);

    /// Word's own floor and ceiling in the font size box. A file may hold
    /// something outside them; [`HalfPoint::clamped`] is for values we author.
    pub const MIN: HalfPoint = HalfPoint(2);
    pub const MAX: HalfPoint = HalfPoint(3960);

    pub fn from_points(points: f64) -> HalfPoint {
        HalfPoint((points * 2.0).round() as i32)
    }

    pub fn points(self) -> f64 {
        self.0 as f64 / 2.0
    }

    /// Held inside the range Word's own UI can produce.
    pub fn clamped(self) -> HalfPoint {
        HalfPoint(self.0.clamp(Self::MIN.0, Self::MAX.0))
    }
}

impl Eighth {
    /// The width Word writes for a plain single-line table border: half a point.
    pub const HAIRLINE: Eighth = Eighth(4);

    pub fn from_points(points: f64) -> Eighth {
        Eighth((points * 8.0).round() as i32)
    }

    pub fn points(self) -> f64 {
        self.0 as f64 / 8.0
    }
}

impl Emu {
    pub const PER_POINT: i64 = 12_700;
    pub const PER_INCH: i64 = 914_400;
    /// EMUs per centimetre. The number is exact, which is the whole point of the
    /// unit: it divides evenly by both inches and centimetres.
    pub const PER_CM: i64 = 360_000;

    pub fn from_points(points: f64) -> Emu {
        Emu((points * Self::PER_POINT as f64).round() as i64)
    }

    pub fn from_twips(twips: Twips) -> Emu {
        Emu(twips.0 as i64 * (Self::PER_POINT / Twips::PER_POINT as i64))
    }

    pub fn points(self) -> f64 {
        self.0 as f64 / Self::PER_POINT as f64
    }

    pub fn inches(self) -> f64 {
        self.0 as f64 / Self::PER_INCH as f64
    }
}

impl Pct50 {
    pub const FULL: Pct50 = Pct50(5000);

    pub fn from_percent(percent: f64) -> Pct50 {
        Pct50((percent * 50.0).round() as i32)
    }

    pub fn percent(self) -> f64 {
        self.0 as f64 / 50.0
    }

    /// The share of `total` this represents. Used wherever a percentage width
    /// has to become a real one.
    pub fn of(self, total: Twips) -> Twips {
        Twips((total.0 as i64 * self.0 as i64 / Pct50::FULL.0 as i64) as i32)
    }
}

impl Line240 {
    pub const SINGLE: Line240 = Line240(240);
    pub const ONE_AND_A_HALF: Line240 = Line240(360);
    pub const DOUBLE: Line240 = Line240(480);

    pub fn from_multiple(multiple: f64) -> Line240 {
        Line240((multiple * 240.0).round() as i32)
    }

    pub fn multiple(self) -> f64 {
        self.0 as f64 / 240.0
    }
}

impl fmt::Display for Twips {
    /// Points, which is what a dialog box shows. Trailing zeros are dropped so
    /// 240 twips reads as `12pt` rather than `12.00pt`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let points = self.points();
        if points.fract() == 0.0 {
            write!(f, "{}pt", points as i64)
        } else {
            write!(f, "{points}pt")
        }
    }
}

impl fmt::Display for HalfPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let points = self.points();
        if points.fract() == 0.0 {
            write!(f, "{}", points as i64)
        } else {
            write!(f, "{points}")
        }
    }
}

/// Parses the text of a measurement attribute as a whole number.
///
/// Word writes these as plain integers, but a leading `+` and surrounding
/// whitespace both occur in files written by other producers, and a value
/// carrying a unit suffix (`0.5in`, `36pt`) is legal in a handful of places the
/// schema types as `ST_UniversalMeasure`. Returns `None` rather than a default
/// so a caller can tell "absent" from "present and unreadable" — the two mean
/// different things everywhere in this format.
pub fn parse_i32(text: &str) -> Option<i32> {
    let text = text.trim();
    let text = text.strip_prefix('+').unwrap_or(text);
    text.parse::<i32>().ok()
}

/// Parses `ST_UniversalMeasure` — a number with a unit suffix — into twips.
///
/// `mm`, `cm`, `in`, `pt`, `pc` and `pi` are the suffixes the schema allows. A
/// bare number is already twips, which is why this can be the only parser a
/// caller needs for the attributes that accept both.
pub fn parse_universal(text: &str) -> Option<Twips> {
    let text = text.trim();
    let split = text.len()
        - text
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_alphabetic())
            .count();
    let (number, suffix) = text.split_at(split);
    if suffix.is_empty() {
        return parse_i32(number).map(Twips);
    }
    let value: f64 = number.trim().parse().ok()?;
    let inches = match suffix {
        "mm" => value / 25.4,
        "cm" => value / 2.54,
        "in" => value,
        "pt" => value / POINTS_PER_INCH,
        // A pica is twelve points, and `pi` is the same unit under another name.
        "pc" | "pi" => value * 12.0 / POINTS_PER_INCH,
        _ => return None,
    };
    Some(Twips::from_inches(inches))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_font_size_is_half_points_and_a_border_width_is_eighths() {
        // Both are spelled `w:sz` in the file. Twelve of one is 6pt and twelve
        // of the other is 1.5pt, which is the whole reason they are two types.
        assert_eq!(HalfPoint(12).points(), 6.0);
        assert_eq!(Eighth(12).points(), 1.5);
    }

    #[test]
    fn words_own_defaults_convert_to_the_numbers_the_dialogs_show() {
        assert_eq!(HalfPoint::DEFAULT.points(), 11.0);
        assert_eq!(Twips::INCH.inches(), 1.0);
        assert_eq!(Twips::LETTER_WIDTH.inches(), 8.5);
        assert_eq!(Twips::LETTER_HEIGHT.inches(), 11.0);
        assert_eq!(Eighth::HAIRLINE.points(), 0.5);
        assert_eq!(Line240::SINGLE.multiple(), 1.0);
    }

    #[test]
    fn a4_is_within_a_twip_of_its_millimetre_definition() {
        // 210 x 297mm. The file's integers are what Word writes, and they are
        // rounded rather than exact — worth pinning so a "correction" to a
        // prettier number is caught.
        assert_eq!(Twips::A4_WIDTH, Twips(11906));
        assert!((Twips::A4_WIDTH.inches() - 210.0 / 25.4).abs() < 1.0 / 1440.0);
        assert!((Twips::A4_HEIGHT.inches() - 297.0 / 25.4).abs() < 1.0 / 1440.0);
    }

    #[test]
    fn a_negative_indent_survives_the_type() {
        // A hanging indent is a negative first-line indent, and it is ordinary
        // rather than exceptional. An unsigned length would make it enormous.
        let hanging = Twips(-720);
        assert_eq!(hanging.points(), -36.0);
        assert_eq!(Twips::from_points(-36.0), hanging);
    }

    #[test]
    fn emus_divide_evenly_by_both_inches_and_centimetres() {
        assert_eq!(Emu::PER_INCH * 100 / 254, Emu::PER_CM);
        assert_eq!(Emu::PER_INCH % Emu::PER_POINT, 0);
        assert_eq!(Emu::from_twips(Twips::INCH), Emu(Emu::PER_INCH));
        assert_eq!(Emu::from_points(72.0), Emu(Emu::PER_INCH));
    }

    #[test]
    fn a_fiftieth_of_a_percent_is_not_a_percent() {
        assert_eq!(Pct50::FULL.percent(), 100.0);
        assert_eq!(Pct50(2500).percent(), 50.0);
        assert_eq!(Pct50::from_percent(33.3), Pct50(1665));
        // Half of a six-inch text column.
        assert_eq!(Pct50(2500).of(Twips(8640)), Twips(4320));
    }

    #[test]
    fn a_measurement_may_carry_its_own_unit() {
        assert_eq!(parse_universal("1440"), Some(Twips::INCH));
        assert_eq!(parse_universal("1in"), Some(Twips::INCH));
        assert_eq!(parse_universal("72pt"), Some(Twips::INCH));
        assert_eq!(parse_universal("2.54cm"), Some(Twips::INCH));
        assert_eq!(parse_universal("25.4mm"), Some(Twips::INCH));
        assert_eq!(parse_universal("6pc"), Some(Twips::INCH));
        assert_eq!(parse_universal("nonsense"), None);
    }

    #[test]
    fn absent_and_unreadable_are_told_apart() {
        // Everywhere in this format a missing attribute means "inherit" and a
        // present one means "decide", so a parser that returns a default for
        // garbage has erased the difference.
        assert_eq!(parse_i32("0"), Some(0));
        assert_eq!(parse_i32(" +240 "), Some(240));
        assert_eq!(parse_i32(""), None);
        assert_eq!(parse_i32("auto"), None);
    }
}
