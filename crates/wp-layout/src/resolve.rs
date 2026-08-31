//! From resolved properties to something a renderer can draw with.
//!
//! [`crate::style`]-level resolution — the `basedOn` chain, the toggles, the
//! numbering layer — happens in `wp-model`. What is left is the part that needs
//! the *theme* and the *characters*: which of a run's four faces draws a given
//! letter, what a themed colour comes out as, and how big a superscript is.

use std::sync::Arc;

use wp_model::color::Theme;
use wp_model::prop::{RunProps, Script, Toggle, UnderlineKind, VertAlign};

use crate::shape::FontRequest;

/// Everything needed to draw one stretch of text.
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub font: FontRequest,
    /// `None` means the document did not choose — the renderer uses its own
    /// foreground, which is what lets a dark UI show `w:val="auto"` as light.
    pub color: Option<[u8; 3]>,
    pub highlight: Option<[u8; 3]>,
    pub shading: Option<[u8; 3]>,
    /// `<w:bdr>` — a box drawn around the run itself, with its colour already
    /// resolved. It costs the line room on all four sides: see
    /// [`TextStyle::border_pad`].
    pub border: Option<wp_model::prop::Border>,
    pub underline: UnderlineKind,
    pub underline_color: Option<[u8; 3]>,
    pub strike: bool,
    pub double_strike: bool,
    pub caps: bool,
    pub small_caps: bool,
    /// Baseline shift in points, positive up. Superscript and subscript
    /// contribute here as well as shrinking the face.
    pub raise: f64,
    /// Extra space between characters, in points.
    pub letter_spacing: f64,
    /// `w:vanish` — drawn only when formatting marks are showing.
    pub hidden: bool,
    pub rtl: bool,
}

impl TextStyle {
    /// The room a run's own border takes on every side of it.
    ///
    /// Measured against Word over `w:sz` 2 to 24 and `w:space` 0 and 4: the
    /// line grows by the gap plus the rule's thickness above and below, and
    /// the run is pushed the same distance right of where it would have sat.
    pub fn border_pad(&self) -> f64 {
        self.border
            .map(|b| f64::from(b.space.unwrap_or(0)) + b.size.map(|s| s.points()).unwrap_or(0.5))
            .unwrap_or(0.0)
    }

    /// Applies the character-level transformations the *style* performs rather
    /// than the font: `w:caps` and `w:smallCaps` change what glyphs are drawn.
    ///
    /// Small caps are drawn as capitals at a smaller size, which is what Word
    /// does when the face has no true small-cap glyphs — and which no face this
    /// application loads does have.
    pub fn transform(&self, text: &str) -> Option<String> {
        if self.caps || self.small_caps {
            Some(text.to_uppercase())
        } else {
            None
        }
    }

    /// The face small capitals are drawn in — the same font, smaller.
    pub fn small_cap_font(&self) -> FontRequest {
        FontRequest {
            size: self.font.size * 0.8,
            ..self.font.clone()
        }
    }

    /// **The face this run's glyphs are actually drawn in**, which is not
    /// always the face it names.
    ///
    /// One function rather than a rule each renderer remembers, because for a
    /// while they did not remember it: the line was *measured* with the smaller
    /// face and both the screen and the paper drew the letters at the size the
    /// style named, so every small-capped heading in every document was set a
    /// quarter too large on advances computed for something smaller. It was
    /// invisible to a comparison that only counts where a word begins, and
    /// obvious the first time anybody put the two pages side by side.
    pub fn drawn_font(&self) -> FontRequest {
        match self.small_caps {
            true => self.small_cap_font(),
            false => self.font.clone(),
        }
    }
}

/// Which of a run's four faces draws a character.
///
/// Word decides this from the character's code page coverage and from
/// `w:hint`, in a table that runs to several pages. This is the readable
/// version of it: the scripts that have their own face in practice, and Latin
/// for everything else.
///
/// **Stated limit.** The `hAnsi`/`ascii` split is a code-page distinction from
/// the 1990s — the two faces are the same in every document this has been run
/// against — and is resolved here at U+0080 rather than by code page.
pub fn face_for(c: char) -> Script {
    match c {
        '\u{0000}'..='\u{007F}' => Script::Ascii,
        // Arabic, Hebrew, Syriac, Thaana, and the Indic and Thai blocks, all of
        // which Word draws with the complex-script face.
        '\u{0590}'..='\u{08FF}'
        | '\u{0900}'..='\u{0DFF}'
        | '\u{0E00}'..='\u{0E7F}'
        | '\u{FB1D}'..='\u{FEFC}' => Script::Complex,
        '\u{2E80}'..='\u{303F}'
        | '\u{3040}'..='\u{30FF}'
        | '\u{3100}'..='\u{312F}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{AC00}'..='\u{D7AF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{FF00}'..='\u{FFEF}' => Script::EastAsian,
        _ => Script::HighAnsi,
    }
}

