//! Colours, and the three ways a Word document writes one down.
//!
//! Fewer spellings than a spreadsheet uses, and different ones:
//!
//! - `w:val="4472C4"` — six hex digits, **no alpha byte**. Excel writes eight.
//! - `w:val="auto"` — "whatever the page is", which is not a colour and must
//!   stay unresolved so a dark UI can choose its own foreground.
//! - `w:themeColor="accent1" w:themeTint="99"` — into the document theme,
//!   lightened or darkened.
//!
//! The theme slots are **named** here rather than indexed as they are in a
//! workbook, which removes Excel's off-by-one trap and installs a different one:
//! `text1` and `background1` are the same two scheme entries as `dark1` and
//! `light1`, under a second set of names that Word also accepts, and `dark1` is
//! `<a:clrScheme>`'s first child while `background1` is not.
//!
//! Highlighting is a fourth thing and deliberately not a `Color`: `w:highlight`
//! takes one of seventeen names and nothing else, because it is the marker-pen
//! palette rather than a colour picker.

/// A colour as a `.docx` spells it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Color {
    /// The page's own foreground. Not resolved to black on purpose.
    #[default]
    Auto,
    /// `RRGGBB`. No alpha: WordprocessingML has no transparency here.
    Rgb([u8; 3]),
    /// A theme slot, with at most one of tint (toward white) and shade (toward
    /// black). Word never writes both, and the schema allows it, so the model
    /// keeps whichever arrived.
    Theme {
        slot: ThemeSlot,
        tint: Option<u8>,
        shade: Option<u8>,
    },
}

/// The named entries of the document's colour scheme.
///
/// Word accepts two names for each of the first four — `dark1`/`text1`,
/// `light1`/`background1`, and so on — and writes the `text`/`background` pair.
/// They are one slot, not two, which is why parsing folds them together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThemeSlot {
    /// `dark1` / `text1` — `<a:clrScheme>`'s **first** child.
    Dark1,
    /// `light1` / `background1` — the second.
    Light1,
    Dark2,
    Light2,
    Accent1,
    Accent2,
    Accent3,
    Accent4,
    Accent5,
    Accent6,
    Hyperlink,
    FollowedHyperlink,
}

impl ThemeSlot {
    /// Position in `<a:clrScheme>`, which writes dk1, lt1, dk2, lt2, accent1..6,
    /// hlink, folHlink in that order.
    pub const fn scheme_index(self) -> usize {
        match self {
            ThemeSlot::Dark1 => 0,
            ThemeSlot::Light1 => 1,
            ThemeSlot::Dark2 => 2,
            ThemeSlot::Light2 => 3,
            ThemeSlot::Accent1 => 4,
            ThemeSlot::Accent2 => 5,
            ThemeSlot::Accent3 => 6,
            ThemeSlot::Accent4 => 7,
            ThemeSlot::Accent5 => 8,
            ThemeSlot::Accent6 => 9,
            ThemeSlot::Hyperlink => 10,
            ThemeSlot::FollowedHyperlink => 11,
        }
    }

    pub fn from_name(name: &str) -> Option<ThemeSlot> {
        Some(match name {
            "dark1" | "text1" => ThemeSlot::Dark1,
            "light1" | "background1" => ThemeSlot::Light1,
            "dark2" | "text2" => ThemeSlot::Dark2,
            "light2" | "background2" => ThemeSlot::Light2,
            "accent1" => ThemeSlot::Accent1,
            "accent2" => ThemeSlot::Accent2,
            "accent3" => ThemeSlot::Accent3,
            "accent4" => ThemeSlot::Accent4,
            "accent5" => ThemeSlot::Accent5,
            "accent6" => ThemeSlot::Accent6,
            "hyperlink" => ThemeSlot::Hyperlink,
            "followedHyperlink" => ThemeSlot::FollowedHyperlink,
            // `none` is a real value of the attribute and means the theme colour
            // was cleared, which is not a slot.
            _ => return None,
        })
    }

