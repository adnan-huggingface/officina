//! Number formats: turning a stored value into the text a cell shows.
//!
//! A spreadsheet cell holds a number and displays a string, and the gap between
//! them is entirely this module. `45352` is a number until a format code says it
//! is 1 March 2024; `0.5` is a number until a format says it is 50% or 12:00 or
//! `1/2`. Nothing else in the model knows the difference, which is deliberate —
//! the value survives every reformat.
//!
//! The code language is Excel's, and it is stranger than it looks:
//!
//! * A format has up to four sections — positive, negative, zero, text —
//!   separated by semicolons, and *which* section applies depends on how many
//!   there are. Two sections split at zero; three split into three.
//! * `m` means month or minute depending on what is next to it. `mm/dd` is a
//!   date and `hh:mm` is a time, with the same token meaning different things.
//! * A comma is a thousands separator between digits and a division by a
//!   thousand after them, so `#,##0,` shows 1,234,567 as `1,235`.
//!
//! None of that is derivable. Every rule below is Excel's observed behaviour.

use std::fmt::Write as _;

use crate::datetime;

/// How many significant digits Excel keeps when a number becomes text.
const SIGNIFICANT_DIGITS: usize = 15;

/// Renders a number the way Excel's General format does.
///
/// Fifteen significant digits, then trailing zeros dropped. This is why
/// `=0.1+0.2&""` is `"0.3"` and not `"0.30000000000000004"`: the sixteenth digit,
/// where the binary representation shows through, is never printed.
pub fn format_general(x: f64) -> String {
    if x == 0.0 {
        // Also catches -0.0, which Excel shows as 0.
        return "0".to_string();
    }
    if !x.is_finite() {
        // Unreachable from evaluation — every operation that would produce one
        // yields an error value instead — but a panic here would be worse.
        return crate::CellError::Num.as_str().to_string();
    }

    // `{:e}` rounds for us and fixes up the exponent if rounding carries
    // (9.99..e0 becomes 1e1), which hand-rolled scaling gets wrong.
    let sci = format!("{:.*e}", SIGNIFICANT_DIGITS - 1, x);
    let (mantissa, exp) = sci
        .split_once('e')
        .expect("`{:e}` always emits an exponent");
    let exp: i32 = exp.parse().expect("`{:e}` emits a decimal exponent");

    let negative = mantissa.starts_with('-');
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };

    let sign = if negative { "-" } else { "" };

    // Outside this band a plain decimal would be mostly padding zeros, and Excel
    // switches to scientific notation.
    if exp >= SIGNIFICANT_DIGITS as i32 || exp <= -5 {
        let mut out = String::from(sign);
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        // Excel always writes at least two exponent digits: `1E-08`, not `1E-8`.
        out.push_str(if exp < 0 { "E-" } else { "E+" });
        let _ = write!(out, "{:02}", exp.abs());
        return out;
    }

    let point = exp + 1; // digits that belong before the decimal point
    let mut out = String::from(sign);
    if point <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-point) as usize));
        out.push_str(digits);
    } else if point as usize >= digits.len() {
        out.push_str(digits);
        out.extend(std::iter::repeat_n('0', point as usize - digits.len()));
    } else {
        out.push_str(&digits[..point as usize]);
        out.push('.');
        out.push_str(&digits[point as usize..]);
    }
    out
}

/// The value a format is applied to.
///
/// Deliberately not [`crate::CellValue`]: that carries an interned string id and
/// would drag the string table into every call site.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FormatValue<'a> {
    Blank,
    Number(f64),
    Bool(bool),
    Text(&'a str),
    Error(crate::CellError),
}

/// The result of applying a format.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Formatted {
    pub text: String,
    /// Set by a `[Red]`-style directive in the format code.
    pub color: Option<[u8; 3]>,
    /// True when the cell should be right-aligned in the absence of an explicit
    /// alignment — which is to say, when the value is a number and not text.
    pub numeric: bool,
}

// ------------------------------------------------------------------ parsing

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Condition {
    cmp: Cmp,
    value: f64,
}

