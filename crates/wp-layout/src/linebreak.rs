//! Where a line may be broken.
//!
//! A documented subset of UAX #14. The full algorithm is a 40-class pair table
//! and about nine hundred lines of tailoring; what is here is the part that
//! decides where real prose breaks, plus the East Asian rules, and it is stated
//! as a small class set with a pair table of its own so that adding a class is
//! adding a row rather than editing a chain of `if`s.
//!
//! **Stated limit.** Not implemented: the Korean syllable classes, regional
//! indicators, emoji ZWJ sequences, the numeric-sequence rules beyond a simple
//! digit-separator-digit run, and line-break tailoring by language. A document
//! containing them breaks at a coarser granularity than Word would choose — it
//! does not break wrongly, it breaks less often.
//!
//! The one rule that is *not* a nicety: **a break opportunity is after the
//! spaces, not before them.** A line ending "the cat " is broken after the
//! space, and the space then hangs past the right margin rather than counting
//! toward the width. Getting this backwards makes every justified line one word
//! short.

/// The character classes this engine distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// Ordinary letters and anything unclassified.
    Alphabetic,
    Digit,
    /// A space that a break may follow.
    Space,
    /// U+00A0 and friends — a space that forbids a break on both sides.
    NoBreakSpace,
    /// A hyphen, after which a break is allowed.
    Hyphen,
    /// U+00AD — invisible, and a break opportunity that draws a hyphen.
    SoftHyphen,
    /// `(` `[` `{` `“` — no break *after*.
    Open,
    /// `)` `]` `}` `”` `,` `.` `;` `:` `!` `?` — no break *before*.
    Close,
    /// `/` — a break is allowed after it, which is how a long URL wraps.
    Slash,
    /// An ideograph or kana: a break is allowed on either side of one.
    Ideographic,
    /// CJK opening brackets — no break after.
    IdeographicOpen,
    /// CJK closing brackets and punctuation — no break before.
    IdeographicClose,
    /// U+200B, which is a break opportunity and nothing else.
    ZeroWidthSpace,
    /// U+2060 and U+FEFF — the opposite: forbid a break here.
    WordJoiner,
}

fn class(c: char) -> Class {
    use Class::*;
    match c {
        '\u{00A0}' | '\u{202F}' | '\u{2007}' => NoBreakSpace,
        '\u{00AD}' => SoftHyphen,
        '\u{200B}' => ZeroWidthSpace,
        '\u{2060}' | '\u{FEFF}' => WordJoiner,
        // U+2011 is the non-breaking hyphen and is deliberately Alphabetic.
        '-' | '\u{2010}' | '\u{2012}' | '\u{2013}' | '\u{2014}' => Hyphen,
        '/' => Slash,
        '(' | '[' | '{' | '\u{201C}' | '\u{2018}' | '\u{00AB}' => Open,
        ')' | ']' | '}' | '\u{201D}' | '\u{2019}' | '\u{00BB}' | ',' | '.' | ';' | ':' | '!'
        | '?' | '\u{2026}' => Close,
        // Listed rather than ranged: the CJK brackets alternate open and close
        // right through U+3008..U+301B, so a range for either would swallow
        // both halves and every closing bracket would forbid the wrong break.
        '\u{3008}' | '\u{300A}' | '\u{300C}' | '\u{300E}' | '\u{3010}' | '\u{3014}'
        | '\u{3016}' | '\u{3018}' | '\u{301A}' | '\u{FF08}' => IdeographicOpen,
        '\u{3001}' | '\u{3002}' | '\u{3009}' | '\u{300B}' | '\u{300D}' | '\u{300F}'
        | '\u{3011}' | '\u{3015}' | '\u{3017}' | '\u{3019}' | '\u{301B}' | '\u{FF01}'
        | '\u{FF09}' | '\u{FF0C}' | '\u{FF0E}' | '\u{FF1F}' => IdeographicClose,
        // CJK ideographs, kana, Hangul syllables, and the fullwidth forms.
        '\u{2E80}'..='\u{303F}'
        | '\u{3040}'..='\u{30FF}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{AC00}'..='\u{D7AF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{FF00}'..='\u{FF60}' => Ideographic,
        c if c.is_whitespace() => Space,
        c if c.is_ascii_digit() => Digit,
        _ => Alphabetic,
    }
}

/// Whether a line may break between `before` and `after`.
fn may_break(before: Class, after: Class) -> bool {
    use Class::*;
    match (before, after) {
        // Nothing breaks around a word joiner or a no-break space, whatever
        // else is beside them. Checked first so no later rule can override it.
        (WordJoiner, _) | (_, WordJoiner) => false,
        (NoBreakSpace, _) | (_, NoBreakSpace) => false,

        // A break is *after* a run of spaces, never before one, and never
        // between two of them.
        (Space, Space) => false,
        (_, Space) => false,
        (Space, _) => true,

        (ZeroWidthSpace, _) => true,
        (_, ZeroWidthSpace) => false,

        // Never before closing punctuation, never after opening.
        (_, Close) | (_, IdeographicClose) => false,
        (Open, _) | (IdeographicOpen, _) => false,

        (SoftHyphen, _) => true,
        // A hyphen breaks after itself, except between two digits: `1-2` and a
        // phone number stay whole.
        (Hyphen, Digit) => false,
        (Hyphen, _) => true,
        (_, Hyphen) => false,
        (Slash, _) => true,

        // A digit run is not broken by its separators: 1,234.56 is one unit,
        // which is what the Close rules above already give for `,` and `.`.
        (Digit, Digit) => false,

        // East Asian text has no spaces, so a break is allowed between any two
        // ideographs — subject to the bracket rules above.
        (Ideographic, Ideographic)
        | (Ideographic, Alphabetic)
        | (Alphabetic, Ideographic)
        | (Ideographic, Digit)
        | (Digit, Ideographic)
        | (IdeographicClose, Ideographic)
        | (Ideographic, IdeographicOpen) => true,

        _ => false,
    }
}