/// The font family a run uses for a given script.
///
/// The *theme reference* wins over the cached name beside it, because
/// `w:asciiTheme="minorHAnsi"` is what stays right after a theme change and
/// `w:ascii="Calibri"` is a copy of what it resolved to when the file was saved.
pub fn family(props: &RunProps, theme: &Theme, script: Script, fallback: &str) -> Arc<str> {
    let fonts = &props.fonts;
    let (reference, name) = match script {
        Script::Ascii => (fonts.ascii_theme, &fonts.ascii),
        Script::HighAnsi => (fonts.high_ansi_theme, &fonts.high_ansi),
        Script::EastAsian => (fonts.east_asian_theme, &fonts.east_asian),
        Script::Complex => (fonts.complex_theme, &fonts.complex),
    };
    if let Some(face) = reference.and_then(|which| theme.font(which)) {
        return face.into();
    }
    if let Some(name) = name {
        return name.clone();
    }
    // A run that names no face for this script falls back to the one it names
    // for Latin before it falls back to the application's own.
    fonts
        .ascii
        .clone()
        .or_else(|| fonts.high_ansi.clone())
        .or_else(|| {
            fonts
                .ascii_theme
                .and_then(|which| theme.font(which))
                .map(Into::into)
        })
        .unwrap_or_else(|| fallback.into())
}