    /// The name Word itself writes, for a writer that has to author one.
    pub const fn name(self) -> &'static str {
        match self {
            ThemeSlot::Dark1 => "text1",
            ThemeSlot::Light1 => "background1",
            ThemeSlot::Dark2 => "text2",
            ThemeSlot::Light2 => "background2",
            ThemeSlot::Accent1 => "accent1",
            ThemeSlot::Accent2 => "accent2",
            ThemeSlot::Accent3 => "accent3",
            ThemeSlot::Accent4 => "accent4",
            ThemeSlot::Accent5 => "accent5",
            ThemeSlot::Accent6 => "accent6",
            ThemeSlot::Hyperlink => "hyperlink",
            ThemeSlot::FollowedHyperlink => "followedHyperlink",
        }
    }
}

impl Color {
    pub const BLACK: Color = Color::Rgb([0x00, 0x00, 0x00]);
    pub const WHITE: Color = Color::Rgb([0xFF, 0xFF, 0xFF]);

    /// Parses a `w:val`, which is six hex digits or the word `auto`.
    ///
    /// Eight digits are accepted and the leading pair discarded: a producer that
    /// has copied a colour out of a workbook writes `FF4472C4`, and refusing it
    /// would lose a colour that is perfectly well specified.
    pub fn from_val(text: &str) -> Option<Color> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("auto") {
            return Some(Color::Auto);
        }
        let hex = text.strip_prefix('#').unwrap_or(text);
        let bytes: Vec<u8> = (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16))
            .collect::<Result<_, _>>()
            .ok()?;
        match bytes.len() {
            3 => Some(Color::Rgb([bytes[0], bytes[1], bytes[2]])),
            4 => Some(Color::Rgb([bytes[1], bytes[2], bytes[3]])),
            _ => None,
        }
    }

    /// `RRGGBB`, or `auto`. What a writer puts in `w:val`.
    pub fn to_val(self) -> String {
        match self {
            Color::Auto => "auto".to_string(),
            Color::Rgb([r, g, b]) => format!("{r:02X}{g:02X}{b:02X}"),
            // A themed colour's `w:val` is the *cached* resolution, which a
            // writer supplies from the theme. Nothing sensible to say here.
            Color::Theme { .. } => "auto".to_string(),
        }
    }

    pub fn is_auto(self) -> bool {
        matches!(self, Color::Auto)
    }

    /// Resolves to RGB, or `None` when there is no answer without knowing what
    /// the colour is being drawn on.
    ///
    /// `Auto` is the honest `None`, and so is a theme slot the document does not
    /// define. Inventing a colour for either is worse than letting the caller
    /// use its own foreground, which is what Word does.
    pub fn resolve(self, theme: &Theme) -> Option<[u8; 3]> {
        match self {
            Color::Auto => None,
            Color::Rgb(rgb) => Some(rgb),
            Color::Theme { slot, tint, shade } => {
                let base = theme.color(slot)?;
                Some(match (tint, shade) {
                    (Some(t), _) => tint_toward_white(base, t),
                    (None, Some(s)) => shade_toward_black(base, s),
                    (None, None) => base,
                })
            }
        }
    }
}

/// `themeTint="99"` — a hex byte, and the *proportion of the original colour
/// that survives*. `FF` is no change and `00` is white.
///
/// Word blends toward white in sRGB rather than lightening in HSL, which is what
/// a spreadsheet's `tint` does. The two produce visibly different colours from
/// the same scheme, and this is the one place where copying Calx's colour code
/// would have been wrong. Stated as a limit rather than hidden: this is checked
/// against the numbers Word caches in `w:val` beside the theme attributes, not
/// against Word's screen, until there is a document surface to compare on.
fn tint_toward_white(base: [u8; 3], tint: u8) -> [u8; 3] {
    let k = tint as f64 / 255.0;
    base.map(|c| (c as f64 * k + 255.0 * (1.0 - k)).round().clamp(0.0, 255.0) as u8)
}

