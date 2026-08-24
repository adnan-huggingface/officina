//! Numbering: bullets, numbered lists, and the nine levels of both.
//!
//! A numbered paragraph does not store its number. It stores `<w:numPr>` — a
//! `numId` and a level — and the number itself is the result of walking every
//! paragraph before it. That is why inserting a paragraph renumbers a document,
//! and why nothing here can be answered per paragraph in isolation.
//!
//! There are two tables, not one, and the indirection matters. `<w:num>` is an
//! *instance* of a list; `<w:abstractNum>` is its definition. Two lists that
//! look identical and count separately are two instances of one definition —
//! which is exactly what Word creates when the user clicks the bullet button
//! twice — so a reader that collapses them makes the second list continue the
//! first.
//!
//! ```text
//! <w:numPr><w:numId w:val="3"/><w:ilvl w:val="1"/></w:numPr>
//!    -> <w:num w:numId="3">  -> abstractNumId 2, plus any <w:lvlOverride>
//!       -> <w:abstractNum w:abstractNumId="2"> -> <w:lvl w:ilvl="1">
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use crate::prop::{Justify, ParaProps, RunProps};
use crate::style::Layers;

/// Levels per list. Fixed by the format, not by us.
pub const LEVELS: usize = 9;

/// `<w:numFmt>` — how a level's counter is spelled.
///
/// The dozen that Word's own list gallery offers are named; the ninety or so
/// others — Hebrew, Thai, four kinds of Japanese, `chineseCountingThousand` —
/// are kept as [`NumFormat::Other`] with their name, so the writer restores them
/// exactly and the renderer falls back to decimal rather than to nothing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum NumFormat {
    #[default]
    Decimal,
    /// `01, 02, …` — padded to the width of the largest number so far, which is
    /// two in practice.
    DecimalZero,
    UpperRoman,
    LowerRoman,
    UpperLetter,
    LowerLetter,
    /// `1st, 2nd, 3rd`.
    Ordinal,
    /// `One, Two, Three`.
    CardinalText,
    /// `First, Second, Third`.
    OrdinalText,
    /// `①②③`.
    DecimalEnclosedCircle,
    /// `*, †, ‡, §` and then doubles of each — the footnote sequence.
    Chicago,
    /// A bullet. The glyph is in `lvlText`, not here, and the font that draws it
    /// is in the level's `rPr` — usually Symbol or Wingdings, and the character
    /// is meaningless in any other face.
    Bullet,
    /// The level is numbered but nothing is drawn for it. Still counts.
    None,
    Other(Arc<str>),
}

impl NumFormat {
    pub fn from_val(text: &str) -> NumFormat {
        match text {
            "decimal" => NumFormat::Decimal,
            "decimalZero" => NumFormat::DecimalZero,
            "upperRoman" => NumFormat::UpperRoman,
            "lowerRoman" => NumFormat::LowerRoman,
            "upperLetter" => NumFormat::UpperLetter,
            "lowerLetter" => NumFormat::LowerLetter,
            "ordinal" => NumFormat::Ordinal,
            "cardinalText" => NumFormat::CardinalText,
            "ordinalText" => NumFormat::OrdinalText,
            "decimalEnclosedCircle" => NumFormat::DecimalEnclosedCircle,
            "chicago" => NumFormat::Chicago,
            "bullet" => NumFormat::Bullet,
            "none" => NumFormat::None,
            other => NumFormat::Other(other.into()),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            NumFormat::Decimal => "decimal",
            NumFormat::DecimalZero => "decimalZero",
            NumFormat::UpperRoman => "upperRoman",
            NumFormat::LowerRoman => "lowerRoman",
            NumFormat::UpperLetter => "upperLetter",
            NumFormat::LowerLetter => "lowerLetter",
            NumFormat::Ordinal => "ordinal",
            NumFormat::CardinalText => "cardinalText",
            NumFormat::OrdinalText => "ordinalText",
            NumFormat::DecimalEnclosedCircle => "decimalEnclosedCircle",
            NumFormat::Chicago => "chicago",
            NumFormat::Bullet => "bullet",
            NumFormat::None => "none",
            NumFormat::Other(name) => name,
        }
    }