impl Condition {
    fn holds(&self, x: f64) -> bool {
        match self.cmp {
            Cmp::Lt => x < self.value,
            Cmp::Le => x <= self.value,
            Cmp::Gt => x > self.value,
            Cmp::Ge => x >= self.value,
            Cmp::Eq => x == self.value,
            Cmp::Ne => x != self.value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    /// `0`, `#`, `?` — a forced, optional, or space-padded digit.
    Digit(u8),
    Point,
    /// A grouping comma between digit placeholders.
    Group,
    Percent,
    Exponent {
        plus: bool,
    },
    Fraction,
    /// `@` — where the cell's text goes.
    Text,
    General,
    Literal(String),
    Year(usize),
    Month(usize),
    Day(usize),
    Hour(usize),
    Minute(usize),
    Second(usize),
    /// `AM/PM`, and whether it was written in lower case.
    Meridiem(bool),
    /// `[h]`, `[mm]`, `[ss]` — a duration rather than a clock reading.
    Elapsed(char, usize),
}

const FORCED: u8 = b'0';
const OPTIONAL: u8 = b'#';
const SPACED: u8 = b'?';

#[derive(Debug, Clone, PartialEq)]
struct Section {
    condition: Option<Condition>,
    color: Option<[u8; 3]>,
    tokens: Vec<Tok>,
}

impl Section {
    fn is_datetime(&self) -> bool {
        self.tokens.iter().any(|t| {
            matches!(
                t,
                Tok::Year(_)
                    | Tok::Month(_)
                    | Tok::Day(_)
                    | Tok::Hour(_)
                    | Tok::Minute(_)
                    | Tok::Second(_)
                    | Tok::Meridiem(_)
                    | Tok::Elapsed(..)
            )
        })
    }

    fn is_general(&self) -> bool {
        self.tokens
            .iter()
            .all(|t| matches!(t, Tok::General) || matches!(t, Tok::Literal(s) if s.is_empty()))
    }
}

/// A parsed format code.
#[derive(Debug, Clone, PartialEq)]
pub struct NumberFormat {
    sections: Vec<Section>,
}

impl Default for NumberFormat {
    fn default() -> Self {
        NumberFormat::general()
    }
}

impl NumberFormat {
    pub fn general() -> Self {
        NumberFormat {
            sections: vec![Section {
                condition: None,
                color: None,
                tokens: vec![Tok::General],
            }],
        }
    }

    /// Parses a format code. An unparseable code falls back to General rather
    /// than failing — a cell must always display something.
    pub fn parse(code: &str) -> Self {
        let sections: Vec<Section> = split_sections(code)
            .into_iter()
            .map(parse_section)
            .collect();
        if sections.is_empty() {
            return NumberFormat::general();
        }
        NumberFormat { sections }
    }

    /// The format code for one of Excel's built-in ids.
    ///
    /// Ids below 164 are reserved and are *not* written into the file — a cell
    /// with `numFmtId="14"` is a date and the file never says so. Getting this
    /// table wrong shows every date in a document as a five-digit number.
    pub fn builtin(id: u32) -> Option<&'static str> {
        Some(match id {
            0 => "General",
            1 => "0",
            2 => "0.00",
            3 => "#,##0",
            4 => "#,##0.00",
            9 => "0%",
            10 => "0.00%",
            11 => "0.00E+00",
            12 => "# ?/?",
            13 => "# ??/??",
            14 => "mm-dd-yy",
            15 => "d-mmm-yy",
            16 => "d-mmm",
            17 => "mmm-yy",
            18 => "h:mm AM/PM",
            19 => "h:mm:ss AM/PM",
            20 => "h:mm",
            21 => "h:mm:ss",
            22 => "m/d/yy h:mm",
            37 => "#,##0 ;(#,##0)",
            38 => "#,##0 ;[Red](#,##0)",
            39 => "#,##0.00;(#,##0.00)",
            40 => "#,##0.00;[Red](#,##0.00)",
            45 => "mm:ss",
            46 => "[h]:mm:ss",
            47 => "mmss.0",
            48 => "##0.0E+0",
            49 => "@",
            _ => return None,
        })
    }

    /// Applies the format to a value.
    pub fn format(&self, value: FormatValue) -> Formatted {
        match value {
            FormatValue::Blank => Formatted::default(),
            FormatValue::Error(e) => Formatted {
                text: e.as_str().to_string(),
                ..Default::default()
            },
            FormatValue::Bool(b) => Formatted {
                text: if b { "TRUE" } else { "FALSE" }.to_string(),
                ..Default::default()
            },
            FormatValue::Text(s) => self.format_text(s),
            FormatValue::Number(x) => self.format_number(x),
        }
    }

    /// The text section — the fourth — if the format has one.
    fn format_text(&self, s: &str) -> Formatted {
        let section = (self.sections.len() >= 4).then(|| &self.sections[3]);
        let Some(section) = section else {
            return Formatted {
                text: s.to_string(),
                ..Default::default()
            };
        };
        let mut out = String::new();
        for tok in &section.tokens {
            match tok {
                Tok::Text | Tok::General => out.push_str(s),
                Tok::Literal(lit) => out.push_str(lit),
                _ => {}
            }
        }
        Formatted {
            text: out,
            color: section.color,
            numeric: false,
        }
    }

    fn format_number(&self, x: f64) -> Formatted {
        let (section, drop_sign) = self.pick(x);
        let text = if section.is_general() {
            format_general(if drop_sign { x.abs() } else { x })
        } else if section.is_datetime() {
            render_datetime(section, x)
        } else {
            render_number(section, if drop_sign { x.abs() } else { x })
        };
        Formatted {
            text,
            color: section.color,
            numeric: true,
        }
    }

    /// Chooses the section, and says whether it supplies its own minus sign.
    fn pick(&self, x: f64) -> (&Section, bool) {
        // A conditional format tests its sections in order and falls through to
        // the last, which acts as "otherwise".
        if self.sections.iter().any(|s| s.condition.is_some()) {
            for section in &self.sections {
                match section.condition {
                    Some(c) if c.holds(x) => return (section, false),
                    None => return (section, false),
                    _ => {}
                }
            }
            return (self.sections.last().expect("never empty"), false);
        }

        let numeric: Vec<&Section> = self.sections.iter().take(3).collect();
        match numeric.len() {
            0 => unreachable!("parse guarantees at least one section"),
            1 => (numeric[0], false),
            // Two sections split at zero: the second is for negatives and
            // writes its own sign, which is how `(1,234)` works.
            2 => {
                if x < 0.0 {
                    (numeric[1], true)
                } else {
                    (numeric[0], false)
                }
            }
            _ => {
                if x < 0.0 {
                    (numeric[1], true)
                } else if x == 0.0 {
                    (numeric[2], false)
                } else {
                    (numeric[0], false)
                }
            }
        }
    }
}

/// Splits on semicolons that are not inside quotes or brackets.
fn split_sections(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut in_brackets = false;
    let mut chars = code.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                current.push(c);
            }
            '[' if !in_quotes => {
                in_brackets = true;
                current.push(c);
            }
            ']' if !in_quotes => {
                in_brackets = false;
                current.push(c);
            }
            // An escape takes the next character with it, semicolon included.
            '\\' if !in_quotes => {
                current.push(c);
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            ';' if !in_quotes && !in_brackets => {
                out.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    out.push(current);
    out
}

fn parse_section(code: String) -> Section {
    let mut section = Section {
        condition: None,
        color: None,
        tokens: Vec::new(),
    };
    let chars: Vec<char> = code.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        match c {
            '0' | '#' | '?' => {
                section.tokens.push(Tok::Digit(c as u8));
                i += 1;
            }
            '.' => {
                section.tokens.push(Tok::Point);
                i += 1;
            }
            ',' => {
                section.tokens.push(Tok::Group);
                i += 1;
            }
            '%' => {
                section.tokens.push(Tok::Percent);
                i += 1;
            }
            '/' => {
                section.tokens.push(Tok::Fraction);
                i += 1;
            }
            '@' => {
                section.tokens.push(Tok::Text);
                i += 1;
            }
            'E' | 'e' if matches!(chars.get(i + 1), Some('+' | '-')) => {
                section.tokens.push(Tok::Exponent {
                    plus: chars[i + 1] == '+',
                });
                i += 2;
            }
            '"' => {
                let mut lit = String::new();
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    lit.push(chars[i]);
                    i += 1;
                }
                i += 1;
                section.tokens.push(Tok::Literal(lit));
            }
            '\\' => {
                if let Some(next) = chars.get(i + 1) {
                    section.tokens.push(Tok::Literal(next.to_string()));
                }
                i += 2;
            }
            // `_x` reserves the width of `x`. A space is close enough without a
            // font to measure with; C11 can do better.
            '_' => {
                section.tokens.push(Tok::Literal(" ".to_string()));
                i += 2;
            }
            // `*x` repeats `x` to fill the cell. Filling needs a column width,
            // so the character is dropped rather than guessed at.
            '*' => i += 2,
            '[' => {
                let close = chars[i..]
                    .iter()
                    .position(|&c| c == ']')
                    .map_or(chars.len(), |p| i + p);
                let inner: String = chars[i + 1..close.min(chars.len())].iter().collect();
                apply_directive(&mut section, &inner);
                i = close + 1;
            }
            _ => {
                // Two multi-letter codes have to be recognized whole, because
                // their letters are also date codes: `General` starts with a
                // `G` and `AM/PM` is `A`, `M`, `/`, `P`, `M`.
                if let Some(len) = keyword(&chars, i, "general") {
                    section.tokens.push(Tok::General);
                    i += len;
                    continue;
                }
                if let Some(len) = keyword(&chars, i, "am/pm") {
                    section.tokens.push(Tok::Meridiem(chars[i] == 'a'));
                    i += len;
                    continue;
                }
                if let Some(len) = keyword(&chars, i, "a/p") {
                    section.tokens.push(Tok::Meridiem(chars[i] == 'a'));
                    i += len;
                    continue;
                }
                // Otherwise a run of one repeated letter: a date code or a word.
                let start = i;
                while i < chars.len() && chars[i].eq_ignore_ascii_case(&chars[start]) {
                    i += 1;
                }
                let run: String = chars[start..i].iter().collect();
                push_letters(&mut section.tokens, &run);
            }
        }
    }
    resolve_minutes(&mut section.tokens);
    section
}