/// `themeShade="BF"` — the same byte, blending toward black instead.
fn shade_toward_black(base: [u8; 3], shade: u8) -> [u8; 3] {
    let k = shade as f64 / 255.0;
    base.map(|c| (c as f64 * k).round().clamp(0.0, 255.0) as u8)
}

/// One of the theme's two font schemes: the faces for each script family.
///
/// `<a:majorFont>` is what headings use and `<a:minorFont>` is body text, which
/// is why `<w:rFonts w:asciiTheme="minorHAnsi"/>` is on very nearly every run in
/// a modern document and a resolver that ignores it draws the whole thing in a
/// fallback face.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontFaces {
    pub latin: Option<std::sync::Arc<str>>,
    pub east_asian: Option<std::sync::Arc<str>>,
    pub complex: Option<std::sync::Arc<str>>,
}

/// The document's colour and font schemes, out of `theme1.xml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// In `<a:clrScheme>` order — dk1 first — because that is the order the
    /// part writes and [`ThemeSlot::scheme_index`] is defined against it. No
    /// swap, unlike the spreadsheet side, because Word names its slots.
    colors: Vec<[u8; 3]>,
    pub major: FontFaces,
    pub minor: FontFaces,
}

impl Default for Theme {
    /// Office's own default scheme, so a document whose theme part is missing
    /// or unreadable still renders in the colours its author saw.
    fn default() -> Self {
        Theme {
            colors: vec![
                [0x00, 0x00, 0x00], // dk1
                [0xFF, 0xFF, 0xFF], // lt1
                [0x44, 0x54, 0x6A], // dk2
                [0xE7, 0xE6, 0xE6], // lt2
                [0x44, 0x72, 0xC4], // accent1
                [0xED, 0x7D, 0x31], // accent2
                [0xA5, 0xA5, 0xA5], // accent3
                [0xFF, 0xC0, 0x00], // accent4
                [0x5B, 0x9B, 0xD5], // accent5
                [0x70, 0xAD, 0x47], // accent6
                [0x05, 0x63, 0xC1], // hlink
                [0x95, 0x4F, 0x72], // folHlink
            ],
            // Office's own scheme since 2013. A document whose theme part is
            // missing still renders in the faces its author saw.
            major: FontFaces {
                latin: Some("Calibri Light".into()),
                ..FontFaces::default()
            },
            minor: FontFaces {
                latin: Some("Calibri".into()),
                ..FontFaces::default()
            },
        }
    }
}

impl Theme {
    /// Builds from the scheme colours in the order the part writes them.
    pub fn from_scheme(scheme: &[[u8; 3]]) -> Theme {
        if scheme.len() < 4 {
            return Theme::default();
        }
        Theme {
            colors: scheme.to_vec(),
            ..Theme::default()
        }
    }

    pub fn color(&self, slot: ThemeSlot) -> Option<[u8; 3]> {
        self.colors.get(slot.scheme_index()).copied()
    }

    /// The face a `<w:rFonts w:*Theme>` reference names.
    pub fn font(&self, which: crate::prop::ThemeFont) -> Option<&str> {
        use crate::prop::ThemeFont::*;
        let scheme = if which.is_major() {
            &self.major
        } else {
            &self.minor
        };
        let face = match which {
            // `Ascii` and `HighAnsi` are both the Latin face: the split is a
            // legacy of code pages and the theme has one entry for both.
            MajorAscii | MajorHighAnsi | MinorAscii | MinorHighAnsi => &scheme.latin,
            MajorEastAsian | MinorEastAsian => &scheme.east_asian,
            MajorComplex | MinorComplex => &scheme.complex,
        };
        face.as_deref()
    }
}

