//! Colours, and the four different ways a spreadsheet writes one down.
//!
//! A cell's colour is almost never an RGB triple in the file. It is one of:
//!
//! - `rgb="FF4472C4"` — ARGB, the only self-contained spelling.
//! - `theme="4" tint="-0.25"` — an index into the *document theme*, darkened.
//!   This is what the whole modern Excel palette uses, so a reader that ignores
//!   it renders a normally-formatted workbook in black on white.
//! - `indexed="10"` — the 1997 fifty-six-colour palette, still emitted by older
//!   producers and by anything that writes BIFF-shaped XML.
//! - `auto="1"` — "whatever the window is", which is not a colour at all and
//!   must stay unresolved so the grid can use its own foreground.
//!
//! Two of these need outside information: the theme part for `theme`, and a
//! baked-in table for `indexed`. Both live here so that no caller has to know
//! which spelling it is looking at.

/// A colour as a file spells it, before anything outside the attribute is known.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Color {
    /// The window's own foreground or background. Deliberately not resolved to
    /// black: a dark theme needs it to come out light.
    #[default]
    Auto,
    /// Straight ARGB. The alpha byte is kept because Excel writes `FF` and
    /// dropping it would make a written colour differ from the one read.
    Rgb([u8; 4]),
    /// Into the legacy fifty-six-colour palette.
    Indexed(u32),
    /// Into the document theme's colour scheme, lightened or darkened by `tint`.
    Theme { index: u32, tint: f64 },
}

impl Color {
    pub const BLACK: Color = Color::Rgb([0xFF, 0x00, 0x00, 0x00]);
    pub const WHITE: Color = Color::Rgb([0xFF, 0xFF, 0xFF, 0xFF]);

    pub fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color::Rgb([0xFF, r, g, b])
    }

    /// Parses the `rgb` attribute, which is `AARRGGBB` but is sometimes written
    /// as `RRGGBB` by producers other than Excel.
    pub fn from_hex(text: &str) -> Option<Color> {
        let text = text.trim();
        let bytes: Vec<u8> = (0..text.len() / 2)
            .map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16))
            .collect::<std::result::Result<_, _>>()
            .ok()?;
        match bytes.len() {
            4 => Some(Color::Rgb([bytes[0], bytes[1], bytes[2], bytes[3]])),
            3 => Some(Color::Rgb([0xFF, bytes[0], bytes[1], bytes[2]])),
            _ => None,
        }
    }

    /// The colour as `AARRGGBB`, for a writer.
    pub fn to_hex(self) -> Option<String> {
        match self {
            Color::Rgb([a, r, g, b]) => Some(format!("{a:02X}{r:02X}{g:02X}{b:02X}")),
            _ => None,
        }
    }

    pub fn is_auto(self) -> bool {
        matches!(self, Color::Auto)
    }

    /// Resolves to RGB, or `None` for a colour that has no answer without
    /// knowing what it is being drawn on.
    ///
    /// `Auto` is the honest `None`. So is a theme index the document does not
    /// define — inventing a colour there would be worse than falling back to
    /// the window's foreground, which is what Excel does when a theme is
    /// missing.
    pub fn resolve(self, theme: &Theme) -> Option<[u8; 3]> {
        match self {
            Color::Auto => None,
            Color::Rgb([_, r, g, b]) => Some([r, g, b]),
            Color::Indexed(i) => indexed(i),
            Color::Theme { index, tint } => theme.color(index).map(|base| apply_tint(base, tint)),
        }
    }
}

/// The document's colour scheme, from `theme1.xml`.
#[derive(Debug, Clone)]
pub struct Theme {
    /// In *Excel's* index order, which is not the order the part writes them.
    colors: Vec<[u8; 3]>,
}

impl Default for Theme {
    /// The Office default scheme, so a workbook whose theme part is missing or
    /// unreadable still renders in the colours its author saw.
    fn default() -> Self {
        Theme {
            colors: vec![
                [0xFF, 0xFF, 0xFF], // 0 background 1 (lt1)
                [0x00, 0x00, 0x00], // 1 text 1 (dk1)
                [0xE7, 0xE6, 0xE6], // 2 background 2 (lt2)
                [0x44, 0x54, 0x6A], // 3 text 2 (dk2)
                [0x44, 0x72, 0xC4], // 4 accent 1
                [0xED, 0x7D, 0x31], // 5 accent 2
                [0xA5, 0xA5, 0xA5], // 6 accent 3
                [0xFF, 0xC0, 0x00], // 7 accent 4
                [0x5B, 0x9B, 0xD5], // 8 accent 5
                [0x70, 0xAD, 0x47], // 9 accent 6
                [0x05, 0x63, 0xC1], // 10 hyperlink
                [0x95, 0x4F, 0x72], // 11 followed hyperlink
            ],
        }
    }
}

impl Theme {
    /// Builds from the scheme colours **in the order `<a:clrScheme>` writes
    /// them**: dk1, lt1, dk2, lt2, accent1..6, hlink, folHlink.
    ///
    /// The first two pairs are then swapped, because a `theme="0"` attribute
    /// means *background 1* and the part's first entry is *dark 1*. Nothing in
    /// either file says so; getting it wrong paints every themed heading white
    /// on white, which reads as "the text vanished" rather than as an off-by-one.
    pub fn from_scheme(scheme: &[[u8; 3]]) -> Theme {
        let at = |i: usize| scheme.get(i).copied();
        let mut colors = Vec::with_capacity(12);
        for index in [1usize, 0, 3, 2] {
            match at(index) {
                Some(c) => colors.push(c),
                None => return Theme::default(),
            }
        }
        for index in 4..12 {
            match at(index) {
                Some(c) => colors.push(c),
                None => break,
            }
        }
        Theme { colors }
    }