fn apply_directive(section: &mut Section, inner: &str) {
    // `[h]`, `[mm]`, `[ss]` are elapsed-time codes, not directives.
    let lower = inner.to_ascii_lowercase();
    if !lower.is_empty() && lower.chars().all(|c| matches!(c, 'h' | 'm' | 's')) {
        let unit = lower.chars().next().expect("non-empty");
        if lower.chars().all(|c| c == unit) {
            section.tokens.push(Tok::Elapsed(unit, lower.len()));
            return;
        }
    }
    if let Some(color) = color_named(&lower) {
        section.color = Some(color);
        return;
    }
    if let Some(condition) = parse_condition(inner) {
        section.condition = Some(condition);
        return;
    }
    // `[$€-407]` carries a currency symbol before the dash; `[$-409]` is a bare
    // locale and contributes nothing.
    if let Some(rest) = inner.strip_prefix('$') {
        let symbol = rest.split('-').next().unwrap_or("");
        if !symbol.is_empty() {
            section.tokens.push(Tok::Literal(symbol.to_string()));
        }
    }
}

fn color_named(name: &str) -> Option<[u8; 3]> {
    Some(match name {
        "black" => [0, 0, 0],
        "white" => [255, 255, 255],
        "red" => [255, 0, 0],
        "green" => [0, 128, 0],
        "blue" => [0, 0, 255],
        "yellow" => [255, 255, 0],
        "magenta" => [255, 0, 255],
        "cyan" => [0, 255, 255],
        _ => return None,
    })
}

fn parse_condition(inner: &str) -> Option<Condition> {
    let (cmp, rest) = if let Some(r) = inner.strip_prefix("<=") {
        (Cmp::Le, r)
    } else if let Some(r) = inner.strip_prefix(">=") {
        (Cmp::Ge, r)
    } else if let Some(r) = inner.strip_prefix("<>") {
        (Cmp::Ne, r)
    } else if let Some(r) = inner.strip_prefix('<') {
        (Cmp::Lt, r)
    } else if let Some(r) = inner.strip_prefix('>') {
        (Cmp::Gt, r)
    } else {
        (Cmp::Eq, inner.strip_prefix('=')?)
    };
    rest.trim()
        .parse()
        .ok()
        .map(|value| Condition { cmp, value })
}

/// The length of `word` if it appears at `i`, case-insensitively.
fn keyword(chars: &[char], i: usize, word: &str) -> Option<usize> {
    let n = word.chars().count();
    if chars.len() < i + n {
        return None;
    }
    let found: String = chars[i..i + n].iter().collect();
    found.eq_ignore_ascii_case(word).then_some(n)
}

/// Turns a run of identical letters into a date token or a literal.
fn push_letters(tokens: &mut Vec<Tok>, run: &str) {
    let n = run.len();
    match run.to_ascii_lowercase().chars().next() {
        Some('y') => tokens.push(Tok::Year(n)),
        Some('m') => tokens.push(Tok::Month(n)),
        Some('d') => tokens.push(Tok::Day(n)),
        Some('h') => tokens.push(Tok::Hour(n)),
        Some('s') => tokens.push(Tok::Second(n)),
        _ => tokens.push(Tok::Literal(run.to_string())),
    }
}

