//! The design language, as named numbers.
//!
//! One light neutral, one accent, no decoration: the page is the product and
//! the chrome recedes. Every colour and metric the chrome of either
//! application draws with is a constant here with a line saying what it is
//! for, so that a menu, a dialog, a strip and a status bar drawn in four files
//! are drawn from one palette — before this the accent green was written out
//! in four places and the hover tint in three, each a little different, and a
//! chrome whose parts disagree by two shades reads as homemade.
//!
//! The values are choices, not measurements: nothing here claims to be what
//! Word does. What *is* held to a number is legibility — every chrome ink is
//! at least 4.5 to 1 against every fill it is set on, computed by the test at
//! the bottom rather than judged by eye — and the desk is lighter than it was
//! so that the paper stays the brightest thing on the screen.

use eframe::egui::{self, Color32};

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// Menu bar, toolbar, strips, status bar and panes: the chrome's one fill.
pub const CHROME: Color32 = rgb(0xF7, 0xF7, 0xF5);
/// The hairlines between the chrome's parts.
pub const CHROME_RULE: Color32 = rgb(0xE2, 0xE2, 0xDF);
/// Chrome text and icon strokes.
pub const INK: Color32 = rgb(0x1F, 0x1F, 0x1F);
/// Secondary text — counts, dates, hints. Set on the tinted fills as well as
/// the plain one, so it is as dark as it is: `#6B6B6B` reads 4.3 to 1 on
/// [`TINT_ON`], and this reads 4.9.
pub const INK_SOFT: Color32 = rgb(0x62, 0x62, 0x62);
/// A sentence about something refused or failed — a key a service did not
/// accept, a choice that cannot be kept — set in the chrome's running size.
/// The message boxes' error red is a badge's fill; this is its ink, dark
/// enough to read on every fill a pane or a box sets it on.
pub const INK_ERROR: Color32 = rgb(0xB0, 0x26, 0x18);
/// Disabled text and strokes. Not held to a contrast: disabled is meant to
/// read as out of reach.
pub const INK_FAINT: Color32 = rgb(0xB0, 0xB0, 0xB0);

/// The suite's accent — primary buttons, a toggled control's bar, the focus
/// ring, the selection. Deliberately not any vendor's brand colour.
pub const ACCENT: Color32 = rgb(0x1E, 0x6F, 0x5C);
pub const ACCENT_HOVER: Color32 = rgb(0x2A, 0x8B, 0x74);
pub const ACCENT_DOWN: Color32 = rgb(0x17, 0x56, 0x4A);

/// A flat control under the pointer.
pub const TINT_HOVER: Color32 = rgb(0xE9, 0xF0, 0xEC);
/// A flat control being pressed.
pub const TINT_DOWN: Color32 = rgb(0xD3, 0xE3, 0xDA);
/// A flat control that is on — a toggle, a lit chip, a held menu title.
pub const TINT_ON: Color32 = rgb(0xDC, 0xEA, 0xE2);

/// Inputs and combos.
pub const FIELD: Color32 = Color32::WHITE;
pub const FIELD_EDGE: Color32 = rgb(0xC8, 0xC8, 0xC5);
/// A field's edge under the pointer or holding the keyboard.
pub const FIELD_EDGE_HOT: Color32 = rgb(0x8C, 0x8C, 0x88);

/// The surface the pages sit on. Mid-light rather than dark: the dark desk
/// made the window heavy and the chrome-coloured panes read as brighter than
/// the document. A judgement, not a measurement.
pub const DESK: Color32 = rgb(0xCF, 0xD1, 0xD4);
/// A page's one-pixel outline.
pub const PAGE_EDGE: Color32 = rgb(0xB8, 0xBA, 0xBD);
/// The shadow under each page, which on a light desk does the work of
/// separating paper from desk that a dark desk did by contrast alone.
pub const PAGE_SHADOW: egui::epaint::Shadow = egui::epaint::Shadow {
    offset: [0, 2],
    blur: 10,
    spread: 0,
    color: Color32::from_black_alpha(46),
};

/// Text selection on the page: the accent at 0.28, premultiplied. Calx
/// selects in the accent already, and a suite whose two applications select
/// in different colours reads as two products.
pub const SELECTION: Color32 = Color32::from_rgba_premultiplied(8, 31, 26, 71);
/// Every match of the find bar's query: `#FFD84D` at 0.55, premultiplied.
pub const MATCH: Color32 = Color32::from_rgba_premultiplied(140, 119, 42, 140);
/// The current match's outline.
pub const MATCH_CURRENT: Color32 = ACCENT;