/// `w:highlight` — the marker-pen palette, and the whole of it.
///
/// Not a [`Color`]: the attribute takes one of these names and nothing else, and
/// modelling it as a colour would let a writer author a value Word rejects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Highlight {
    #[default]
    None,
    Black,
    Blue,
    Cyan,
    DarkBlue,
    DarkCyan,
    DarkGray,
    DarkGreen,
    DarkMagenta,
    DarkRed,
    DarkYellow,
    Green,
    LightGray,
    Magenta,
    Red,
    White,
    Yellow,
}

impl Highlight {
    pub fn from_name(name: &str) -> Option<Highlight> {
        Some(match name {
            "none" => Highlight::None,
            "black" => Highlight::Black,
            "blue" => Highlight::Blue,
            "cyan" => Highlight::Cyan,
            "darkBlue" => Highlight::DarkBlue,
            "darkCyan" => Highlight::DarkCyan,
            "darkGray" => Highlight::DarkGray,
            "darkGreen" => Highlight::DarkGreen,
            "darkMagenta" => Highlight::DarkMagenta,
            "darkRed" => Highlight::DarkRed,
            "darkYellow" => Highlight::DarkYellow,
            "green" => Highlight::Green,
            "lightGray" => Highlight::LightGray,
            "magenta" => Highlight::Magenta,
            "red" => Highlight::Red,
            "white" => Highlight::White,
            "yellow" => Highlight::Yellow,
            _ => return None,
        })
    }