/// Decides which `m` runs are minutes rather than months.
///
/// The rule is positional: an `m` is a minute when it sits next to an hour or a
/// second, and a month otherwise. `mm/dd` and `hh:mm` use the same token and
/// mean different things, so this cannot be decided while tokenizing.
fn resolve_minutes(tokens: &mut [Tok]) {
    let significant = |t: &Tok| {
        matches!(
            t,
            Tok::Year(_)
                | Tok::Month(_)
                | Tok::Day(_)
                | Tok::Hour(_)
                | Tok::Minute(_)
                | Tok::Second(_)
                | Tok::Elapsed(..)
        )
    };
    let positions: Vec<usize> = (0..tokens.len())
        .filter(|i| matches!(tokens[*i], Tok::Month(_)))
        .collect();
    for i in positions {
        let before = tokens[..i].iter().rev().find(|t| significant(t));
        let after = tokens[i + 1..].iter().find(|t| significant(t));
        let is_minute = matches!(before, Some(Tok::Hour(_)) | Some(Tok::Elapsed('h', _)))
            || matches!(after, Some(Tok::Second(_)));
        if is_minute {
            if let Tok::Month(n) = tokens[i] {
                tokens[i] = Tok::Minute(n);
            }
        }
    }
}

// --------------------------------------------------------------- rendering

const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

const DAY_NAMES: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

fn render_datetime(section: &Section, serial: f64) -> String {
    // An elapsed-time format measures a duration, so a value that is not a
    // valid date is still meaningful.
    let elapsed = section.tokens.iter().any(|t| matches!(t, Tok::Elapsed(..)));
    let Some(when) = datetime::from_serial(serial) else {
        if elapsed {
            return render_elapsed(section, serial);
        }
        return crate::CellError::Value.as_str().to_string();
    };
    if elapsed {
        return render_elapsed(section, serial);
    }

    // A twelve-hour clock needs to know before it renders the hour.
    let twelve = section.tokens.iter().any(|t| matches!(t, Tok::Meridiem(_)));
    let hour = if twelve {
        match when.hour % 12 {
            0 => 12,
            h => h,
        }
    } else {
        when.hour
    };

    let mut out = String::new();
    for tok in &section.tokens {
        match tok {
            Tok::Year(n) if *n <= 2 => {
                let _ = write!(out, "{:02}", when.year % 100);
            }
            Tok::Year(_) => {
                let _ = write!(out, "{:04}", when.year);
            }
            Tok::Month(1) => {
                let _ = write!(out, "{}", when.month);
            }
            Tok::Month(2) => {
                let _ = write!(out, "{:02}", when.month);
            }
            Tok::Month(3) => out.push_str(&MONTH_NAMES[(when.month as usize - 1) % 12][..3]),
            Tok::Month(4) => out.push_str(MONTH_NAMES[(when.month as usize - 1) % 12]),
            // `mmmmm` is the single-letter month, which exists only so that a
            // chart axis can read J F M A M J.
            Tok::Month(_) => out.push_str(&MONTH_NAMES[(when.month as usize - 1) % 12][..1]),
            Tok::Day(1) => {
                let _ = write!(out, "{}", when.day);
            }
            Tok::Day(2) => {
                let _ = write!(out, "{:02}", when.day);
            }
            Tok::Day(3) => {
                let name = DAY_NAMES[datetime::weekday_from_serial(serial) as usize];
                out.push_str(&name[..3]);
            }
            Tok::Day(_) => {
                out.push_str(DAY_NAMES[datetime::weekday_from_serial(serial) as usize]);
            }
            Tok::Hour(1) => {
                let _ = write!(out, "{hour}");
            }
            Tok::Hour(_) => {
                let _ = write!(out, "{hour:02}");
            }
            Tok::Minute(1) => {
                let _ = write!(out, "{}", when.minute);
            }
            Tok::Minute(_) => {
                let _ = write!(out, "{:02}", when.minute);
            }
            Tok::Second(1) => {
                let _ = write!(out, "{}", when.second);
            }
            Tok::Second(_) => {
                let _ = write!(out, "{:02}", when.second);
            }
            Tok::Meridiem(lower) => {
                let marker = if when.hour < 12 { "AM" } else { "PM" };
                if *lower {
                    out.push_str(&marker.to_ascii_lowercase());
                } else {
                    out.push_str(marker);
                }
            }
            Tok::Literal(lit) => out.push_str(lit),
            Tok::General => out.push_str(&format_general(serial)),
            // Inside a date these punctuate rather than doing arithmetic.
            Tok::Fraction => out.push('/'),
            Tok::Group => out.push(','),
            // Fractional seconds: `ss.00`.
            Tok::Point => out.push('.'),
            Tok::Digit(_) => {}
            _ => {}
        }
    }
    // The fractional-second digits, if the format asked for any.
    let decimals = section
        .tokens
        .iter()
        .skip_while(|t| !matches!(t, Tok::Point))
        .filter(|t| matches!(t, Tok::Digit(_)))
        .count();
    if decimals > 0 {
        let fraction = serial * 86_400.0;
        let frac = fraction - fraction.floor();
        let scaled = (frac * 10f64.powi(decimals as i32)).round() as u64;
        let _ = write!(out, "{scaled:0width$}", width = decimals);
    }
    out
}

fn render_elapsed(section: &Section, serial: f64) -> String {
    let total_seconds = serial * 86_400.0;
    let mut out = String::new();
    for tok in &section.tokens {
        match tok {
            Tok::Elapsed(unit, width) => {
                let value = match unit {
                    'h' => (total_seconds / 3_600.0).floor(),
                    'm' => (total_seconds / 60.0).floor(),
                    _ => total_seconds.floor(),
                };
                let _ = write!(out, "{:0width$}", value as i64, width = width);
            }
            Tok::Minute(n) => {
                let value = (total_seconds / 60.0).floor() as i64 % 60;
                let _ = write!(out, "{:0width$}", value, width = n);
            }
            Tok::Second(n) => {
                let value = total_seconds.floor() as i64 % 60;
                let _ = write!(out, "{:0width$}", value, width = n);
            }
            Tok::Literal(lit) => out.push_str(lit),
            Tok::Fraction => out.push('/'),
            _ => {}
        }
    }
    out
}