/// The notice bar under the toolbar: its fill and its rule.
pub const NOTICE: Color32 = rgb(0xFF, 0xF4, 0xCE);
pub const NOTICE_RULE: Color32 = rgb(0xE5, 0xC5, 0x58);

/// Tracked-change and comment colours, assigned in order of an author's
/// first appearance in the document.
pub const AUTHORS: [Color32; 8] = [
    rgb(0xC0, 0x39, 0x2B),
    rgb(0x1F, 0x6F, 0xB2),
    rgb(0x2E, 0x8B, 0x57),
    rgb(0x8E, 0x44, 0xAD),
    rgb(0xB7, 0x77, 0x0D),
    rgb(0x12, 0x85, 0x7A),
    rgb(0xB0, 0x30, 0x60),
    rgb(0x5D, 0x6D, 0x7E),
];

/// The colour an author's changes wear, wrapping round once eight are used
/// up — a document with nine reviewers is a document whose ninth shares a
/// colour, which is what Word does too.
pub fn author(index: usize) -> Color32 {
    AUTHORS[index % AUTHORS.len()]
}

/// Corner radii: controls, menus and popovers, dialogs.
pub const RADIUS_CONTROL: u8 = 4;
pub const RADIUS_MENU: u8 = 6;
pub const RADIUS_DIALOG: u8 = 8;

/// Chrome type: the running size, the secondary size, dialog headings.
pub const TEXT: f32 = 13.0;
pub const TEXT_SMALL: f32 = 12.0;
pub const HEADING: f32 = 15.0;

/// An icon: the glyph's side, its stroke, and the target it sits in — which
/// is the least a pointer can reliably hit.
pub const ICON: f32 = 16.0;
pub const ICON_STROKE: f32 = 1.5;
pub const TARGET: egui::Vec2 = egui::vec2(28.0, 24.0);

/// Row heights, top to bottom of the window.
pub const MENU_BAR: f32 = 28.0;
pub const TOOLBAR: f32 = 36.0;
pub const STRIP: f32 = 32.0;
pub const STATUS: f32 = 26.0;
pub const PANE_HEADER: f32 = 30.0;

/// How long the caret shows, and how long it hides, in seconds.
pub const BLINK: f64 = 0.53;

/// The contrast ratio between two colours, as WCAG 2 defines it: the
/// relative luminance of the lighter plus 0.05 over that of the darker plus
/// 0.05. Text is legible at 4.5 to 1 and comfortable at 7.
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (l1, l2) = (luminance(a), luminance(b));
    let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

fn luminance(colour: Color32) -> f32 {
    let channel = |value: u8| {
        let c = value as f32 / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(colour.r()) + 0.7152 * channel(colour.g()) + 0.0722 * channel(colour.b())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_text_reads_against_every_fill_it_is_set_on() {
        // The inks that carry words, against every fill they are set on. The
        // faint ink is left out on purpose: disabled is meant to look out of
        // reach, and a disabled row that reads as well as a live one is a row
        // nobody can tell is disabled.
        let fills = [CHROME, FIELD, TINT_HOVER, TINT_DOWN, TINT_ON, NOTICE];
        for ink in [INK, INK_SOFT, INK_ERROR] {
            for fill in fills {
                let ratio = contrast(ink, fill);
                assert!(
                    ratio >= 4.5,
                    "{ink:?} on {fill:?} reads at {ratio:.2}:1, under 4.5"
                );
            }
        }
        // The desk carries only the page badge, which is a chrome pill — but
        // the running ink is set straight on it in tests and probes.
        assert!(contrast(INK, DESK) >= 4.5);
        // White on the accent, which is every primary button.
        assert!(contrast(Color32::WHITE, ACCENT) >= 4.5);
        // And the formula itself, against the two answers everybody knows.
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01);
        assert!((contrast(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 0.001);
    }

    #[test]
    fn the_authors_colours_wrap_round_rather_than_run_out() {
        assert_eq!(author(0), AUTHORS[0]);
        assert_eq!(author(8), AUTHORS[0]);
        assert_eq!(author(11), AUTHORS[3]);
    }
}
