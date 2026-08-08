//! Text functions.
//!
//! Two things to know before reading.
//!
//! **Positions are one-based and counted in characters.** Excel counts UTF-16
//! code units, so `LEN` of an emoji is 2 there and 1 here. Matching that exactly
//! would mean `MID` could cut a surrogate pair in half, which Rust's `String`
//! cannot represent at all. Characters are the closest honest approximation.
//!
//! **`CHAR` and `CODE` are Windows-1252, not Unicode.** They predate Unicode and
//! still speak the Windows ANSI code page; `UNICHAR` and `UNICODE` are the
//! modern pair. `CHAR(133)` is an ellipsis, not a control character.

use ss_model::CellError;

use crate::ast::Expr;
use crate::eval::Evaluator;
use crate::value::{text_to_number, Operand, Value};

use super::criteria::wildcard_match;
use super::{arity, map_text, scalar_args, visit_args, FnImpl};

pub(super) fn lookup(name: &str) -> Option<FnImpl> {
    Some(match name {
        "LEN" => |ev, a| map_text(ev, a, |s| Value::Number(s.chars().count() as f64)),
        "LOWER" => |ev, a| map_text(ev, a, |s| Value::text(s.to_lowercase())),
        "UPPER" => |ev, a| map_text(ev, a, |s| Value::text(s.to_uppercase())),
        "PROPER" => |ev, a| map_text(ev, a, |s| Value::text(proper(s))),
        "TRIM" => |ev, a| map_text(ev, a, |s| Value::text(trim(s))),
        "CLEAN" => |ev, a| map_text(ev, a, |s| Value::text(clean(s))),
        "VALUE" => |ev, a| map_text(ev, a, value_of),
        "LEFT" => |ev: &mut Evaluator, a: &[Expr]| take(ev, a, true),
        "RIGHT" => |ev: &mut Evaluator, a: &[Expr]| take(ev, a, false),
        "MID" => mid,
        "REPT" => rept,
        "REPLACE" => replace,
        "SUBSTITUTE" => substitute,
        "FIND" => |ev: &mut Evaluator, a: &[Expr]| locate(ev, a, false),
        "SEARCH" => |ev: &mut Evaluator, a: &[Expr]| locate(ev, a, true),
        "EXACT" => exact,
        "CONCATENATE" => concatenate,
        "CONCAT" => concat,
        "TEXTJOIN" => textjoin,
        "T" => t,
        "CHAR" => char_,
        "CODE" => code,
        "UNICHAR" => unichar,
        "UNICODE" => unicode,
        "NUMBERVALUE" => numbervalue,
        _ => return None,
    })
}

fn proper(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut start_of_word = true;
    for c in s.chars() {
        if start_of_word {
            out.extend(c.to_uppercase());
        } else {
            out.extend(c.to_lowercase());
        }
        // A word runs until the next non-letter, so "o'brien" becomes "O'Brien".
        start_of_word = !c.is_alphabetic();
    }
    out
}

/// `TRIM` removes leading and trailing spaces and collapses internal runs.
///
/// Only the ASCII space, deliberately: the function exists to tidy text imported
/// with ragged padding, and a non-breaking space is content, not padding. Excel
/// leaves those alone too, which is why `TRIM` so often appears to do nothing on
/// text pasted from a web page.
fn trim(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for c in s.chars() {
        if c == ' ' {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(c);
        }
    }
    out
}

fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

fn value_of(s: &str) -> Value {
    match text_to_number(s) {
        Some(n) => Value::Number(n),
        None => Value::Error(CellError::Value),
    }
}

/// Reads an argument as a character count, rejecting the negatives Excel rejects.
fn count_arg(ev: &mut Evaluator, arg: Option<&Expr>, default: f64) -> Result<usize, CellError> {
    let n = match arg {
        Some(e) => ev.eval_number(e)?,
        None => default,
    };
    if n < 0.0 || !n.is_finite() {
        return Err(CellError::Value);
    }
    Ok(n.trunc() as usize)
}

fn chars_of(s: &str) -> Vec<char> {
    s.chars().collect()
}