/// The shape of a numeric section, worked out once before any digits are laid.
struct Shape {
    int_places: Vec<u8>,
    dec_places: Vec<u8>,
    grouped: bool,
    percent: u32,
    /// Trailing commas before the decimal point, each dividing by a thousand.
    thousands: u32,
    scientific: bool,
    exp_places: usize,
    fraction_den: Option<usize>,
    fraction_num: usize,
}

fn shape_of(section: &Section) -> Shape {
    let mut shape = Shape {
        int_places: Vec::new(),
        dec_places: Vec::new(),
        grouped: false,
        percent: 0,
        thousands: 0,
        scientific: false,
        exp_places: 0,
        fraction_den: None,
        fraction_num: 0,
    };
    let mut past_point = false;
    let mut past_exponent = false;
    let mut past_fraction = false;
    let mut pending_commas = 0u32;
    // Digit placeholders since the last non-digit token. In `# ?/?` this is
    // what tells the numerator apart from the whole-number part.
    let mut run = 0usize;

    for tok in &section.tokens {
        match tok {
            Tok::Digit(kind) => {
                if past_fraction {
                    shape.fraction_den = Some(shape.fraction_den.unwrap_or(0) + 1);
                } else if past_exponent {
                    shape.exp_places += 1;
                } else if past_point {
                    shape.dec_places.push(*kind);
                } else {
                    // A comma seen between digits groups; one seen after the
                    // last digit scales instead. Which it is only becomes clear
                    // when another digit turns up.
                    if pending_commas > 0 {
                        shape.grouped = true;
                        pending_commas = 0;
                    }
                    shape.int_places.push(*kind);
                }
                run += 1;
            }
            Tok::Point => {
                past_point = true;
                run = 0;
            }
            Tok::Group => {
                if past_point {
                    // A comma after the point is neither grouping nor scaling.
                } else {
                    pending_commas += 1;
                }
            }
            Tok::Percent => shape.percent += 1,
            Tok::Exponent { .. } => {
                past_exponent = true;
                shape.scientific = true;
            }
            Tok::Fraction => {
                past_fraction = true;
                // The digits immediately before the slash were the numerator,
                // not part of the whole number.
                shape.fraction_num = run;
                let keep = shape.int_places.len().saturating_sub(run);
                shape.int_places.truncate(keep);
                run = 0;
            }
            _ => run = 0,
        }
    }
    shape.thousands = pending_commas;
    shape
}

fn render_number(section: &Section, x: f64) -> String {
    let shape = shape_of(section);
    let mut value = x;
    for _ in 0..shape.percent {
        value *= 100.0;
    }
    for _ in 0..shape.thousands {
        value /= 1_000.0;
    }

    if shape.scientific {
        return render_scientific(section, value, &shape);
    }
    if let Some(den_places) = shape.fraction_den {
        return render_fraction(section, value, &shape, den_places);
    }

    let decimals = shape.dec_places.len();
    let negative = value < 0.0;
    let magnitude = value.abs();
    let rounded = round_decimal(magnitude, decimals as i32, Rounding::HalfAway);

    let integer = rounded.trunc();
    let mut int_digits = format!("{integer:.0}");
    if int_digits == "0" && shape.int_places.iter().all(|p| *p != FORCED) {
        // `#.##` shows 0.5 as `.5`, with no leading zero at all.
        int_digits.clear();
    }
    let mut dec_digits = if decimals > 0 {
        let frac = rounded - integer;
        let scaled = (frac * 10f64.powi(decimals as i32)).round() as u64;
        format!("{scaled:0width$}", width = decimals)
    } else {
        String::new()
    };
    // A trailing `#` drops a zero it would otherwise print, which is why
    // `#.##` shows 1.5 as `1.5` and not `1.50`.
    while let Some(last) = dec_digits.len().checked_sub(1) {
        if dec_digits.as_bytes()[last] == b'0' && shape.dec_places[last] == OPTIONAL {
            dec_digits.pop();
        } else {
            break;
        }
    }

    let mut out = String::new();
    // A section with no digit placeholders at all — `[<0]"low"` — says what it
    // says, and prefixing a minus to it would be nonsense.
    let lays_digits = !shape.int_places.is_empty() || !shape.dec_places.is_empty();
    if negative && rounded != 0.0 && lays_digits {
        out.push('-');
    }
    emit(&mut out, section, &shape, &int_digits, &dec_digits);
    out
}

/// Lays the digit strings into the section's placeholders.
fn emit(out: &mut String, section: &Section, shape: &Shape, integer: &str, decimals: &str) {
    let grouped = if shape.grouped {
        group_thousands(integer)
    } else {
        integer.to_string()
    };

    // Digits fill the integer placeholders from the *right*: `#,##0` on 123
    // is `123`, not `1230`, and `(000) 000-0000` lays a phone number out
    // correctly. Any overflow lands in the leftmost placeholder that gets one.
    let chars: Vec<char> = grouped.chars().collect();
    let placeholders = shape.int_places.len();
    // Placeholders with no digit to show at all, at the left.
    let empty = placeholders.saturating_sub(chars.len());
    let mut placeholder = 0usize;
    let mut taken = 0usize;
    let mut dec_seen = 0usize;
    let mut past_point = false;

    for tok in &section.tokens {
        match tok {
            Tok::Digit(kind) if !past_point => {
                if placeholder < empty {
                    if *kind == FORCED {
                        out.push('0');
                    } else if *kind == SPACED {
                        out.push(' ');
                    }
                } else {
                    let remaining_after = placeholders - placeholder - 1;
                    let take = chars.len().saturating_sub(taken + remaining_after).max(1);
                    let end = (taken + take).min(chars.len());
                    out.extend(&chars[taken.min(end)..end]);
                    taken = end;
                }
                placeholder += 1;
            }
            Tok::Digit(kind) => {
                match decimals.as_bytes().get(dec_seen) {
                    Some(d) => out.push(*d as char),
                    None if *kind == FORCED => out.push('0'),
                    None if *kind == SPACED => out.push(' '),
                    None => {}
                }
                dec_seen += 1;
            }
            Tok::Point => {
                // `#.##` on a whole number still shows the point in Excel only
                // when a decimal digit follows; with none, the point is dropped.
                if !decimals.is_empty() || shape.dec_places.contains(&FORCED) {
                    out.push('.');
                }
                past_point = true;
            }
            Tok::Literal(lit) => out.push_str(lit),
            Tok::Percent => out.push('%'),
            Tok::General => out.push_str(&grouped),
            Tok::Group | Tok::Text | Tok::Fraction | Tok::Exponent { .. } => {}
            _ => {}
        }
    }
}