    pub const fn name(self) -> &'static str {
        match self {
            Highlight::None => "none",
            Highlight::Black => "black",
            Highlight::Blue => "blue",
            Highlight::Cyan => "cyan",
            Highlight::DarkBlue => "darkBlue",
            Highlight::DarkCyan => "darkCyan",
            Highlight::DarkGray => "darkGray",
            Highlight::DarkGreen => "darkGreen",
            Highlight::DarkMagenta => "darkMagenta",
            Highlight::DarkRed => "darkRed",
            Highlight::DarkYellow => "darkYellow",
            Highlight::Green => "green",
            Highlight::LightGray => "lightGray",
            Highlight::Magenta => "magenta",
            Highlight::Red => "red",
            Highlight::White => "white",
            Highlight::Yellow => "yellow",
        }
    }

    /// What to paint behind the text. `None` is not a colour.
    ///
    /// These are the sixteen VGA colours, which is exactly what the feature is:
    /// it predates theme colours by a decade and has never been extended.
    pub const fn rgb(self) -> Option<[u8; 3]> {
        Some(match self {
            Highlight::None => return None,
            Highlight::Black => [0x00, 0x00, 0x00],
            Highlight::Blue => [0x00, 0x00, 0xFF],
            Highlight::Cyan => [0x00, 0xFF, 0xFF],
            Highlight::DarkBlue => [0x00, 0x00, 0x80],
            Highlight::DarkCyan => [0x00, 0x80, 0x80],
            Highlight::DarkGray => [0x80, 0x80, 0x80],
            Highlight::DarkGreen => [0x00, 0x80, 0x00],
            Highlight::DarkMagenta => [0x80, 0x00, 0x80],
            Highlight::DarkRed => [0x80, 0x00, 0x00],
            Highlight::DarkYellow => [0x80, 0x80, 0x00],
            Highlight::Green => [0x00, 0xFF, 0x00],
            Highlight::LightGray => [0xC0, 0xC0, 0xC0],
            Highlight::Magenta => [0xFF, 0x00, 0xFF],
            Highlight::Red => [0xFF, 0x00, 0x00],
            Highlight::White => [0xFF, 0xFF, 0xFF],
            Highlight::Yellow => [0xFF, 0xFF, 0x00],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_colour_has_no_alpha_byte() {
        assert_eq!(
            Color::from_val("4472C4"),
            Some(Color::Rgb([0x44, 0x72, 0xC4]))
        );
        // Eight digits are a workbook's spelling; take the colour rather than
        // refusing a perfectly well specified one.
        assert_eq!(
            Color::from_val("FF4472C4"),
            Some(Color::Rgb([0x44, 0x72, 0xC4]))
        );
        assert_eq!(Color::from_val("auto"), Some(Color::Auto));
        assert_eq!(Color::from_val("zzz"), None);
        assert_eq!(Color::Rgb([0x44, 0x72, 0xC4]).to_val(), "4472C4");
    }

    #[test]
    fn text1_and_dark1_are_one_slot_under_two_names() {
        assert_eq!(ThemeSlot::from_name("text1"), Some(ThemeSlot::Dark1));
        assert_eq!(ThemeSlot::from_name("dark1"), Some(ThemeSlot::Dark1));
        assert_eq!(ThemeSlot::from_name("background1"), Some(ThemeSlot::Light1));
        assert_eq!(ThemeSlot::from_name("light1"), Some(ThemeSlot::Light1));
        assert_eq!(ThemeSlot::from_name("none"), None);
    }

    #[test]
    fn the_scheme_is_read_in_the_order_the_part_writes_it() {
        // Excel's `theme="0"` means lt1 and the part writes dk1 first, so a
        // reader has to swap. Word names its slots, so there is nothing to swap
        // — and this test exists so nobody copies the swap across.
        let theme = Theme::default();
        assert_eq!(theme.color(ThemeSlot::Dark1), Some([0, 0, 0]));
        assert_eq!(theme.color(ThemeSlot::Light1), Some([0xFF, 0xFF, 0xFF]));
    }

    #[test]
    fn a_tint_lightens_and_a_shade_darkens_the_same_slot() {
        let theme = Theme::default();
        let base = theme.color(ThemeSlot::Accent1).unwrap();
        let lighter = Color::Theme {
            slot: ThemeSlot::Accent1,
            tint: Some(0x99),
            shade: None,
        }
        .resolve(&theme)
        .unwrap();
        let darker = Color::Theme {
            slot: ThemeSlot::Accent1,
            tint: None,
            shade: Some(0xBF),
        }
        .resolve(&theme)
        .unwrap();
        for i in 0..3 {
            assert!(lighter[i] >= base[i], "tint must not darken channel {i}");
            assert!(darker[i] <= base[i], "shade must not lighten channel {i}");
        }
        // The endpoints are exact, and are what pin the direction of the byte.
        let white = tint_toward_white(base, 0x00);
        let black = shade_toward_black(base, 0x00);
        assert_eq!(white, [0xFF, 0xFF, 0xFF]);
        assert_eq!(black, [0x00, 0x00, 0x00]);
        assert_eq!(tint_toward_white(base, 0xFF), base);
        assert_eq!(shade_toward_black(base, 0xFF), base);
    }

    #[test]
    fn auto_and_an_undefined_slot_both_refuse_to_invent_a_colour() {
        let empty = Theme::from_scheme(&[[1, 2, 3]]);
        // Fewer than four entries is not a scheme; the default stands in.
        assert_eq!(empty.color(ThemeSlot::Dark1), Some([0, 0, 0]));

        let partial = Theme::from_scheme(&[[1, 1, 1], [2, 2, 2], [3, 3, 3], [4, 4, 4]]);
        assert_eq!(partial.color(ThemeSlot::Accent1), None);
        assert_eq!(
            Color::Theme {
                slot: ThemeSlot::Accent1,
                tint: None,
                shade: None
            }
            .resolve(&partial),
            None
        );
        assert_eq!(Color::Auto.resolve(&Theme::default()), None);
    }

    #[test]
    fn highlighting_is_the_sixteen_vga_colours_and_none() {
        assert_eq!(
            Highlight::from_name("darkYellow"),
            Some(Highlight::DarkYellow)
        );
        assert_eq!(Highlight::from_name("teal"), None);
        assert_eq!(Highlight::None.rgb(), None);
        assert_eq!(Highlight::Yellow.rgb(), Some([0xFF, 0xFF, 0x00]));
        for name in ["none", "black", "darkGray", "lightGray", "white"] {
            let h = Highlight::from_name(name).unwrap();
            assert_eq!(h.name(), name, "round trip through the name Word writes");
        }
    }
}
