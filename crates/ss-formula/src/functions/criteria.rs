//! The criteria mini-language shared by `SUMIF`, `COUNTIF`, and their plural
//! forms.
//!
//! A criterion is a value, possibly with a comparison operator glued to the
//! front as text: `">5"`, `"<>"`, `"apple"`, `"*a*"`. It looks trivial and is
//! not — the operator is only recognized when the criterion arrives as *text*,
//! wildcards apply only to equality, and the right-hand side is re-parsed as a
//! number when it looks like one, so `">5"` matches the number 5.5 but not the
//! text "5.5".

use std::cmp::Ordering;

use crate::value::{compare, text_to_number, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Criterion {
    op: Op,
    target: Value,
    /// Set when the target is text containing `*` or `?`, and the operator is
    /// equality. Wildcards mean nothing to `>` and `<`.
    pattern: Option<String>,
}

impl Criterion {
    /// Reads a criterion from the value the user supplied.
    pub fn parse(v: &Value) -> Self {
        let Value::Text(s) = v else {
            // A number, boolean, or blank criterion is a plain equality test.
            return Criterion {
                op: Op::Eq,
                target: v.clone(),
                pattern: None,
            };
        };

        // Longest operator first, or `<=` reads as `<`.
        let (op, rest) = if let Some(r) = s.strip_prefix("<=") {
            (Op::Le, r)
        } else if let Some(r) = s.strip_prefix(">=") {
            (Op::Ge, r)
        } else if let Some(r) = s.strip_prefix("<>") {
            (Op::Ne, r)
        } else if let Some(r) = s.strip_prefix('<') {
            (Op::Lt, r)
        } else if let Some(r) = s.strip_prefix('>') {
            (Op::Gt, r)
        } else if let Some(r) = s.strip_prefix('=') {
            (Op::Eq, r)
        } else {
            (Op::Eq, s.as_str())
        };

        // `">5"` compares against the number 5, not the text "5".
        let target = if rest.is_empty() {
            // `"="` matches blanks, `"<>"` matches everything that is not blank.
            Value::Blank
        } else if let Some(n) = text_to_number(rest) {
            Value::Number(n)
        } else if rest.eq_ignore_ascii_case("TRUE") {
            Value::Bool(true)
        } else if rest.eq_ignore_ascii_case("FALSE") {
            Value::Bool(false)
        } else {
            Value::Text(rest.to_string())
        };

        let pattern = match (&target, op) {
            (Value::Text(t), Op::Eq | Op::Ne) if has_wildcard(t) => Some(t.clone()),
            _ => None,
        };

        Criterion {
            op,
            target,
            pattern,
        }
    }
}

/// True when the criterion has to go through the wildcard matcher.
///
/// An escape counts even though it disables the wildcard: `"a~*"` must match the
/// literal text `a*`, and a plain comparison would look for the tilde too.
fn has_wildcard(s: &str) -> bool {
    s.bytes().any(|b| matches!(b, b'*' | b'?' | b'~'))
}

/// Which comparison family a value belongs to. Two values only compare when
/// they agree here.
fn kind(v: &Value) -> u8 {
    match v {
        Value::Blank => 0,
        Value::Number(_) => 1,
        Value::Text(_) => 2,
        Value::Bool(_) => 3,
        Value::Error(_) => 4,
    }
}

/// Tests one value against a criterion.
pub fn matches_criteria(c: &Criterion, v: &Value) -> bool {
    // An error value never satisfies a criterion; callers that care surface it
    // themselves before getting here.
    if v.is_error() {
        return false;
    }

    if let Some(pattern) = &c.pattern {
        let Value::Text(text) = v else {
            return c.op == Op::Ne;
        };
        let hit = wildcard_match(pattern, text);
        return if c.op == Op::Ne { !hit } else { hit };
    }

    // Criteria compare *within* a type, which is where they part ways with the
    // `=` operator. Under the operator's rules text sorts above every number, so
    // `">2"` would match a column's text header — and `SUMIF(A:A,">2")` down a
    // column with a heading would quietly include it. Excel does not: a numeric
    // criterion only ever looks at numbers.
    //
    // Blank is its own type here for the same reason. `"=0"` must not match an
    // empty cell, or every unused row in the range would count.
    if kind(v) != kind(&c.target) {
        return c.op == Op::Ne;
    }

    let Ok(ord) = compare(v, &c.target) else {
        return false;
    };
    match c.op {
        Op::Eq => ord == Ordering::Equal,
        Op::Ne => ord != Ordering::Equal,
        Op::Lt => ord == Ordering::Less,
        Op::Le => ord != Ordering::Greater,
        Op::Gt => ord == Ordering::Greater,
        Op::Ge => ord != Ordering::Less,
    }
}

/// Excel's wildcard match: `*` for any run, `?` for one character, `~` to escape
/// either. Case-insensitive, like every other text comparison.
///
/// Iterative with backtracking rather than recursive: `SEARCH` runs this per
/// cell over whole columns, and a pattern of many `*` should not turn that
/// exponential.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let t: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();

    let (mut pi, mut ti) = (0usize, 0usize);
    // Where to resume if the current `*` turns out to have matched too little.
    let mut star: Option<(usize, usize)> = None;

    while ti < t.len() {
        let literal = match p.get(pi) {
            Some('*') => {
                star = Some((pi, ti));
                pi += 1;
                continue;
            }
            Some('?') => {
                pi += 1;
                ti += 1;
                continue;
            }
            Some('~') => p.get(pi + 1).copied(),
            Some(&c) => Some(c),
            None => None,
        };
        let width = if p.get(pi) == Some(&'~') && literal.is_some() {
            2
        } else {
            1
        };

        if literal == Some(t[ti]) {
            pi += width;
            ti += 1;
        } else if let Some((sp, st)) = star {
            // Let the star swallow one more character and try again.
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }

    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(criterion: &str, v: Value) -> bool {
        matches_criteria(&Criterion::parse(&Value::text(criterion)), &v)
    }

    #[test]
    fn comparison_criteria_parse_their_operator() {
        assert!(m(">5", Value::Number(5.5)));
        assert!(!m(">5", Value::Number(5.0)));
        assert!(m(">=5", Value::Number(5.0)));
        assert!(m("<>5", Value::Number(4.0)));
        assert!(!m("<>5", Value::Number(5.0)));
        assert!(m("<=5", Value::Number(5.0)));
    }

    #[test]
    fn a_bare_value_is_an_equality_test() {
        assert!(m("5", Value::Number(5.0)));
        assert!(
            m("apple", Value::text("APPLE")),
            "text matching ignores case"
        );
        assert!(!m("apple", Value::text("apples")));
    }

    #[test]
    fn a_numeric_criterion_matches_the_number_not_its_text() {
        // The right-hand side is re-parsed, so ">5" is a numeric comparison.
        assert!(m(">5", Value::Number(10.0)));
        // But a number never equals text, so "5" does not match the text "5".
        assert!(!m("5", Value::text("5")));
    }

    #[test]
    fn wildcards_apply_only_to_equality() {
        assert!(m("a*", Value::text("apple")));
        assert!(m("?pple", Value::text("apple")));
        assert!(!m("?pple", Value::text("pple")));
        assert!(m("<>a*", Value::text("banana")));
        // `>` takes the asterisk literally, so this is an ordinary text compare.
        assert!(m(">a*", Value::text("b")));
    }

    #[test]
    fn tilde_escapes_a_wildcard() {
        assert!(m("a~*", Value::text("a*")));
        assert!(!m("a~*", Value::text("ab")));
        assert!(m("a~?", Value::text("a?")));
    }

    #[test]
    fn a_numeric_criterion_never_matches_text() {
        // The failure this prevents: `SUMIF(A:A,">0")` down a column with a text
        // heading. Under the `=` operator's rules text sorts above every number,
        // so the heading would satisfy ">0" and be counted.
        assert!(!m(">0", Value::text("apple")));
        assert!(!m("<0", Value::text("apple")));
        assert!(!m(">0", Value::Bool(true)));
        // Inequality still holds across types, which is what makes "<>3" pick up
        // the text rows as well as the numeric ones.
        assert!(m("<>3", Value::text("apple")));
        assert!(!m(">\"a\"", Value::Number(5.0)));
    }

    #[test]
    fn blank_criteria_distinguish_empty_from_zero() {
        // The trap: if blank coerced to 0 here, "=0" would match every empty
        // cell in the range and SUMIF over a sparse column would be nonsense.
        assert!(m("=", Value::Blank));
        assert!(!m("=", Value::Number(0.0)));
        assert!(!m("0", Value::Blank));
        assert!(m("<>", Value::Number(0.0)));
        assert!(!m("<>", Value::Blank));
    }

    #[test]
    fn wildcard_backtracking_handles_repeated_stars() {
        assert!(wildcard_match("*a*b*c*", "xxaxxbxxcxx"));
        assert!(!wildcard_match("*a*b*c*", "xxaxxcxxbxx"));
        assert!(wildcard_match("*", ""));
        assert!(wildcard_match("", ""));
        assert!(!wildcard_match("", "a"));
        // Would be exponential without the single-star backtrack point.
        assert!(!wildcard_match("*a*a*a*a*a*a*a*b", &"a".repeat(64)));
    }
}