fn group_thousands(digits: &str) -> String {
    if digits.len() <= 3 {
        return digits.to_string();
    }
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    let lead = digits.len() % 3;
    if lead > 0 {
        out.push_str(&digits[..lead]);
    }
    for (i, chunk) in digits.as_bytes()[lead..].chunks(3).enumerate() {
        if i > 0 || lead > 0 {
            out.push(',');
        }
        out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
    }
    out
}

fn render_scientific(section: &Section, value: f64, shape: &Shape) -> String {
    let decimals = shape.dec_places.len();
    let mantissa_places = shape.int_places.len().max(1);
    if value == 0.0 {
        let mut out = format!("{:.*}", decimals, 0.0);
        let _ = write!(out, "E+{:0width$}", 0, width = shape.exp_places.max(2));
        return out;
    }
    let mut exp = value.abs().log10().floor() as i32;
    // `##0.0E+0` puts the exponent in steps of three — engineering notation.
    if mantissa_places > 1 {
        exp -= exp.rem_euclid(mantissa_places as i32);
    }
    let mantissa = value / 10f64.powi(exp);
    let plus = section
        .tokens
        .iter()
        .any(|t| matches!(t, Tok::Exponent { plus: true }));

    let mut out = format!("{mantissa:.decimals$}");
    out.push('E');
    if exp < 0 {
        out.push('-');
    } else if plus {
        out.push('+');
    }
    let _ = write!(
        out,
        "{:0width$}",
        exp.abs(),
        width = shape.exp_places.max(1)
    );
    out
}

/// `# ?/?` — the closest fraction with a denominator of the requested width.
fn render_fraction(section: &Section, value: f64, shape: &Shape, den_places: usize) -> String {
    let whole_part = !shape.int_places.is_empty();
    let negative = value < 0.0;
    let magnitude = value.abs();
    let whole = if whole_part { magnitude.floor() } else { 0.0 };
    let remainder = magnitude - whole;

    let max_den = 10u64.pow(den_places as u32) - 1;
    let (mut num, mut den) = (0u64, 1u64);
    let mut best = f64::INFINITY;
    for d in 1..=max_den.max(1) {
        let n = (remainder * d as f64).round() as u64;
        let error = (remainder - n as f64 / d as f64).abs();
        if error < best {
            best = error;
            num = n;
            den = d;
            if error == 0.0 {
                break;
            }
        }
    }

    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if whole_part {
        // A fraction that rounded up to a whole is absorbed by the whole part.
        let carry = num == den && den != 0;
        let whole = whole + f64::from(u8::from(carry));
        let show_fraction = !carry && num > 0;
        if whole != 0.0 {
            let _ = write!(out, "{whole}");
            if show_fraction {
                out.push(' ');
            }
        }
        if show_fraction {
            let _ = write!(out, "{num}/{den}");
        }
        if out.is_empty() || out == "-" {
            out.push('0');
        }
    } else {
        let _ = write!(out, "{}/{}", num + whole as u64 * den, den);
    }
    // Literals outside the digits — a trailing unit, say — still belong.
    for tok in &section.tokens {
        if let Tok::Literal(lit) = tok {
            if !lit.trim().is_empty() {
                out.push_str(lit);
            }
        }
    }
    out
}

/// How a value is rounded once the digit to drop has been found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rounding {
    /// Half away from zero: 2.5 goes to 3, and -2.5 goes to -3.
    HalfAway,
    /// Always away from zero.
    Up,
    /// Always toward zero.
    Down,
}

