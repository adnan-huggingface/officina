//! What a formula evaluates to, and Excel's rules for converting between types.
//!
//! The conversions are the interesting part. Excel is not a consistently typed
//! language: `"1"+1` is 2, `SUM("1",1)` is 2, but `SUM(A1,1)` where A1 holds the
//! *text* "1" is 1. Whether text becomes a number depends on how the value
//! reached the function, not on what the value is. Nothing here can be inferred
//! from first principles — every rule below is Excel's observed behaviour.

use std::cmp::Ordering;

use ss_model::CellError;

use crate::graph::AreaRef;

/// A single value: what one cell holds, and what a scalar expression produces.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Value {
    /// An empty cell, or an argument written as nothing at all.
    #[default]
    Blank,
    Number(f64),
    Text(String),
    Bool(bool),
    Error(CellError),
}

impl Value {
    pub fn text(s: impl Into<String>) -> Self {
        Value::Text(s.into())
    }

    pub const fn is_error(&self) -> bool {
        matches!(self, Value::Error(_))
    }

    pub const fn as_error(&self) -> Option<CellError> {
        match self {
            Value::Error(e) => Some(*e),
            _ => None,
        }
    }

    /// Excel's cross-type ordering: numbers sort before text, text before
    /// booleans. So `TRUE > "zzz" > 9E307`, which surprises people but is what
    /// sorting and comparison both do.
    const fn type_rank(&self) -> u8 {
        match self {
            Value::Number(_) => 0,
            Value::Text(_) => 1,
            Value::Bool(_) => 2,
            Value::Blank => 3,
            Value::Error(_) => 4,
        }
    }

    /// Coerces to a number the way an arithmetic operator does.
    ///
    /// Blank is zero, booleans are 1 and 0, and text is converted if it looks
    /// like a number written out. Anything else is `#VALUE!`.
    pub fn to_number(&self) -> Result<f64, CellError> {
        match self {
            Value::Blank => Ok(0.0),
            Value::Number(n) => Ok(*n),
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Value::Text(s) => text_to_number(s).ok_or(CellError::Value),
            Value::Error(e) => Err(*e),
        }
    }

    /// Coerces to text the way `&` and the text functions do.
    pub fn to_text(&self) -> Result<String, CellError> {
        match self {
            Value::Blank => Ok(String::new()),
            Value::Number(n) => Ok(format_general(*n)),
            Value::Bool(b) => Ok(if *b { "TRUE" } else { "FALSE" }.to_string()),
            Value::Text(s) => Ok(s.clone()),
            Value::Error(e) => Err(*e),
        }
    }

    /// Coerces to a condition, as `IF` and `AND` do.
    ///
    /// Numbers are false only at zero. Text converts only when it spells a
    /// boolean — `IF("yes",..)` is `#VALUE!`, not true.
    pub fn to_bool(&self) -> Result<bool, CellError> {
        match self {
            Value::Blank => Ok(false),
            Value::Number(n) => Ok(*n != 0.0),
            Value::Bool(b) => Ok(*b),
            Value::Text(s) => {
                if s.eq_ignore_ascii_case("TRUE") {
                    Ok(true)
                } else if s.eq_ignore_ascii_case("FALSE") {
                    Ok(false)
                } else {
                    Err(CellError::Value)
                }
            }
            Value::Error(e) => Err(*e),
        }
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Number(n)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Text(s)
    }
}

impl From<CellError> for Value {
    fn from(e: CellError) -> Self {
        Value::Error(e)
    }
}

impl<T: Into<Value>> From<Result<T, CellError>> for Value {
    fn from(r: Result<T, CellError>) -> Self {
        match r {
            Ok(v) => v.into(),
            Err(e) => Value::Error(e),
        }
    }
}