/// The byte offsets in `text` at which a line may be broken.
///
/// An offset is the index of the character *after* the break, so it is a
/// position at which the text can be cut in two. Zero is never returned — a
/// break at the start of a piece of text is not an opportunity, it is a
/// different line — and neither is `text.len()`, because whether a line ends
/// after the last character is the caller's decision.
pub fn opportunities(text: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut previous: Option<(Class, usize)> = None;
    for (offset, c) in text.char_indices() {
        let current = class(c);
        if let Some((before, _)) = previous {
            if offset > 0 && may_break(before, current) {
                out.push(offset);
            }
        }
        previous = Some((current, offset));
    }
    out
}

/// Whether a line may be broken between two adjacent characters.
///
/// The same table [`opportunities`] walks, asked one boundary at a time. A
/// paragraph is measured in pieces — a run of bold, a stretch of a different
/// script, a field — and those seams are *not* break opportunities: a bold
/// opening quotation mark must not be left hanging at the end of a line merely
/// because the roman text before it ended there.
pub fn may_break_at(before: char, after: char) -> bool {
    may_break(class(before), class(after))
}

/// Whether the character at `offset` is a space that may hang past the margin.
pub fn is_hanging_space(c: char) -> bool {
    matches!(class(c), Class::Space)
}

/// Whether a break at this offset should draw a hyphen.
pub fn breaks_with_hyphen(text: &str, offset: usize) -> bool {
    text[..offset]
        .chars()
        .next_back()
        .is_some_and(|c| class(c) == Class::SoftHyphen)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn breaks(text: &str) -> Vec<&str> {
        let mut pieces = Vec::new();
        let mut start = 0;
        for offset in opportunities(text) {
            pieces.push(&text[start..offset]);
            start = offset;
        }
        pieces.push(&text[start..]);
        pieces
    }

    #[test]
    fn a_break_comes_after_the_space_rather_than_before_it() {
        // The rule that decides whether justification is right. A line ending
        // "the cat " breaks after the space and the space hangs; breaking before
        // it would leave every justified line one word short.
        assert_eq!(breaks("the cat sat"), ["the ", "cat ", "sat"]);
    }

    #[test]
    fn runs_of_spaces_stay_with_the_word_before_them() {
        assert_eq!(breaks("two  spaces"), ["two  ", "spaces"]);
        assert_eq!(breaks("trailing   "), ["trailing   "]);
    }

    #[test]
    fn a_non_breaking_space_is_not_a_break() {
        assert_eq!(breaks("Mr\u{00A0}Smith"), ["Mr\u{00A0}Smith"]);
        assert_eq!(breaks("10\u{00A0}km away"), ["10\u{00A0}km ", "away"]);
    }

    #[test]
    fn a_hyphen_breaks_after_itself_and_a_number_range_does_not() {
        assert_eq!(breaks("well-known"), ["well-", "known"]);
        assert_eq!(breaks("2019-2024"), ["2019-2024"]);
        // U+2011 is the non-breaking hyphen and exists precisely for this.
        assert_eq!(breaks("well\u{2011}known"), ["well\u{2011}known"]);
    }

    #[test]
    fn punctuation_stays_with_the_word_it_belongs_to() {
        assert_eq!(breaks("end. Next"), ["end. ", "Next"]);
        assert_eq!(breaks("(aside) after"), ["(aside) ", "after"]);
        assert_eq!(breaks("one, two"), ["one, ", "two"]);
    }

    #[test]
    fn a_number_is_not_broken_at_its_separators() {
        assert_eq!(breaks("1,234.56"), ["1,234.56"]);
    }

    #[test]
    fn a_url_may_break_after_a_slash() {
        assert_eq!(
            breaks("https://example.com/path"),
            ["https:/", "/", "example.com/", "path"]
        );
    }

    #[test]
    fn east_asian_text_breaks_between_characters_because_it_has_no_spaces() {
        // A paragraph of Chinese with no break opportunity would be one line as
        // wide as the paragraph, running off the page.
        let pieces = breaks("日本語のテキスト");
        assert!(pieces.len() > 4, "{pieces:?}");
        // But not before a closing mark.
        assert_eq!(breaks("終わり。"), ["終", "わ", "り。"]);
        // And not after an opening bracket.
        assert_eq!(breaks("「引用"), ["「引", "用"]);
    }

    #[test]
    fn a_zero_width_space_is_a_break_and_a_word_joiner_forbids_one() {
        assert_eq!(breaks("long\u{200B}word"), ["long\u{200B}", "word"]);
        // A word joiner forbids a break beside *itself*, so it has to go where
        // the break would otherwise be — after the hyphen, not before it.
        assert_eq!(breaks("no-break"), ["no-", "break"]);
        assert_eq!(breaks("no-\u{2060}break"), ["no-\u{2060}break"]);
    }

    #[test]
    fn a_soft_hyphen_breaks_and_says_it_should_draw_a_hyphen() {
        let text = "sepa\u{00AD}rate";
        let at = opportunities(text);
        assert_eq!(at.len(), 1);
        assert!(breaks_with_hyphen(text, at[0]));
        assert!(!breaks_with_hyphen("plain text", 6));
    }

    #[test]
    fn nothing_breaks_at_the_very_start() {
        assert!(opportunities(" leading").iter().all(|&at| at > 0));
        assert!(opportunities("").is_empty());
        assert!(opportunities("x").is_empty());
    }
}