    pub fn color(&self, index: u32) -> Option<[u8; 3]> {
        self.colors.get(index as usize).copied()
    }

    pub fn len(&self) -> usize {
        self.colors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.colors.is_empty()
    }
}

/// Lightens or darkens through HSL, which is what `tint` is defined against.
///
/// Doing it in RGB instead — scaling each channel — turns a tinted red into a
/// washed-out pink of the wrong hue. The difference is obvious side by side and
/// invisible in isolation, which is the worst kind of wrong.
pub fn apply_tint(rgb: [u8; 3], tint: f64) -> [u8; 3] {
    if tint == 0.0 {
        return rgb;
    }
    let (h, s, l) = to_hsl(rgb);
    let l = if tint < 0.0 {
        l * (1.0 + tint)
    } else {
        l * (1.0 - tint) + tint
    };
    from_hsl(h, s, l.clamp(0.0, 1.0))
}

fn to_hsl(rgb: [u8; 3]) -> (f64, f64, f64) {
    let [r, g, b] = rgb.map(|c| f64::from(c) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f64::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;
    (h, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> [u8; 3] {
    if s == 0.0 {
        let v = (l * 255.0).round() as u8;
        return [v, v, v];
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let channel = |mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).round() as u8
    };
    [channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0)]
}

/// The legacy palette. Index 64 is the system foreground and 65 the system
/// background — both are `Auto` in disguise, so neither resolves.
fn indexed(index: u32) -> Option<[u8; 3]> {
    const PALETTE: [u32; 64] = [
        0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x000000,
        0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x800000, 0x008000,
        0x000080, 0x808000, 0x800080, 0x008080, 0xC0C0C0, 0x808080, 0x9999FF, 0x993366, 0xFFFFCC,
        0xCCFFFF, 0x660066, 0xFF8080, 0x0066CC, 0xCCCCFF, 0x000080, 0xFF00FF, 0xFFFF00, 0x00FFFF,
        0x800080, 0x800000, 0x008080, 0x0000FF, 0x00CCFF, 0xCCFFFF, 0xCCFFCC, 0xFFFF99, 0x99CCFF,
        0xFF99CC, 0xCC99FF, 0xFFCC99, 0x3366FF, 0x33CCCC, 0x99CC00, 0xFFCC00, 0xFF9900, 0xFF6600,
        0x666699, 0x969696, 0x003366, 0x339966, 0x003300, 0x333300, 0x993300, 0x993366, 0x333399,
        0x333333,
    ];
    let packed = *PALETTE.get(index as usize)?;
    Some([
        (packed >> 16) as u8,
        ((packed >> 8) & 0xFF) as u8,
        (packed & 0xFF) as u8,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argb_and_rgb_are_both_accepted() {
        assert_eq!(
            Color::from_hex("FF4472C4"),
            Some(Color::rgb(0x44, 0x72, 0xC4))
        );
        assert_eq!(
            Color::from_hex("4472C4"),
            Some(Color::rgb(0x44, 0x72, 0xC4))
        );
        assert_eq!(Color::from_hex("nonsense"), None);
        assert_eq!(Color::rgb(1, 2, 3).to_hex().as_deref(), Some("FF010203"));
    }

    #[test]
    fn theme_zero_is_the_background_not_the_first_entry() {
        // `<a:clrScheme>` writes dk1 first, and `theme="0"` means background 1.
        // If they were read in order, every themed heading would be white.
        let scheme = [
            [0x11, 0x11, 0x11], // dk1
            [0xEE, 0xEE, 0xEE], // lt1
            [0x22, 0x22, 0x22], // dk2
            [0xDD, 0xDD, 0xDD], // lt2
            [0x44, 0x72, 0xC4], // accent1
        ];
        let theme = Theme::from_scheme(&scheme);
        assert_eq!(theme.color(0), Some([0xEE, 0xEE, 0xEE]), "lt1");
        assert_eq!(theme.color(1), Some([0x11, 0x11, 0x11]), "dk1");
        assert_eq!(theme.color(2), Some([0xDD, 0xDD, 0xDD]), "lt2");
        assert_eq!(theme.color(3), Some([0x22, 0x22, 0x22]), "dk2");
        assert_eq!(theme.color(4), Some([0x44, 0x72, 0xC4]), "accent1");
    }

    #[test]
    fn a_short_scheme_falls_back_rather_than_shifting_everything() {
        let theme = Theme::from_scheme(&[[0, 0, 0], [255, 255, 255]]);
        assert_eq!(theme.color(0), Theme::default().color(0));
    }

    #[test]
    fn tint_lightens_and_darkens_without_changing_hue() {
        let base = [0x44, 0x72, 0xC4];
        let lighter = apply_tint(base, 0.6);
        let darker = apply_tint(base, -0.5);
        let sum = |c: [u8; 3]| u32::from(c[0]) + u32::from(c[1]) + u32::from(c[2]);
        assert!(sum(lighter) > sum(base), "{lighter:?}");
        assert!(sum(darker) < sum(base), "{darker:?}");
        // Blue stays the dominant channel in both directions.
        assert!(lighter[2] > lighter[0] && darker[2] > darker[0]);
        assert_eq!(apply_tint(base, 0.0), base);
    }

    #[test]
    fn auto_never_resolves_so_the_window_decides() {
        let theme = Theme::default();
        assert_eq!(Color::Auto.resolve(&theme), None);
        assert_eq!(
            Color::Indexed(64).resolve(&theme),
            None,
            "system foreground"
        );
        assert_eq!(Color::Indexed(2).resolve(&theme), Some([0xFF, 0, 0]));
        assert_eq!(
            Color::Theme {
                index: 99,
                tint: 0.0
            }
            .resolve(&theme),
            None
        );
    }
}