/// Rounds to `digits` places after the decimal point, in decimal.
///
/// The detour through a decimal string is not decoration. `ROUND(1.005, 2)` is
/// 1.01 in Excel, but 1.005 as an f64 is really 1.00499999999999989, so scaling
/// by 100 and rounding gives 1.00. Excel rounds the fifteen-significant-digit
/// decimal it would *display*, and matching it means doing the same.
pub fn round_decimal(x: f64, digits: i32, mode: Rounding) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let sign = if x < 0.0 { -1.0 } else { 1.0 };

    let sci = format!("{:.14e}", x.abs());
    let (mantissa, exp) = sci
        .split_once('e')
        .expect("`{:e}` always emits an exponent");
    let exp: i32 = exp.parse().expect("`{:e}` emits a decimal exponent");
    let all: Vec<u8> = mantissa.bytes().filter(u8::is_ascii_digit).collect();

    // Digit `i` has place value 10^(exp - i); we keep those down to 10^-digits.
    let keep = exp + digits + 1;

    if keep <= 0 {
        let carry = match mode {
            Rounding::HalfAway => keep == 0 && all[0] >= b'5',
            Rounding::Up => true,
            Rounding::Down => false,
        };
        return if carry {
            sign * 10f64.powi(-digits)
        } else {
            0.0
        };
    }
    let keep = keep as usize;
    if keep >= all.len() {
        return x;
    }

    let mut kept: Vec<u8> = all[..keep].to_vec();
    let carry = match mode {
        Rounding::HalfAway => all[keep] >= b'5',
        Rounding::Up => all[keep..].iter().any(|&d| d != b'0'),
        Rounding::Down => false,
    };

    let mut exp = exp;
    if carry {
        let mut i = keep;
        loop {
            if i == 0 {
                // Every digit was a nine: 999 became 1000, one place wider.
                kept.insert(0, b'1');
                kept.pop();
                exp += 1;
                break;
            }
            i -= 1;
            if kept[i] == b'9' {
                kept[i] = b'0';
            } else {
                kept[i] += 1;
                break;
            }
        }
    }

    let digits_text = String::from_utf8(kept).expect("ASCII digits");
    let rebuilt: f64 = format!("{}e{}", digits_text, exp - (keep as i32 - 1))
        .parse()
        .unwrap_or(x.abs());
    sign * rebuilt
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(code: &str, x: f64) -> String {
        NumberFormat::parse(code)
            .format(FormatValue::Number(x))
            .text
    }

    #[test]
    fn general_format_stops_at_fifteen_digits() {
        // The sixteenth digit is where binary floating point stops agreeing
        // with what the user typed.
        assert_eq!(format_general(0.1 + 0.2), "0.3");
        assert_eq!(format_general(1.0 / 3.0), "0.333333333333333");
        assert_eq!(format_general(1.0), "1");
        assert_eq!(format_general(-0.0), "0");
        assert_eq!(format_general(1e15), "1E+15");
        assert_eq!(format_general(1e-8), "1E-08", "two exponent digits minimum");
    }

    #[test]
    fn digit_placeholders_pad_or_drop() {
        assert_eq!(show("0", 5.0), "5");
        assert_eq!(show("000", 5.0), "005");
        assert_eq!(show("0.00", 1.5), "1.50");
        assert_eq!(show("#.##", 1.5), "1.5");
        assert_eq!(show("#.##", 0.5), ".5", "`#` drops the leading zero");
        assert_eq!(show("0.##", 0.5), "0.5");
        assert_eq!(show("0.00", 1.005), "1.01", "decimal rounding, not binary");
        assert_eq!(show("0.00", 2.675), "2.68");
    }

    #[test]
    fn commas_group_between_digits_and_scale_after_them() {
        assert_eq!(show("#,##0", 1234567.0), "1,234,567");
        assert_eq!(show("#,##0.00", 1234.5), "1,234.50");
        assert_eq!(show("#,##0", 123.0), "123");
        // A trailing comma divides by a thousand instead of grouping.
        assert_eq!(show("#,##0,", 1234567.0), "1,235");
        assert_eq!(show("0,,", 12_000_000.0), "12");
    }

    #[test]
    fn sections_split_by_sign_and_by_count() {
        // One section covers everything, so the minus has to be supplied.
        assert_eq!(show("0.00", -1.5), "-1.50");
        // Two sections split at zero, and the second writes its own sign.
        assert_eq!(show("#,##0;(#,##0)", -1234.0), "(1,234)");
        assert_eq!(show("#,##0;(#,##0)", 1234.0), "1,234");
        // Three sections give zero its own.
        assert_eq!(show("0;(0);\"-\"", 0.0), "-");
        assert_eq!(show("0;(0);\"-\"", -3.0), "(3)");
    }

    #[test]
    fn a_percentage_is_scaled_as_well_as_suffixed() {
        assert_eq!(show("0%", 0.5), "50%");
        assert_eq!(show("0.0%", 0.1234), "12.3%");
        assert_eq!(show("0%", 1.0), "100%");
    }

    #[test]
    fn dates_render_from_the_serial() {
        // 45352 is 1 March 2024, a Friday.
        assert_eq!(show("yyyy-mm-dd", 45352.0), "2024-03-01");
        assert_eq!(show("m/d/yy", 45352.0), "3/1/24");
        assert_eq!(show("d-mmm-yyyy", 45352.0), "1-Mar-2024");
        assert_eq!(show("mmmm d, yyyy", 45352.0), "March 1, 2024");
        assert_eq!(show("dddd", 45352.0), "Friday");
        assert_eq!(show("ddd", 45352.0), "Fri");
        assert_eq!(show("mmmmm", 45352.0), "M");
    }

    #[test]
    fn the_same_m_is_a_month_or_a_minute_depending_on_its_neighbours() {
        // The rule that cannot be decided while tokenizing.
        let noon_ish = 45352.0 + (13.0 * 3600.0 + 5.0 * 60.0 + 9.0) / 86400.0;
        assert_eq!(show("mm/dd", noon_ish), "03/01");
        assert_eq!(show("hh:mm", noon_ish), "13:05");
        assert_eq!(show("mm:ss", noon_ish), "05:09", "minute before a second");
        assert_eq!(show("h:mm:ss", noon_ish), "13:05:09");
        assert_eq!(show("m/d/yy h:mm", noon_ish), "3/1/24 13:05");
    }

    #[test]
    fn a_twelve_hour_clock_needs_its_marker() {
        let afternoon = 45352.0 + 13.5 / 24.0;
        let morning = 45352.0 + 9.25 / 24.0;
        let midnight = 45352.0;
        assert_eq!(show("h:mm AM/PM", afternoon), "1:30 PM");
        assert_eq!(show("h:mm AM/PM", morning), "9:15 AM");
        // Midnight is twelve, not zero, on a twelve-hour clock.
        assert_eq!(show("h:mm AM/PM", midnight), "12:00 AM");
        assert_eq!(show("h:mm", midnight), "0:00");
    }

    #[test]
    fn elapsed_time_passes_twenty_four_hours() {
        // The point of `[h]`: a duration is not a clock reading, so 1.5 days is
        // 36 hours rather than half past midnight.
        assert_eq!(show("[h]:mm", 1.5), "36:00");
        assert_eq!(show("h:mm", 1.5), "12:00");
        assert_eq!(show("[m]", 1.0), "1440");
    }

    #[test]
    fn colors_and_conditions_come_out_of_the_brackets() {
        let f = NumberFormat::parse("[Red]-0.00");
        assert_eq!(f.format(FormatValue::Number(1.0)).color, Some([255, 0, 0]));
        let f = NumberFormat::parse("[>100]\"high\";[<0]\"low\";0");
        assert_eq!(f.format(FormatValue::Number(200.0)).text, "high");
        assert_eq!(f.format(FormatValue::Number(-5.0)).text, "low");
        assert_eq!(f.format(FormatValue::Number(50.0)).text, "50");
    }

    #[test]
    fn literals_survive_in_all_their_spellings() {
        assert_eq!(show("0\" kg\"", 5.0), "5 kg");
        assert_eq!(show("\\$0", 5.0), "$5");
        assert_eq!(show("[$$-409]#,##0.00", 1234.5), "$1,234.50");
        // `_(` reserves a width; `*` fills, and needs a column we do not have.
        assert_eq!(show("_(0_)", 5.0), " 5 ");
        assert_eq!(show("*-0", 5.0), "5");
    }

    #[test]
    fn digits_fill_placeholders_from_the_right() {
        // A phone-number format only works if the placeholders are positional.
        assert_eq!(show("(000) 000-0000", 5551234567.0), "(555) 123-4567");
        // More digits than placeholders overflow into the leftmost.
        assert_eq!(show("00", 12345.0), "12345");
    }

    #[test]
    fn scientific_and_engineering_notation() {
        assert_eq!(show("0.00E+00", 12345.0), "1.23E+04");
        assert_eq!(show("0.00E+00", 0.00012), "1.20E-04");
        // Three integer placeholders step the exponent in threes.
        assert_eq!(show("##0.0E+0", 12345.0), "12.3E+3");
    }

    #[test]
    fn fractions_pick_the_closest_denominator_that_fits() {
        assert_eq!(show("# ?/?", 2.5), "2 1/2");
        assert_eq!(show("# ?/?", 2.0), "2");
        assert_eq!(show("# ??/??", 2.7), "2 7/10");
        assert_eq!(show("# ?/?", 0.333), "1/3");
    }

    #[test]
    fn text_uses_the_fourth_section_and_otherwise_passes_through() {
        let f = NumberFormat::parse("0.00");
        assert_eq!(f.format(FormatValue::Text("hi")).text, "hi");
        let f = NumberFormat::parse("0.00;-0.00;0;\"[\"@\"]\"");
        assert_eq!(f.format(FormatValue::Text("hi")).text, "[hi]");
        // A blank cell shows nothing whatever the format says.
        assert_eq!(f.format(FormatValue::Blank).text, "");
    }

    #[test]
    fn the_builtin_ids_that_files_never_spell_out() {
        // A cell with `numFmtId="14"` is a date and the file does not say so.
        assert_eq!(NumberFormat::builtin(14), Some("mm-dd-yy"));
        assert_eq!(NumberFormat::builtin(0), Some("General"));
        assert_eq!(NumberFormat::builtin(9), Some("0%"));
        assert_eq!(
            NumberFormat::builtin(164),
            None,
            "custom formats start here"
        );
        let date = NumberFormat::parse(NumberFormat::builtin(14).expect("id 14"));
        assert_eq!(date.format(FormatValue::Number(45352.0)).text, "03-01-24");
    }

    #[test]
    fn an_unparseable_code_still_shows_something() {
        // A cell must always display; a format we cannot read is not a reason
        // to show nothing.
        assert_eq!(show("", 5.0), "5");
        assert_eq!(show("General", 5.5), "5.5");
        assert_eq!(show("[nonsense]0", 5.0), "5");
    }
    #[test]
    fn rounding_is_half_away_from_zero_not_bankers() {
        // Rust's `f64::round` agrees here, but `format!("{:.0}")` does not — it
        // rounds half to even, which would make this 2.
        assert_eq!(round_decimal(2.5, 0, Rounding::HalfAway), 3.0);
        assert_eq!(round_decimal(-2.5, 0, Rounding::HalfAway), -3.0);
        assert_eq!(round_decimal(1.5, 0, Rounding::HalfAway), 2.0);
        assert_eq!(round_decimal(0.5, 0, Rounding::HalfAway), 1.0);
    }

    #[test]
    fn rounding_works_on_the_decimal_not_the_binary_value() {
        // The case that catches every naive implementation.
        assert_eq!(round_decimal(1.005, 2, Rounding::HalfAway), 1.01);
        assert_eq!(round_decimal(2.675, 2, Rounding::HalfAway), 2.68);
        assert_eq!(round_decimal(1.015, 2, Rounding::HalfAway), 1.02);
    }

    #[test]
    fn rounding_to_negative_digits_rounds_left_of_the_point() {
        assert_eq!(round_decimal(1234.0, -2, Rounding::HalfAway), 1200.0);
        assert_eq!(round_decimal(1250.0, -2, Rounding::HalfAway), 1300.0);
        assert_eq!(round_decimal(1234.0, -5, Rounding::HalfAway), 0.0);
        assert_eq!(round_decimal(1234.0, -3, Rounding::Up), 2000.0);
    }

    #[test]
    fn rounding_carries_all_the_way_through_nines() {
        assert_eq!(round_decimal(9.99, 1, Rounding::HalfAway), 10.0);
        assert_eq!(round_decimal(0.999, 2, Rounding::Up), 1.0);
        assert_eq!(round_decimal(-9.99, 1, Rounding::HalfAway), -10.0);
    }

    #[test]
    fn up_and_down_move_away_from_and_toward_zero() {
        assert_eq!(round_decimal(3.2, 0, Rounding::Up), 4.0);
        assert_eq!(round_decimal(-3.2, 0, Rounding::Up), -4.0);
        assert_eq!(round_decimal(3.9, 0, Rounding::Down), 3.0);
        assert_eq!(round_decimal(-3.9, 0, Rounding::Down), -3.0);
        assert_eq!(round_decimal(0.001, 1, Rounding::Up), 0.1);
        assert_eq!(round_decimal(0.001, 1, Rounding::Down), 0.0);
    }
}