/// Compares two values by Excel's rules.
///
/// Text compares case-insensitively — `"a"="A"` is true, and `EXACT` is the
/// function you reach for when it should not be. Blank takes on the type of
/// whatever it is compared against, which is why `A1=0` and `A1=""` are *both*
/// true for an empty A1.
pub fn compare(a: &Value, b: &Value) -> Result<Ordering, CellError> {
    if let Value::Error(e) = a {
        return Err(*e);
    }
    if let Value::Error(e) = b {
        return Err(*e);
    }
    match (a, b) {
        (Value::Blank, Value::Blank) => Ok(Ordering::Equal),
        (Value::Blank, other) => compare(&blank_as(other), other),
        (other, Value::Blank) => compare(other, &blank_as(other)),
        (Value::Number(x), Value::Number(y)) => Ok(x.partial_cmp(y).unwrap_or(Ordering::Equal)),
        (Value::Text(x), Value::Text(y)) => Ok(compare_text_insensitive(x, y)),
        (Value::Bool(x), Value::Bool(y)) => Ok(x.cmp(y)),
        // Different types never compare equal; they order by type instead.
        _ => Ok(a.type_rank().cmp(&b.type_rank())),
    }
}

/// The zero value of whatever type blank is being compared against.
fn blank_as(other: &Value) -> Value {
    match other {
        Value::Text(_) => Value::Text(String::new()),
        Value::Bool(_) => Value::Bool(false),
        _ => Value::Number(0.0),
    }
}

/// Case-insensitive comparison, done without allocating for the common case.
pub fn compare_text_insensitive(a: &str, b: &str) -> Ordering {
    let mut x = a.chars().flat_map(char::to_lowercase);
    let mut y = b.chars().flat_map(char::to_lowercase);
    loop {
        match (x.next(), y.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) => match p.cmp(&q) {
                Ordering::Equal => continue,
                other => return other,
            },
        }
    }
}

/// Parses text as a number the way Excel does when coercing.
///
/// Accepts surrounding space, a sign, a currency symbol, group separators, a
/// trailing percent sign, and scientific notation. Deliberately does *not* fall
/// through to Rust's `f64::from_str`, which would accept `inf` and `NaN` — both
/// of which are ordinary text to Excel.
pub fn text_to_number(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }

    let (body, scale) = match t.strip_suffix('%') {
        Some(b) => (b.trim_end(), 0.01),
        None => (t, 1.0),
    };

    let mut rest = body;
    let mut sign = 1.0;
    if let Some(r) = rest.strip_prefix('-') {
        sign = -1.0;
        rest = r.trim_start();
    } else if let Some(r) = rest.strip_prefix('+') {
        rest = r.trim_start();
    }
    rest = rest.strip_prefix('$').unwrap_or(rest).trim_start();

    // Rebuild without group separators, validating the shape as we go. A comma
    // is only a separator between digits; `1,,2` and a trailing `1,` are text.
    let bytes = rest.as_bytes();
    let mut cleaned = String::with_capacity(rest.len());
    let mut i = 0;
    let mut digits_before = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'0'..=b'9' => {
                cleaned.push(bytes[i] as char);
                digits_before += 1;
                i += 1;
            }
            b',' if digits_before > 0 && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) => i += 1,
            _ => break,
        }
    }

    let mut digits_after = 0;
    if bytes.get(i) == Some(&b'.') {
        cleaned.push('.');
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            cleaned.push(bytes[i] as char);
            digits_after += 1;
            i += 1;
        }
    }
    if digits_before == 0 && digits_after == 0 {
        return None;
    }

    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        let mut exp = String::from("e");
        if matches!(bytes.get(j), Some(b'+' | b'-')) {
            exp.push(bytes[j] as char);
            j += 1;
        }
        let start = j;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            exp.push(bytes[j] as char);
            j += 1;
        }
        if j > start {
            cleaned.push_str(&exp);
            i = j;
        }
    }

    if rest[i..].trim().is_empty() {
        let n: f64 = cleaned.parse().ok()?;
        n.is_finite().then_some(sign * n * scale)
    } else {
        None
    }
}

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
        return CellError::Num.as_str().to_string();
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
        out.push_str(&format!("{:02}", exp.abs()));
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

/// A rectangular block of values.
///
/// Arrays are always rectangular and always hold scalars — Excel has no ragged
/// or nested arrays — so the shape can be trusted by everything downstream.
#[derive(Debug, Clone, PartialEq)]
pub struct Array {
    rows: usize,
    cols: usize,
    cells: Vec<Value>,
}