/// `LEFT` and `RIGHT`. Asking for more characters than exist is not an error —
/// you get the whole string.
fn take(ev: &mut Evaluator, args: &[Expr], from_left: bool) -> Operand {
    if !arity(args, 1, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let text = match ev.eval_text(&args[0]) {
        Ok(t) => t,
        Err(e) => return Operand::error(e),
    };
    let n = match count_arg(ev, args.get(1), 1.0) {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    let chars = chars_of(&text);
    let n = n.min(chars.len());
    let slice = if from_left {
        &chars[..n]
    } else {
        &chars[chars.len() - n..]
    };
    Operand::text(slice.iter().collect::<String>())
}

/// `MID(text, start, count)`. `start` is one-based and zero is an error, not the
/// beginning — the single most common off-by-one in spreadsheet formulas.
fn mid(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 3, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let text = match ev.eval_text(&args[0]) {
        Ok(t) => t,
        Err(e) => return Operand::error(e),
    };
    let start = match ev.eval_number(&args[1]) {
        Ok(n) => n.trunc(),
        Err(e) => return Operand::error(e),
    };
    if start < 1.0 {
        return Operand::error(CellError::Value);
    }
    let count = match count_arg(ev, args.get(2), 0.0) {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    let chars = chars_of(&text);
    let start = (start as usize - 1).min(chars.len());
    let end = start.saturating_add(count).min(chars.len());
    Operand::text(chars[start..end].iter().collect::<String>())
}

fn rept(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let text = match ev.eval_text(&args[0]) {
        Ok(t) => t,
        Err(e) => return Operand::error(e),
    };
    let n = match count_arg(ev, args.get(1), 0.0) {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    // Excel caps a cell at 32767 characters and reports #VALUE! past it. Without
    // this, `REPT("x", 1e9)` would try to allocate a gigabyte.
    const MAX_CELL_TEXT: usize = 32_767;
    if text.chars().count().saturating_mul(n) > MAX_CELL_TEXT {
        return Operand::error(CellError::Value);
    }
    Operand::text(text.repeat(n))
}

/// `REPLACE(old, start, count, new)` — replace by position.
fn replace(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 4, Some(4)) {
        return Operand::error(CellError::Value);
    }
    let old = match ev.eval_text(&args[0]) {
        Ok(t) => t,
        Err(e) => return Operand::error(e),
    };
    let start = match ev.eval_number(&args[1]) {
        Ok(n) => n.trunc(),
        Err(e) => return Operand::error(e),
    };
    if start < 1.0 {
        return Operand::error(CellError::Value);
    }
    let count = match count_arg(ev, args.get(2), 0.0) {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    let new = match ev.eval_text(&args[3]) {
        Ok(t) => t,
        Err(e) => return Operand::error(e),
    };
    let chars = chars_of(&old);
    let start = (start as usize - 1).min(chars.len());
    let end = start.saturating_add(count).min(chars.len());
    let mut out: String = chars[..start].iter().collect();
    out.push_str(&new);
    out.extend(&chars[end..]);
    Operand::text(out)
}

/// `SUBSTITUTE(text, old, new, [instance])` — replace by content.
///
/// Without `instance` every occurrence goes; with it, only the nth. An empty
/// `old` matches nothing, rather than matching everywhere and looping forever.
fn substitute(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 3, Some(4)) {
        return Operand::error(CellError::Value);
    }
    let values = match scalar_args(ev, &args[..3]) {
        Ok(v) => v,
        Err(e) => return Operand::error(e),
    };
    let (text, old, new) = match (
        values[0].to_text(),
        values[1].to_text(),
        values[2].to_text(),
    ) {
        (Ok(a), Ok(b), Ok(c)) => (a, b, c),
        _ => return Operand::error(CellError::Value),
    };
    let instance = match args.get(3) {
        Some(e) => match ev.eval_number(e) {
            Ok(n) if n >= 1.0 => Some(n.trunc() as usize),
            Ok(_) => return Operand::error(CellError::Value),
            Err(e) => return Operand::error(e),
        },
        None => None,
    };

    if old.is_empty() {
        return Operand::text(text);
    }
    match instance {
        None => Operand::text(text.replace(&old, &new)),
        Some(want) => {
            let mut seen = 0;
            let mut out = String::with_capacity(text.len());
            let mut rest = text.as_str();
            while let Some(pos) = rest.find(&old) {
                seen += 1;
                out.push_str(&rest[..pos]);
                if seen == want {
                    out.push_str(&new);
                } else {
                    out.push_str(&old);
                }
                rest = &rest[pos + old.len()..];
            }
            out.push_str(rest);
            Operand::text(out)
        }
    }
}

/// `FIND` (exact, literal) and `SEARCH` (case-insensitive, wildcards).
///
/// Both return `#VALUE!` rather than 0 when the text is not there, which is why
/// they are almost always wrapped in `IFERROR`.
fn locate(ev: &mut Evaluator, args: &[Expr], fuzzy: bool) -> Operand {
    if !arity(args, 2, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let needle = match ev.eval_text(&args[0]) {
        Ok(t) => t,
        Err(e) => return Operand::error(e),
    };
    let haystack = match ev.eval_text(&args[1]) {
        Ok(t) => t,
        Err(e) => return Operand::error(e),
    };
    let start = match args.get(2) {
        Some(e) => match ev.eval_number(e) {
            Ok(n) if n >= 1.0 => n.trunc() as usize - 1,
            Ok(_) => return Operand::error(CellError::Value),
            Err(e) => return Operand::error(e),
        },
        None => 0,
    };

    let text = chars_of(&haystack);
    if start > text.len() {
        return Operand::error(CellError::Value);
    }

    if !fuzzy {
        let pattern = chars_of(&needle);
        for i in start..=text.len().saturating_sub(pattern.len()) {
            if text.len() < i + pattern.len() {
                break;
            }
            if text[i..i + pattern.len()] == pattern[..] {
                return Operand::number(i as f64 + 1.0);
            }
        }
        return Operand::error(CellError::Value);
    }

    // A wildcard match anchored at each position in turn: the pattern has to
    // match from here, but need not reach the end, hence the trailing star.
    let anchored = format!("{needle}*");
    for i in start..=text.len() {
        let tail: String = text[i..].iter().collect();
        if wildcard_match(&anchored, &tail) {
            return Operand::number(i as f64 + 1.0);
        }
    }
    Operand::error(CellError::Value)
}

/// The one text comparison that respects case.
fn exact(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let a = ev.eval(&args[0]);
    let b = ev.eval(&args[1]);
    ev.broadcast(a, b, |x, y| match (x.to_text(), y.to_text()) {
        (Ok(x), Ok(y)) => Value::Bool(x == y),
        (Err(e), _) | (_, Err(e)) => Value::Error(e),
    })
}

/// `CONCATENATE` takes scalars only; `CONCAT` flattens ranges. The pair exists
/// because the old one could not do the useful thing.
fn concatenate(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if args.is_empty() {
        return Operand::error(CellError::Value);
    }
    let mut out = String::new();
    for a in args {
        match ev.eval_scalar(a).to_text() {
            Ok(s) => out.push_str(&s),
            Err(e) => return Operand::error(e),
        }
    }
    Operand::text(out)
}

fn concat(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if args.is_empty() {
        return Operand::error(CellError::Value);
    }
    let mut out = String::new();
    let mut err = None;
    visit_args(ev, args, &mut |v, _| {
        if err.is_some() {
            return;
        }
        match v.to_text() {
            Ok(s) => out.push_str(&s),
            Err(e) => err = Some(e),
        }
    });
    match err {
        Some(e) => Operand::error(e),
        None => Operand::text(out),
    }
}

/// `TEXTJOIN(delimiter, ignore_empty, text...)`.
fn textjoin(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 3, None) {
        return Operand::error(CellError::Value);
    }
    let delimiter = match ev.eval_text(&args[0]) {
        Ok(t) => t,
        Err(e) => return Operand::error(e),
    };
    let ignore_empty = match ev.eval_bool(&args[1]) {
        Ok(b) => b,
        Err(e) => return Operand::error(e),
    };
    let mut parts: Vec<String> = Vec::new();
    let mut err = None;
    visit_args(ev, &args[2..], &mut |v, _| {
        if err.is_some() {
            return;
        }
        match v.to_text() {
            Ok(s) => {
                if !(ignore_empty && s.is_empty()) {
                    parts.push(s);
                }
            }
            Err(e) => err = Some(e),
        }
    });
    match err {
        Some(e) => Operand::error(e),
        None => Operand::text(parts.join(&delimiter)),
    }
}

/// `T` passes text through and turns everything else into an empty string —
/// except errors, which it keeps.
fn t(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(1)) {
        return Operand::error(CellError::Value);
    }
    match ev.eval_scalar(&args[0]) {
        Value::Text(s) => Operand::text(s),
        Value::Error(e) => Operand::error(e),
        _ => Operand::text(""),
    }
}

/// The Windows-1252 characters at 128–159, where it differs from Latin-1.
///
/// This range is undefined control characters in ISO 8859-1 and printable
/// punctuation in the Windows code page, which is why `CHAR(147)` is a curly
/// quote in Excel and nothing at all in a strict Latin-1 reading.
const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{81}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{8D}', '\u{017D}', '\u{8F}',
    '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{9D}', '\u{017E}', '\u{0178}',
];

fn char_(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    super::map_number(ev, args, |n| {
        let n = n.trunc();
        if !(1.0..=255.0).contains(&n) {
            return Value::Error(CellError::Value);
        }
        let code = n as u32;
        let c = match code {
            128..=159 => CP1252_HIGH[(code - 128) as usize],
            _ => char::from_u32(code).unwrap_or('\u{FFFD}'),
        };
        Value::Text(c.to_string())
    })
}

fn code(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    map_text(ev, args, |s| match s.chars().next() {
        None => Value::Error(CellError::Value),
        Some(c) => Value::Number(f64::from(cp1252_code(c))),
    })
}

fn cp1252_code(c: char) -> u32 {
    if let Some(i) = CP1252_HIGH.iter().position(|&h| h == c) {
        return 128 + i as u32;
    }
    match c as u32 {
        n @ (0..=127 | 160..=255) => n,
        // Excel answers 63 — a question mark — for anything the code page
        // cannot represent, rather than erroring.
        _ => 63,
    }
}

fn unichar(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    super::map_number(ev, args, |n| {
        let n = n.trunc();
        if n < 1.0 || n > f64::from(char::MAX as u32) {
            return Value::Error(CellError::Value);
        }
        match char::from_u32(n as u32) {
            // Lone surrogates are not characters; Excel reports #VALUE!.
            Some(c) => Value::Text(c.to_string()),
            None => Value::Error(CellError::Value),
        }
    })
}

fn unicode(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    map_text(ev, args, |s| match s.chars().next() {
        None => Value::Error(CellError::Value),
        Some(c) => Value::Number(f64::from(c as u32)),
    })
}

/// `NUMBERVALUE(text, [decimal_separator], [group_separator])` — `VALUE` for
/// text written with someone else's punctuation.
fn numbervalue(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let text = match ev.eval_text(&args[0]) {
        Ok(t) => t,
        Err(e) => return Operand::error(e),
    };
    let separator = |ev: &mut Evaluator, i: usize, fallback: char| -> Result<char, CellError> {
        match args.get(i) {
            Some(e) => Ok(ev.eval_text(e)?.chars().next().unwrap_or(fallback)),
            None => Ok(fallback),
        }
    };
    let decimal = match separator(ev, 1, '.') {
        Ok(c) => c,
        Err(e) => return Operand::error(e),
    };
    let group = match separator(ev, 2, ',') {
        Ok(c) => c,
        Err(e) => return Operand::error(e),
    };
    if decimal == group {
        return Operand::error(CellError::Value);
    }

    // An empty string is 0 here, where VALUE would call it #VALUE!.
    if text.trim().is_empty() {
        return Operand::number(0.0);
    }
    let normalized: String = text
        .chars()
        .filter(|&c| c != group && !c.is_whitespace())
        .map(|c| if c == decimal { '.' } else { c })
        .collect();
    match text_to_number(&normalized) {
        Some(n) => Operand::number(n),
        None => Operand::error(CellError::Value),
    }
}