    /// Whether this level draws a counter at all. A bullet level's `%n` is
    /// never substituted — the bullet glyph is literal text.
    pub fn counts(&self) -> bool {
        !matches!(self, NumFormat::Bullet | NumFormat::None)
    }

    /// Spells `n` in this format.
    ///
    /// Out-of-range values fall back to decimal rather than producing nothing: a
    /// list that reaches 4000 in Roman numerals is unusual, and a missing number
    /// is worse than an ugly one.
    pub fn spell(&self, n: i32) -> String {
        if n < 0 {
            return n.to_string();
        }
        match self {
            NumFormat::Decimal | NumFormat::None | NumFormat::Bullet => n.to_string(),
            NumFormat::DecimalZero => format!("{n:02}"),
            NumFormat::UpperRoman => roman(n).unwrap_or_else(|| n.to_string()),
            NumFormat::LowerRoman => roman(n)
                .map(|s| s.to_lowercase())
                .unwrap_or_else(|| n.to_string()),
            NumFormat::UpperLetter => letters(n, 'A').unwrap_or_else(|| n.to_string()),
            NumFormat::LowerLetter => letters(n, 'a').unwrap_or_else(|| n.to_string()),
            NumFormat::Ordinal => ordinal(n),
            NumFormat::CardinalText => spell_cardinal(n).unwrap_or_else(|| n.to_string()),
            NumFormat::OrdinalText => spell_ordinal(n).unwrap_or_else(|| ordinal(n)),
            NumFormat::DecimalEnclosedCircle => enclosed_circle(n).unwrap_or_else(|| n.to_string()),
            NumFormat::Chicago => chicago(n),
            NumFormat::Other(_) => n.to_string(),
        }
    }
}