impl Array {
    /// Panics if `cells.len() != rows * cols`; the shape is an invariant, not a hint.
    pub fn new(rows: usize, cols: usize, cells: Vec<Value>) -> Self {
        assert_eq!(
            cells.len(),
            rows * cols,
            "array shape does not match its cells"
        );
        Array { rows, cols, cells }
    }

    pub fn filled(rows: usize, cols: usize, v: Value) -> Self {
        Array {
            rows,
            cols,
            cells: vec![v; rows * cols],
        }
    }

    pub fn row_vector(cells: Vec<Value>) -> Self {
        Array {
            rows: 1,
            cols: cells.len(),
            cells,
        }
    }

    pub const fn rows(&self) -> usize {
        self.rows
    }

    pub const fn cols(&self) -> usize {
        self.cols
    }

    pub fn get(&self, row: usize, col: usize) -> Option<&Value> {
        (row < self.rows && col < self.cols).then(|| &self.cells[row * self.cols + col])
    }

    pub fn values(&self) -> impl Iterator<Item = &Value> {
        self.cells.iter()
    }

    /// The single value an array collapses to where a scalar is wanted.
    pub fn first(&self) -> Value {
        self.cells.first().cloned().unwrap_or(Value::Blank)
    }
}

/// One or more rectangles of the workbook.
///
/// Kept unresolved so functions that need reference semantics — and there are
/// many, from `ROW` to `OFFSET` — can see the address rather than the contents.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RefSet {
    pub areas: Vec<AreaRef>,
}

impl RefSet {
    pub fn one(area: AreaRef) -> Self {
        RefSet { areas: vec![area] }
    }

    /// True when this names exactly one cell, which is the shape most functions
    /// that take a reference actually require.
    pub fn single_cell(&self) -> bool {
        self.areas.len() == 1 && {
            let r = self.areas[0].range;
            r.rows() == 1 && r.cols() == 1
        }
    }
}

/// What evaluating an expression produces.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Value(Value),
    Array(Array),
    Ref(RefSet),
}

impl Operand {
    pub fn error(e: CellError) -> Self {
        Operand::Value(Value::Error(e))
    }

    pub fn number(n: f64) -> Self {
        Operand::Value(Value::Number(n))
    }

    pub fn text(s: impl Into<String>) -> Self {
        Operand::Value(Value::Text(s.into()))
    }

    pub fn boolean(b: bool) -> Self {
        Operand::Value(Value::Bool(b))
    }

    /// Wraps an array, collapsing a one-by-one result to the value it holds.
    ///
    /// Operators and mapped functions produce an array whenever *either* side
    /// was one, and a single-cell reference is one. Without this, `A1+1` would
    /// come back as a one-cell array rather than a number, and every caller
    /// downstream would have to unwrap it.
    pub fn from_array(a: Array) -> Self {
        if a.rows() == 1 && a.cols() == 1 {
            Operand::Value(a.first())
        } else {
            Operand::Array(a)
        }
    }
}

impl From<Value> for Operand {
    fn from(v: Value) -> Self {
        Operand::Value(v)
    }
}