/// Turns resolved run properties into a drawing style.
pub fn text_style(props: &RunProps, theme: &Theme, script: Script, fallback: &str) -> TextStyle {
    let size = match script {
        Script::Complex => props.complex_font_size(),
        _ => props.font_size(),
    };
    let mut font = FontRequest {
        family: family(props, theme, script, fallback),
        size: size.points(),
        bold: props.toggles.is_on(match script {
            Script::Complex => Toggle::BoldCs,
            _ => Toggle::Bold,
        }) || props.toggles.is_on(Toggle::Bold),
        italic: props.toggles.is_on(Toggle::Italic) || props.toggles.is_on(Toggle::ItalicCs),
        // `<w:kern>` is a threshold, not a switch: it names the size at or
        // above which Word closes up the pairs, and zero means never. Word's
        // own document defaults name two half-points, so this is on for
        // ordinary prose and the exception is a run that turns it off.
        kern: props
            .kern
            .is_some_and(|least| least.0 > 0 && f64::from(least.0) / 2.0 <= size.points()),
    };

    // Superscript and subscript are a position, and Word shrinks the glyphs
    // itself rather than storing a size. The shift is a third of the original
    // size up, or a fifth down, which is what Word's own rendering does.
    let raise = match props.vert_align.unwrap_or_default() {
        VertAlign::Baseline => props.raise.map(|r| r.points()).unwrap_or(0.0),
        VertAlign::Superscript => {
            let shift = font.size / 3.0;
            font = font.shrunk();
            shift
        }
        VertAlign::Subscript => {
            let shift = -font.size / 5.0;
            font = font.shrunk();
            shift
        }
    };

    let underline = props.underline.as_ref();
    TextStyle {
        font,
        color: props.color.and_then(|c| c.resolve(theme)),
        highlight: props.highlight.and_then(|h| h.rgb()),
        shading: props
            .shading
            .and_then(|s| s.background())
            .and_then(|c| c.resolve(theme)),
        border: props.border.filter(|b| b.style.draws()).map(|b| {
            // Resolved here for the same reason a paragraph's border is: the
            // theme is in reach now and is gone by the time a page is painted.
            let mut b = b;
            b.color = b.color.map(|c| match c.resolve(theme) {
                Some(rgb) => wp_model::color::Color::Rgb(rgb),
                None => wp_model::color::Color::Auto,
            });
            b
        }),
        underline: underline.map(|u| u.kind).unwrap_or_default(),
        underline_color: underline
            .and_then(|u| u.color)
            .and_then(|c| c.resolve(theme)),
        strike: props.toggles.is_on(Toggle::Strike),
        double_strike: props.toggles.is_on(Toggle::DoubleStrike),
        caps: props.toggles.is_on(Toggle::Caps),
        small_caps: props.toggles.is_on(Toggle::SmallCaps),
        raise,
        letter_spacing: props.letter_spacing.map(|s| s.points()).unwrap_or(0.0),
        hidden: props.hidden(),
        rtl: props.rtl.unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wp_model::color::{Color, ThemeSlot};
    use wp_model::prop::{Fonts, ThemeFont, Underline};
    use wp_model::units::HalfPoint;

    fn plain() -> RunProps {
        RunProps {
            size: Some(HalfPoint(22)),
            ..RunProps::default()
        }
    }

    #[test]
    fn a_character_chooses_which_of_the_runs_four_faces_draws_it() {
        assert_eq!(face_for('a'), Script::Ascii);
        assert_eq!(face_for('é'), Script::HighAnsi);
        assert_eq!(face_for('日'), Script::EastAsian);
        assert_eq!(face_for('م'), Script::Complex);
    }

    #[test]
    fn a_theme_reference_beats_the_cached_name_beside_it() {
        let theme = Theme::default();
        let mut props = plain();
        props.fonts = Fonts {
            ascii: Some("Stale Face".into()),
            ascii_theme: Some(ThemeFont::MinorHighAnsi),
            ..Fonts::default()
        };
        assert_eq!(
            family(&props, &theme, Script::Ascii, "fallback").as_ref(),
            "Calibri"
        );
    }

    #[test]
    fn a_run_with_no_east_asian_face_falls_back_to_its_latin_one() {
        let theme = Theme::default();
        let mut props = plain();
        props.fonts.ascii = Some("Arial".into());
        assert_eq!(
            family(&props, &theme, Script::EastAsian, "fallback").as_ref(),
            "Arial"
        );
        // And a run that names nothing at all falls back to the application's.
        assert_eq!(
            family(&plain(), &theme, Script::Ascii, "fallback").as_ref(),
            "fallback"
        );
    }

    #[test]
    fn a_complex_script_run_takes_its_size_from_its_own_property() {
        let theme = Theme::default();
        let mut props = plain();
        props.size_complex = Some(HalfPoint(28));
        assert_eq!(
            text_style(&props, &theme, Script::Complex, "f").font.size,
            14.0
        );
        assert_eq!(
            text_style(&props, &theme, Script::Ascii, "f").font.size,
            11.0
        );
    }

    #[test]
    fn a_superscript_is_shrunk_and_lifted_without_the_document_saying_either() {
        let theme = Theme::default();
        let mut props = plain();
        props.vert_align = Some(VertAlign::Superscript);
        let style = text_style(&props, &theme, Script::Ascii, "f");
        assert!(style.font.size < 11.0);
        assert!(style.raise > 0.0);

        props.vert_align = Some(VertAlign::Subscript);
        let style = text_style(&props, &theme, Script::Ascii, "f");
        assert!(style.raise < 0.0);
    }

    #[test]
    fn a_themed_colour_is_resolved_and_auto_is_left_for_the_renderer() {
        let theme = Theme::default();
        let mut props = plain();
        props.color = Some(Color::Theme {
            slot: ThemeSlot::Accent1,
            tint: None,
            shade: None,
        });
        assert_eq!(
            text_style(&props, &theme, Script::Ascii, "f").color,
            Some([0x44, 0x72, 0xC4])
        );

        props.color = Some(Color::Auto);
        assert_eq!(text_style(&props, &theme, Script::Ascii, "f").color, None);
    }

    #[test]
    fn small_capitals_are_drawn_as_capitals_because_the_face_has_none() {
        let theme = Theme::default();
        let mut props = plain();
        props.toggles.set(Toggle::SmallCaps, true);
        let style = text_style(&props, &theme, Script::Ascii, "f");
        assert_eq!(style.transform("Heading").as_deref(), Some("HEADING"));
        assert!(style.small_cap_font().size < style.font.size);
        assert!(
            TextStyle::transform(&text_style(&plain(), &theme, Script::Ascii, "f"), "plain")
                .is_none()
        );
    }

    #[test]
    fn an_underline_carries_its_own_colour_and_a_plain_run_has_none() {
        let theme = Theme::default();
        let mut props = plain();
        props.underline = Some(Underline {
            kind: UnderlineKind::Wave,
            color: Some(Color::Rgb([0xFF, 0, 0])),
        });
        let style = text_style(&props, &theme, Script::Ascii, "f");
        assert_eq!(style.underline, UnderlineKind::Wave);
        assert_eq!(style.underline_color, Some([0xFF, 0, 0]));
        assert_eq!(
            text_style(&plain(), &theme, Script::Ascii, "f").underline,
            UnderlineKind::None
        );
    }
}