/// `1 -> I`, `4 -> IV`, `1990 -> MCMXC`. `None` outside 1..=3999, which is the
/// whole of what Roman numerals can say.
fn roman(n: i32) -> Option<String> {
    if !(1..=3999).contains(&n) {
        return None;
    }
    const TABLE: [(i32, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut left = n;
    let mut out = String::new();
    for (value, glyph) in TABLE {
        while left >= value {
            out.push_str(glyph);
            left -= value;
        }
    }
    Some(out)
}

/// `1 -> a`, `26 -> z`, `27 -> aa`, `53 -> aaa`.
///
/// **Not** bijective base-26, which would make 27 `aa` and 28 `ab`. Word repeats
/// one letter, so 28 is `bb`. Getting this wrong is invisible until a list runs
/// past twenty-six items, and then every label after it is wrong.
fn letters(n: i32, first: char) -> Option<String> {
    if n < 1 {
        return None;
    }
    let index = (n - 1) % 26;
    let repeat = ((n - 1) / 26) + 1;
    let letter = char::from_u32(first as u32 + index as u32)?;
    Some(std::iter::repeat_n(letter, repeat as usize).collect())
}

/// `1 -> 1st`. English, which is what the format's own name commits to.
fn ordinal(n: i32) -> String {
    let suffix = match (n % 100, n % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

const ONES: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];

const TENS: [&str; 10] = [
    "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];

/// `1 -> One`. Capitalised as Word writes it, and `None` past 9999 — a list that
/// long spelled out in words is not a thing this has to be right about.
fn spell_cardinal(n: i32) -> Option<String> {
    let words = cardinal_words(n)?;
    Some(capitalise(&words))
}

fn cardinal_words(n: i32) -> Option<String> {
    if !(0..=9999).contains(&n) {
        return None;
    }
    if n < 20 {
        return Some(ONES[n as usize].to_string());
    }
    if n < 100 {
        let tens = TENS[(n / 10) as usize];
        return Some(match n % 10 {
            0 => tens.to_string(),
            rest => format!("{tens}-{}", ONES[rest as usize]),
        });
    }
    if n < 1000 {
        let hundreds = format!("{} hundred", ONES[(n / 100) as usize]);
        return Some(match n % 100 {
            0 => hundreds,
            rest => format!("{hundreds} {}", cardinal_words(rest)?),
        });
    }
    let thousands = format!("{} thousand", cardinal_words(n / 1000)?);
    Some(match n % 1000 {
        0 => thousands,
        rest => format!("{thousands} {}", cardinal_words(rest)?),
    })
}

/// `1 -> First`.
fn spell_ordinal(n: i32) -> Option<String> {
    let words = cardinal_words(n)?;
    let (head, last) = match words.rsplit_once([' ', '-']) {
        Some((head, last)) => (Some((head, &words[head.len()..head.len() + 1])), last),
        None => (None, words.as_str()),
    };
    let ordinal_last = match last {
        "one" => "first".to_string(),
        "two" => "second".to_string(),
        "three" => "third".to_string(),
        "five" => "fifth".to_string(),
        "eight" => "eighth".to_string(),
        "nine" => "ninth".to_string(),
        "twelve" => "twelfth".to_string(),
        other if other.ends_with('y') => format!("{}ieth", &other[..other.len() - 1]),
        other => format!("{other}th"),
    };
    let full = match head {
        Some((head, separator)) => format!("{head}{separator}{ordinal_last}"),
        None => ordinal_last,
    };
    Some(capitalise(&full))
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `①` through `⑳`, and then nothing — Unicode's circled digits stop at 20 and
/// so does Word's rendering of them.
fn enclosed_circle(n: i32) -> Option<String> {
    (1..=20)
        .contains(&n)
        .then(|| char::from_u32(0x2460 + (n as u32 - 1)))
        .flatten()
        .map(String::from)
}

/// The footnote sequence: `*`, `†`, `‡`, `§`, then each doubled, then tripled.
fn chicago(n: i32) -> String {
    const MARKS: [char; 4] = ['*', '†', '‡', '§'];
    if n < 1 {
        return n.to_string();
    }
    let index = ((n - 1) % 4) as usize;
    let repeat = ((n - 1) / 4 + 1) as usize;
    std::iter::repeat_n(MARKS[index], repeat).collect()
}

/// What follows the number before the paragraph's text begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Suffix {
    /// A tab, which lands on the level's own indent. The default, and what makes
    /// a list's text line up.
    #[default]
    Tab,
    Space,
    Nothing,
}

impl Suffix {
    pub fn from_val(text: &str) -> Option<Suffix> {
        Some(match text {
            "tab" => Suffix::Tab,
            "space" => Suffix::Space,
            "nothing" => Suffix::Nothing,
            _ => return None,
        })
    }
}

/// One `<w:lvl>`.
#[derive(Debug, Clone)]
pub struct Level {
    pub index: u8,
    /// `<w:start>` — the first number, which is 1 for nearly everything and 0
    /// for a list the author restarted from zero.
    pub start: i32,
    pub format: NumFormat,
    /// `<w:lvlText>` — the label, with `%1` through `%9` standing for the
    /// counters of levels one through nine. `%1.%2.` is `1.1.`, and a bullet
    /// level's text is the bullet glyph with no placeholder in it at all.
    pub text: Arc<str>,
    pub justify: Justify,
    /// `<w:lvlRestart>` — the one-based level whose increment restarts this one.
    /// **Zero means never**, which is how a numbering that runs continuously
    /// through a document's sections is written. Absent means the default: any
    /// shallower level restarts it.
    pub restart: Option<u8>,
    pub suffix: Suffix,
    /// `<w:isLgl>` — draw every level of the label in decimal however the
    /// deeper levels are formatted, so `IV.b` becomes `4.2`. It changes the
    /// *label*, not the counters.
    pub legal: bool,
    /// The indent, tabs and justification a paragraph at this level takes.
    pub para: ParaProps,
    /// The formatting of the number or bullet itself — not of the paragraph's
    /// text. A bullet's font lives here, and it is the reason a Wingdings
    /// character does not turn the whole paragraph into Wingdings.
    pub run: RunProps,
    /// `<w:lvlPicBulletId>` — an image used as the bullet, by id into the
    /// numbering part's own `<w:numPicBullet>` list.
    pub picture_bullet: Option<u32>,
}

impl Level {
    pub fn new(index: u8) -> Level {
        Level {
            index,
            start: 1,
            format: NumFormat::Decimal,
            text: format!("%{}.", index + 1).into(),
            justify: Justify::Start,
            restart: None,
            suffix: Suffix::Tab,
            legal: false,
            para: ParaProps::default(),
            run: RunProps::default(),
            picture_bullet: None,
        }
    }

    /// The formatting layers a paragraph at this level inherits.
    pub fn layers(&self) -> Layers {
        Layers {
            para: self.para.clone(),
            run: RunProps::default(),
        }
    }
}

/// How many levels a definition actually uses. Word writes all nine regardless;
/// this is a hint for its own UI and is preserved rather than trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MultiLevel {
    Single,
    #[default]
    Multi,
    /// `hybridMultilevel` — what Word writes for a list the user built by
    /// clicking buttons, meaning "the levels do not share a scheme".
    Hybrid,
}

impl MultiLevel {
    pub fn from_val(text: &str) -> Option<MultiLevel> {
        Some(match text {
            "singleLevel" => MultiLevel::Single,
            "multilevel" => MultiLevel::Multi,
            "hybridMultilevel" => MultiLevel::Hybrid,
            _ => return None,
        })
    }
}

/// `<w:abstractNum>` — a list *definition*.
#[derive(Debug, Clone)]
pub struct AbstractNum {
    pub id: u32,
    /// `<w:nsid>` and `<w:tmpl>` are Word's own identifiers for the gallery
    /// entry a list came from. Kept because two lists sharing an nsid are meant
    /// to look alike.
    pub nsid: Option<Arc<str>>,
    pub template: Option<Arc<str>>,
    pub name: Option<Arc<str>>,
    pub multi_level: MultiLevel,
    /// `<w:numStyleLink>` — this definition is a pointer to a numbering *style*
    /// rather than a definition in its own right. Following it is the caller's
    /// job because it needs the style table.
    pub num_style_link: Option<Arc<str>>,
    /// `<w:styleLink>` — the numbering style this definition backs.
    pub style_link: Option<Arc<str>>,
    pub levels: Vec<Option<Level>>,
}

impl AbstractNum {
    pub fn new(id: u32) -> AbstractNum {
        AbstractNum {
            id,
            nsid: None,
            template: None,
            name: None,
            multi_level: MultiLevel::Multi,
            num_style_link: None,
            style_link: None,
            levels: vec![None; LEVELS],
        }
    }

    pub fn set_level(&mut self, level: Level) {
        let index = level.index as usize;
        if index < LEVELS {
            self.levels[index] = Some(level);
        }
    }
}

/// `<w:lvlOverride>` — one instance's departure from its definition.
#[derive(Debug, Clone)]
pub struct LevelOverride {
    pub index: u8,
    /// `<w:startOverride>` — restart at this number. Present far more often than
    /// a replacement level, because it is what "Restart numbering at 1" writes.
    pub start: Option<i32>,
    /// A whole replacement `<w:lvl>`.
    pub level: Option<Box<Level>>,
}

/// `<w:num>` — an *instance* of a definition, and what `<w:numId>` names.
#[derive(Debug, Clone)]
pub struct Num {
    pub id: u32,
    pub abstract_id: u32,
    pub overrides: Vec<LevelOverride>,
}

impl Num {
    pub fn new(id: u32, abstract_id: u32) -> Num {
        Num {
            id,
            abstract_id,
            overrides: Vec::new(),
        }
    }

    fn override_for(&self, level: u8) -> Option<&LevelOverride> {
        self.overrides.iter().find(|o| o.index == level)
    }
}

/// `<w:numPicBullet>` — a picture drawn in place of a bullet glyph.
///
/// The level points at one of these by id, and Word draws the image at the
/// size the shape states rather than at the image's own. A reader that only
/// takes the level's `<w:lvlText>` draws the character Word left there as a
/// fallback, which is a Symbol dot where the author put a picture.
#[derive(Debug, Clone, PartialEq)]
pub struct PictureBullet {
    /// The relationship naming the image, *qualified with the part it belongs
    /// to* — `numbering:rId1`.
    ///
    /// `numbering.xml` numbers its relationships from `rId1` exactly as
    /// `document.xml` does, and they are different relationships of different
    /// parts: a bullet asking for a bare `rId1` would be handed whatever the
    /// document's first relationship points at. This is a handle for fetching
    /// the bytes and nothing else — the numbering part is written back byte
    /// for byte, so it never travels the other way.
    pub rel: Arc<str>,
    /// What the shape says to draw it at, in points. Word obeys this and not
    /// the image's natural size, which for the icons Word ships is many times
    /// larger than the line it sits on.
    pub width: f64,
    pub height: f64,
}

/// The whole of `numbering.xml`.
#[derive(Debug, Clone, Default)]
pub struct Numbering {
    abstracts: HashMap<u32, AbstractNum>,
    instances: HashMap<u32, Num>,
    picture_bullets: HashMap<u32, PictureBullet>,
}

impl Numbering {
    pub fn new() -> Numbering {
        Numbering::default()
    }

    pub fn insert_abstract(&mut self, definition: AbstractNum) {
        self.abstracts.insert(definition.id, definition);
    }

    pub fn insert_num(&mut self, instance: Num) {
        self.instances.insert(instance.id, instance);
    }

    pub fn insert_picture_bullet(&mut self, id: u32, bullet: PictureBullet) {
        self.picture_bullets.insert(id, bullet);
    }

    /// The picture a level draws instead of a bullet, if it names one and the
    /// part actually holds it. A level may point at an id that is not there —
    /// an edit that removed the picture and left the reference — and then the
    /// bullet is the character the level states.
    pub fn picture_bullet(&self, id: u32) -> Option<&PictureBullet> {
        self.picture_bullets.get(&id)
    }

    pub fn abstract_num(&self, id: u32) -> Option<&AbstractNum> {
        self.abstracts.get(&id)
    }

    pub fn num(&self, id: u32) -> Option<&Num> {
        self.instances.get(&id)
    }

    pub fn is_empty(&self) -> bool {
        self.abstracts.is_empty() && self.instances.is_empty()
    }

    pub fn nums(&self) -> impl Iterator<Item = &Num> {
        self.instances.values()
    }

    pub fn abstracts(&self) -> impl Iterator<Item = &AbstractNum> {
        self.abstracts.values()
    }

    /// The lowest ids not yet taken, for an author making a new definition:
    /// abstract first, instance second. Word numbers both from 1 upward and
    /// never reuses a freed id within a document's life; neither does this.
    pub fn free_ids(&self) -> (u32, u32) {
        let next = |taken: &mut dyn Iterator<Item = u32>| taken.max().unwrap_or(0) + 1;
        (
            next(&mut self.abstracts.keys().copied()),
            next(&mut self.instances.keys().copied()),
        )
    }

    /// The level a `<w:numPr>` resolves to, with the instance's override applied.
    ///
    /// An override may replace the level outright or only its start, and the two
    /// are independent: the common case is a start override on a level that is
    /// otherwise the definition's.
    pub fn level(&self, num_id: u32, level: u8) -> Option<&Level> {
        let instance = self.instances.get(&num_id)?;
        if let Some(over) = instance.override_for(level) {
            if let Some(level) = &over.level {
                return Some(level);
            }
        }
        let definition = self.abstracts.get(&instance.abstract_id)?;
        definition.levels.get(level as usize)?.as_ref()
    }

    /// Where this instance's level starts counting, override included.
    pub fn start(&self, num_id: u32, level: u8) -> i32 {
        let override_start = self
            .instances
            .get(&num_id)
            .and_then(|instance| instance.override_for(level))
            .and_then(|over| over.start);
        override_start
            .or_else(|| self.level(num_id, level).map(|level| level.start))
            .unwrap_or(1)
    }

    /// The formatting a paragraph inherits from its list level.
    pub fn layers(&self, num: crate::prop::NumRef) -> Option<Layers> {
        num.is_numbered()
            .then(|| self.level(num.num_id, num.level))
            .flatten()
            .map(Level::layers)
    }
}

/// The running counters of every list in a document.
///
/// A number is a function of every numbered paragraph before it, so this is
/// walked forward once and the labels fall out. Rebuilding it from scratch is
/// cheap enough that a re-layout after an edit does exactly that rather than
/// trying to patch the state in the middle, where an off-by-one is invisible
/// until someone reads the printed page.
#[derive(Debug, Clone, Default)]
pub struct Counters {
    /// Value per (instance, level). Absent means the level has not been reached
    /// yet, which is not the same as being at its start value.
    counts: HashMap<(u32, u8), i32>,
}

impl Counters {
    pub fn new() -> Counters {
        Counters::default()
    }

    pub fn reset(&mut self) {
        self.counts.clear();
    }

    /// Advances the counter for one numbered paragraph and returns its label.
    ///
    /// `None` when the paragraph is not in a list, or when the list it names
    /// does not exist — a `numId` pointing at nothing is a document Word still
    /// opens, and it draws no number.
    pub fn advance(&mut self, numbering: &Numbering, num: crate::prop::NumRef) -> Option<String> {
        if !num.is_numbered() {
            return None;
        }
        let level = numbering.level(num.num_id, num.level)?;
        let key = (num.num_id, num.level);
        let next = match self.counts.get(&key) {
            Some(current) => current + 1,
            None => numbering.start(num.num_id, num.level),
        };
        self.counts.insert(key, next);
        self.restart_deeper(numbering, num.num_id, num.level);
        Some(self.label(numbering, num.num_id, level))
    }

    /// Clears every deeper level that this one restarts.
    ///
    /// The default is that any shallower level restarts a deeper one, which is
    /// what makes 1.1, 1.2, 2.1 rather than 1.1, 1.2, 2.3. `lvlRestart` names a
    /// different trigger, and zero means never — which is how a numbering that
    /// runs continuously through a long document is written.
    fn restart_deeper(&mut self, numbering: &Numbering, num_id: u32, changed: u8) {
        for deeper in changed + 1..LEVELS as u8 {
            let trigger = numbering
                .level(num_id, deeper)
                .and_then(|level| level.restart);
            let restarts = match trigger {
                Some(0) => false,
                // The attribute is one-based and names the level that resets
                // this one, so it is a trigger *index* of one less.
                Some(one_based) => changed <= one_based.saturating_sub(1),
                None => true,
            };
            if restarts {
                self.counts.remove(&(num_id, deeper));
            }
        }
    }

    /// The current value of a level, whether or not it has been reached.
    fn value(&self, numbering: &Numbering, num_id: u32, level: u8) -> i32 {
        self.counts
            .get(&(num_id, level))
            .copied()
            .unwrap_or_else(|| numbering.start(num_id, level))
    }

    /// Substitutes `%1`..`%9` in a level's text with the counters they name.
    ///
    /// A bullet level has no placeholders and its text is the glyph, so this
    /// returns it unchanged — which is why the same function serves both.
    fn label(&self, numbering: &Numbering, num_id: u32, level: &Level) -> String {
        let mut out = String::with_capacity(level.text.len());
        let mut chars = level.text.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            match chars.peek().and_then(|d| d.to_digit(10)) {
                Some(one_based @ 1..=9) => {
                    chars.next();
                    let referenced = (one_based - 1) as u8;
                    let value = self.value(numbering, num_id, referenced);
                    // `isLgl` draws every level of the label in decimal, however
                    // the level it names is formatted.
                    let format = if level.legal {
                        NumFormat::Decimal
                    } else {
                        numbering
                            .level(num_id, referenced)
                            .map(|l| l.format.clone())
                            .unwrap_or_default()
                    };
                    out.push_str(&format.spell(value));
                }
                // A literal percent, or `%0`, which is not a placeholder.
                _ => out.push('%'),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prop::NumRef;

    fn decimal_list() -> Numbering {
        let mut numbering = Numbering::new();
        let mut definition = AbstractNum::new(0);
        for index in 0..3u8 {
            let mut level = Level::new(index);
            level.text = match index {
                0 => "%1.".into(),
                1 => "%1.%2.".into(),
                _ => "%1.%2.%3.".into(),
            };
            definition.set_level(level);
        }
        numbering.insert_abstract(definition);
        numbering.insert_num(Num::new(1, 0));
        numbering
    }

    fn at(num_id: u32, level: u8) -> NumRef {
        NumRef { num_id, level }
    }

    #[test]
    fn two_instances_of_one_definition_count_separately() {
        // Clicking the bullet button twice makes two lists, not one longer one.
        let mut numbering = decimal_list();
        numbering.insert_num(Num::new(2, 0));
        let mut counters = Counters::new();
        assert_eq!(
            counters.advance(&numbering, at(1, 0)).as_deref(),
            Some("1.")
        );
        assert_eq!(
            counters.advance(&numbering, at(1, 0)).as_deref(),
            Some("2.")
        );
        assert_eq!(
            counters.advance(&numbering, at(2, 0)).as_deref(),
            Some("1.")
        );
        assert_eq!(
            counters.advance(&numbering, at(1, 0)).as_deref(),
            Some("3.")
        );
    }

    #[test]
    fn a_shallower_level_restarts_the_deeper_ones() {
        let numbering = decimal_list();
        let mut counters = Counters::new();
        assert_eq!(
            counters.advance(&numbering, at(1, 0)).as_deref(),
            Some("1.")
        );
        assert_eq!(
            counters.advance(&numbering, at(1, 1)).as_deref(),
            Some("1.1.")
        );
        assert_eq!(
            counters.advance(&numbering, at(1, 1)).as_deref(),
            Some("1.2.")
        );
        assert_eq!(
            counters.advance(&numbering, at(1, 0)).as_deref(),
            Some("2.")
        );
        // 2.1, not 2.3 — the whole point of the restart rule.
        assert_eq!(
            counters.advance(&numbering, at(1, 1)).as_deref(),
            Some("2.1.")
        );
    }

    #[test]
    fn lvl_restart_zero_runs_the_level_continuously() {
        let mut numbering = decimal_list();
        let mut definition = AbstractNum::new(0);
        let mut top = Level::new(0);
        top.text = "%1.".into();
        definition.set_level(top);
        let mut second = Level::new(1);
        second.text = "%2.".into();
        second.restart = Some(0);
        definition.set_level(second);
        numbering.insert_abstract(definition);

        let mut counters = Counters::new();
        counters.advance(&numbering, at(1, 0));
        assert_eq!(
            counters.advance(&numbering, at(1, 1)).as_deref(),
            Some("1.")
        );
        counters.advance(&numbering, at(1, 0));
        assert_eq!(
            counters.advance(&numbering, at(1, 1)).as_deref(),
            Some("2."),
            "a level that never restarts keeps counting through the one above it"
        );
    }

    #[test]
    fn a_start_override_is_how_restart_numbering_is_written() {
        let mut numbering = decimal_list();
        let mut instance = Num::new(2, 0);
        instance.overrides.push(LevelOverride {
            index: 0,
            start: Some(5),
            level: None,
        });
        numbering.insert_num(instance);

        let mut counters = Counters::new();
        assert_eq!(
            counters.advance(&numbering, at(2, 0)).as_deref(),
            Some("5.")
        );
        assert_eq!(
            counters.advance(&numbering, at(2, 0)).as_deref(),
            Some("6.")
        );
        // The definition it overrides is untouched.
        assert_eq!(
            counters.advance(&numbering, at(1, 0)).as_deref(),
            Some("1.")
        );
    }

    #[test]
    fn a_deeper_level_shows_its_parents_current_number() {
        // 2.1 has to know that level one is at 2, which is state and not a
        // property of the paragraph being numbered.
        let numbering = decimal_list();
        let mut counters = Counters::new();
        for _ in 0..3 {
            counters.advance(&numbering, at(1, 0));
        }
        assert_eq!(
            counters.advance(&numbering, at(1, 2)).as_deref(),
            Some("3.1.1.")
        );
    }

    #[test]
    fn is_lgl_draws_every_level_in_decimal() {
        let mut numbering = Numbering::new();
        let mut definition = AbstractNum::new(0);
        let mut top = Level::new(0);
        top.format = NumFormat::UpperRoman;
        top.text = "%1.".into();
        definition.set_level(top);
        let mut second = Level::new(1);
        second.format = NumFormat::LowerLetter;
        second.text = "%1.%2".into();
        second.legal = true;
        definition.set_level(second);
        numbering.insert_abstract(definition);
        numbering.insert_num(Num::new(1, 0));

        let mut counters = Counters::new();
        for _ in 0..4 {
            counters.advance(&numbering, at(1, 0));
        }
        assert_eq!(
            counters.advance(&numbering, at(1, 1)).as_deref(),
            Some("4.1")
        );
    }

    #[test]
    fn a_bullet_level_has_no_placeholder_and_counts_anyway() {
        let mut numbering = Numbering::new();
        let mut definition = AbstractNum::new(0);
        let mut level = Level::new(0);
        level.format = NumFormat::Bullet;
        level.text = "\u{F0B7}".into();
        definition.set_level(level);
        numbering.insert_abstract(definition);
        numbering.insert_num(Num::new(1, 0));

        let mut counters = Counters::new();
        assert_eq!(
            counters.advance(&numbering, at(1, 0)).as_deref(),
            Some("\u{F0B7}")
        );
        assert_eq!(
            counters.advance(&numbering, at(1, 0)).as_deref(),
            Some("\u{F0B7}")
        );
    }

    #[test]
    fn a_num_id_pointing_at_nothing_draws_no_number() {
        let numbering = decimal_list();
        let mut counters = Counters::new();
        assert_eq!(counters.advance(&numbering, at(99, 0)), None);
        assert_eq!(counters.advance(&numbering, at(0, 0)), None);
    }

    #[test]
    fn letters_repeat_rather_than_carrying() {
        // Word writes aa for 27 and bb for 28. Bijective base-26 would give ab,
        // and a list that runs past twenty-six is where that shows.
        assert_eq!(NumFormat::LowerLetter.spell(1), "a");
        assert_eq!(NumFormat::LowerLetter.spell(26), "z");
        assert_eq!(NumFormat::LowerLetter.spell(27), "aa");
        assert_eq!(NumFormat::LowerLetter.spell(28), "bb");
        assert_eq!(NumFormat::LowerLetter.spell(53), "aaa");
        assert_eq!(NumFormat::UpperLetter.spell(27), "AA");
    }

    #[test]
    fn the_number_formats_spell_what_they_say_they_do() {
        assert_eq!(NumFormat::UpperRoman.spell(1990), "MCMXC");
        assert_eq!(NumFormat::LowerRoman.spell(4), "iv");
        assert_eq!(NumFormat::UpperRoman.spell(4000), "4000", "out of range");
        assert_eq!(NumFormat::DecimalZero.spell(7), "07");
        assert_eq!(NumFormat::Ordinal.spell(1), "1st");
        assert_eq!(NumFormat::Ordinal.spell(11), "11th");
        assert_eq!(NumFormat::Ordinal.spell(22), "22nd");
        assert_eq!(NumFormat::Ordinal.spell(113), "113th");
        assert_eq!(NumFormat::CardinalText.spell(21), "Twenty-one");
        assert_eq!(NumFormat::CardinalText.spell(105), "One hundred five");
        assert_eq!(NumFormat::OrdinalText.spell(1), "First");
        assert_eq!(NumFormat::OrdinalText.spell(2), "Second");
        assert_eq!(NumFormat::OrdinalText.spell(12), "Twelfth");
        assert_eq!(NumFormat::OrdinalText.spell(20), "Twentieth");
        assert_eq!(NumFormat::OrdinalText.spell(21), "Twenty-first");
        assert_eq!(NumFormat::DecimalEnclosedCircle.spell(3), "③");
        assert_eq!(NumFormat::Chicago.spell(1), "*");
        assert_eq!(NumFormat::Chicago.spell(5), "**");
    }

    #[test]
    fn an_unknown_format_keeps_its_name_and_draws_a_number() {
        let format = NumFormat::from_val("chineseCountingThousand");
        assert_eq!(format.name(), "chineseCountingThousand");
        assert_eq!(format.spell(3), "3");
    }

    #[test]
    fn a_replacement_level_beats_the_definition() {
        let mut numbering = decimal_list();
        let mut instance = Num::new(3, 0);
        let mut replacement = Level::new(0);
        replacement.text = "Item %1:".into();
        instance.overrides.push(LevelOverride {
            index: 0,
            start: None,
            level: Some(Box::new(replacement)),
        });
        numbering.insert_num(instance);

        let mut counters = Counters::new();
        assert_eq!(
            counters.advance(&numbering, at(3, 0)).as_deref(),
            Some("Item 1:")
        );
    }
}