impl<T: Into<Value>> From<Result<T, CellError>> for Operand {
    fn from(r: Result<T, CellError>) -> Self {
        Operand::Value(r.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_format_stops_at_fifteen_digits() {
        // The whole point: the sixteenth digit is where binary floating point
        // stops agreeing with what the user typed.
        assert_eq!(format_general(0.1 + 0.2), "0.3");
        assert_eq!(format_general(1.0 / 3.0), "0.333333333333333");
        assert_eq!(format_general(2.0 / 3.0), "0.666666666666667");
    }

    #[test]
    fn general_format_writes_integers_without_a_point() {
        assert_eq!(format_general(1.0), "1");
        assert_eq!(format_general(-42.0), "-42");
        assert_eq!(format_general(0.0), "0");
        assert_eq!(format_general(-0.0), "0", "negative zero displays as zero");
        assert_eq!(format_general(1234567890.0), "1234567890");
    }

    #[test]
    fn general_format_switches_to_scientific_at_the_edges() {
        assert_eq!(format_general(1e15), "1E+15");
        assert_eq!(format_general(1e-8), "1E-08", "two exponent digits minimum");
        assert_eq!(format_general(1e-5), "1E-05");
        assert_eq!(
            format_general(0.0001),
            "0.0001",
            "just inside the plain band"
        );
        assert_eq!(format_general(123456789012345.0), "123456789012345");
    }

    #[test]
    fn text_to_number_accepts_what_excel_accepts() {
        assert_eq!(text_to_number("1"), Some(1.0));
        assert_eq!(text_to_number("  -2.5  "), Some(-2.5));
        assert_eq!(text_to_number("+3"), Some(3.0));
        assert_eq!(text_to_number("1e3"), Some(1000.0));
        assert_eq!(text_to_number("50%"), Some(0.5));
        assert_eq!(text_to_number("$1,000"), Some(1000.0));
        assert_eq!(text_to_number(".5"), Some(0.5));
        assert_eq!(text_to_number("5."), Some(5.0));
    }

    #[test]
    fn text_to_number_rejects_what_is_merely_text() {
        assert_eq!(text_to_number(""), None);
        assert_eq!(text_to_number("   "), None);
        assert_eq!(text_to_number("abc"), None);
        assert_eq!(text_to_number("1 2"), None);
        assert_eq!(text_to_number("1,,000"), None);
        assert_eq!(
            text_to_number("1,00"),
            Some(100.0),
            "misplaced groups still parse"
        );
        assert_eq!(text_to_number("--1"), None);
        // Rust's f64 parser would take all three of these.
        assert_eq!(text_to_number("inf"), None);
        assert_eq!(text_to_number("NaN"), None);
        assert_eq!(text_to_number("infinity"), None);
    }

    #[test]
    fn comparison_orders_numbers_before_text_before_booleans() {
        let n = Value::Number(9e307);
        let t = Value::text("a");
        let b = Value::Bool(false);
        assert_eq!(compare(&n, &t), Ok(Ordering::Less));
        assert_eq!(compare(&t, &b), Ok(Ordering::Less));
        assert_eq!(compare(&b, &n), Ok(Ordering::Greater));
    }

    #[test]
    fn comparison_of_text_ignores_case() {
        assert_eq!(
            compare(&Value::text("a"), &Value::text("A")),
            Ok(Ordering::Equal)
        );
        assert_eq!(
            compare(&Value::text("a"), &Value::text("B")),
            Ok(Ordering::Less)
        );
    }

    #[test]
    fn blank_takes_the_type_of_its_counterpart() {
        // Both of these are true in Excel for an empty cell, which cannot happen
        // if blank has a single fixed type.
        assert_eq!(
            compare(&Value::Blank, &Value::Number(0.0)),
            Ok(Ordering::Equal)
        );
        assert_eq!(
            compare(&Value::Blank, &Value::text("")),
            Ok(Ordering::Equal)
        );
        assert_eq!(
            compare(&Value::Blank, &Value::Bool(false)),
            Ok(Ordering::Equal)
        );
        assert_eq!(compare(&Value::Blank, &Value::Blank), Ok(Ordering::Equal));
        // But "" and 0 are different types, so they are not equal to each other.
        assert_ne!(
            compare(&Value::text(""), &Value::Number(0.0)),
            Ok(Ordering::Equal)
        );
    }

    #[test]
    fn errors_win_over_comparison() {
        assert_eq!(
            compare(&Value::Error(CellError::Div0), &Value::Number(1.0)),
            Err(CellError::Div0)
        );
    }

    #[test]
    fn text_coerces_to_bool_only_when_it_spells_one() {
        assert_eq!(Value::text("true").to_bool(), Ok(true));
        assert_eq!(Value::text("FALSE").to_bool(), Ok(false));
        assert_eq!(Value::text("yes").to_bool(), Err(CellError::Value));
        assert_eq!(Value::Number(-3.0).to_bool(), Ok(true));
        assert_eq!(Value::Number(0.0).to_bool(), Ok(false));
    }
}
