//! Cell addresses and values.

use std::fmt;

use crate::strings::StrId;
use crate::style::StyleId;

/// Excel's grid limits. Addresses outside these are not representable, which is
/// what lets `CellRef` fit in 8 bytes and the chunk index fit in a `u32`.
pub const MAX_ROWS: u32 = 1_048_576;
pub const MAX_COLS: u32 = 16_384;

/// A zero-based cell address.
///
/// Zero-based internally, one-based in the UI and in A1 notation. The conversion
/// happens exactly once, at the display boundary — mixing the two conventions in
/// the middle of a spreadsheet engine is a reliable source of off-by-one bugs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellRef {
    pub row: u32,
    pub col: u32,
}

impl CellRef {
    pub const fn new(row: u32, col: u32) -> Self {
        CellRef { row, col }
    }

    pub const fn is_valid(&self) -> bool {
        self.row < MAX_ROWS && self.col < MAX_COLS
    }

    /// Formats as A1 notation: `(0, 0)` -> `A1`.
    pub fn to_a1(self) -> String {
        format!("{}{}", column_name(self.col), self.row + 1)
    }

    /// Parses A1 notation, rejecting anything out of range.
    ///
    /// Absolute markers (`$A$1`) are accepted and discarded — absoluteness is a
    /// property of a formula reference, not of an address.
    pub fn from_a1(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let bytes = s.as_bytes();
        let mut i = 0;

        if bytes.get(i) == Some(&b'$') {
            i += 1;
        }
        let letters_start = i;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        if i == letters_start {
            return None;
        }
        let col = column_index(&s[letters_start..i])?;

        if bytes.get(i) == Some(&b'$') {
            i += 1;
        }
        let digits = &s[i..];
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let row_one_based: u32 = digits.parse().ok()?;
        if row_one_based == 0 {
            return None;
        }

        let cell = CellRef::new(row_one_based - 1, col);
        cell.is_valid().then_some(cell)
    }
}

impl fmt::Display for CellRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_a1())
    }
}

/// Column index to letters: 0 -> `A`, 25 -> `Z`, 26 -> `AA`.
///
/// This is bijective base-26, not ordinary base-26 — there is no zero digit, so
/// the usual `n % 26` loop is off by one at every carry.
pub fn column_name(mut col: u32) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (col % 26) as u8);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).expect("ASCII letters are valid UTF-8")
}

/// Letters to column index, case-insensitive. `None` if out of range or not letters.
pub fn column_index(name: &str) -> Option<u32> {
    if name.is_empty() {
        return None;
    }
    let mut acc: u32 = 0;
    for b in name.bytes() {
        if !b.is_ascii_alphabetic() {
            return None;
        }
        let digit = (b.to_ascii_uppercase() - b'A') as u32 + 1;
        acc = acc.checked_mul(26)?.checked_add(digit)?;
        if acc > MAX_COLS {
            return None; // reject early rather than overflow on long inputs
        }
    }
    (1..=MAX_COLS).contains(&acc).then(|| acc - 1)
}

/// The error values Excel propagates through formulas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CellError {
    Null,
    Div0,
    Value,
    Ref,
    Name,
    Num,
    NotAvailable,
    GettingData,
    /// Not an Excel error code. Ours, for a cycle with iterative calc disabled.
    Circular,
}

impl CellError {
    pub const fn as_str(self) -> &'static str {
        match self {
            CellError::Null => "#NULL!",
            CellError::Div0 => "#DIV/0!",
            CellError::Value => "#VALUE!",
            CellError::Ref => "#REF!",
            CellError::Name => "#NAME?",
            CellError::Num => "#NUM!",
            CellError::NotAvailable => "#N/A",
            CellError::GettingData => "#GETTING_DATA",
            CellError::Circular => "#CIRCULAR!",
        }
    }

    /// Parses an Excel error literal. Named `from_code` rather than
    /// `from_str` so it is not mistaken for the `FromStr` trait method.
    pub fn from_code(s: &str) -> Option<Self> {
        Some(match s {
            "#NULL!" => CellError::Null,
            "#DIV/0!" => CellError::Div0,
            "#VALUE!" => CellError::Value,
            "#REF!" => CellError::Ref,
            "#NAME?" => CellError::Name,
            "#NUM!" => CellError::Num,
            "#N/A" => CellError::NotAvailable,
            "#GETTING_DATA" => CellError::GettingData,
            "#CIRCULAR!" => CellError::Circular,
            _ => return None,
        })
    }
}

impl fmt::Display for CellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A cell's value.
///
/// Dates are not a variant: Excel stores them as numbers with a date number
/// format, and modeling them separately would desynchronize from that on the
/// first round trip.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum CellValue {
    #[default]
    Blank,
    Number(f64),
    Text(StrId),
    Bool(bool),
    Error(CellError),
}

impl CellValue {
    pub const fn is_blank(&self) -> bool {
        matches!(self, CellValue::Blank)
    }
}

/// Identifies a formula in the sheet's formula arena.
///
/// Non-zero so `Option<FormulaId>` occupies 4 bytes rather than 8 via the null
/// niche. That is 8 bytes off every `Cell`, and the store holds 256 of them per
/// chunk — a 25% cut in the workbook's dominant allocation. Arena indices are
/// therefore one-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FormulaId(std::num::NonZeroU32);

impl FormulaId {
    /// Wraps a one-based arena index. `None` for 0, which is not a valid index.
    pub const fn new(one_based: u32) -> Option<Self> {
        match std::num::NonZeroU32::new(one_based) {
            Some(n) => Some(FormulaId(n)),
            None => None,
        }
    }

    /// From a zero-based arena position.
    pub const fn from_index(index: u32) -> Self {
        match std::num::NonZeroU32::new(index + 1) {
            Some(n) => FormulaId(n),
            None => unreachable!(),
        }
    }

    /// Back to a zero-based arena position.
    pub const fn index(self) -> u32 {
        self.0.get() - 1
    }
}

/// One cell.
///
/// A cell that is visually empty is not necessarily absent: an empty cell with a
/// border or a fill is real content that must round-trip, so `Blank` value and
/// "no cell here" are distinct states.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Cell {
    pub value: CellValue,
    pub style: StyleId,
    /// Present when the value was computed rather than typed.
    pub formula: Option<FormulaId>,
}

impl Cell {
    /// True when this cell carries nothing at all and could be dropped without
    /// changing the document.
    pub fn is_vacant(&self) -> bool {
        self.value.is_blank() && self.formula.is_none() && self.style == StyleId::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_names_use_bijective_base26() {
        // The carry cases are where a plain base-26 loop goes wrong.
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(25), "Z");
        assert_eq!(column_name(26), "AA");
        assert_eq!(column_name(51), "AZ");
        assert_eq!(column_name(52), "BA");
        assert_eq!(column_name(701), "ZZ");
        assert_eq!(column_name(702), "AAA");
        assert_eq!(column_name(MAX_COLS - 1), "XFD");
    }

    #[test]
    fn column_names_round_trip() {
        for col in [0, 1, 25, 26, 27, 51, 52, 701, 702, 703, MAX_COLS - 1] {
            let name = column_name(col);
            assert_eq!(
                column_index(&name),
                Some(col),
                "round trip failed for {name}"
            );
        }
    }

    #[test]
    fn column_index_rejects_out_of_range_and_junk() {
        assert_eq!(column_index("XFE"), None, "one past the last column");
        assert_eq!(column_index("ZZZZZZZZ"), None, "must not overflow");
        assert_eq!(column_index(""), None);
        assert_eq!(column_index("A1"), None);
        assert_eq!(column_index("1"), None);
    }

    #[test]
    fn a1_parsing_handles_absolute_markers_and_case() {
        assert_eq!(CellRef::from_a1("A1"), Some(CellRef::new(0, 0)));
        assert_eq!(CellRef::from_a1("$A$1"), Some(CellRef::new(0, 0)));
        assert_eq!(CellRef::from_a1("$A1"), Some(CellRef::new(0, 0)));
        assert_eq!(CellRef::from_a1("A$1"), Some(CellRef::new(0, 0)));
        assert_eq!(CellRef::from_a1("bc7"), Some(CellRef::new(6, 54)));
        assert_eq!(CellRef::from_a1(" C3 "), Some(CellRef::new(2, 2)));
    }

    #[test]
    fn a1_parsing_rejects_the_out_of_range_and_the_malformed() {
        assert_eq!(CellRef::from_a1("A0"), None, "rows are one-based in A1");
        assert_eq!(CellRef::from_a1("XFE1"), None);
        assert_eq!(CellRef::from_a1("A1048577"), None, "one past the last row");
        assert_eq!(CellRef::from_a1("1A"), None);
        assert_eq!(CellRef::from_a1("A"), None);
        assert_eq!(CellRef::from_a1("1"), None);
        assert_eq!(CellRef::from_a1(""), None);
        assert_eq!(CellRef::from_a1("A1:B2"), None, "a range is not an address");
    }

    #[test]
    fn a1_round_trips_at_the_corners() {
        for cell in [
            CellRef::new(0, 0),
            CellRef::new(MAX_ROWS - 1, MAX_COLS - 1),
            CellRef::new(9, 27),
        ] {
            assert_eq!(CellRef::from_a1(&cell.to_a1()), Some(cell));
        }
        assert_eq!(CellRef::new(1_048_575, 16_383).to_a1(), "XFD1048576");
    }

    #[test]
    fn error_codes_round_trip() {
        for e in [
            CellError::Null,
            CellError::Div0,
            CellError::Value,
            CellError::Ref,
            CellError::Name,
            CellError::Num,
            CellError::NotAvailable,
        ] {
            assert_eq!(CellError::from_code(e.as_str()), Some(e));
        }
    }

    #[test]
    fn a_formatted_empty_cell_is_not_vacant() {
        // Losing these on save would silently strip borders and fills from
        // otherwise-empty cells, which is a visible corruption of the document.
        let styled = Cell {
            value: CellValue::Blank,
            style: StyleId(7),
            formula: None,
        };
        assert!(!styled.is_vacant());
        assert!(Cell::default().is_vacant());
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn cell_stays_small() {
        // The chunked store allocates 256 of these per chunk, so growth here is
        // multiplied across the whole workbook.
        assert_eq!(
            std::mem::size_of::<Cell>(),
            24,
            "Cell grew; check the store's memory math"
        );
        assert_eq!(
            std::mem::size_of::<Option<FormulaId>>(),
            4,
            "the null niche was lost; Option<FormulaId> must stay 4 bytes"
        );
    }

    #[test]
    fn formula_ids_are_one_based_in_the_arena() {
        assert_eq!(FormulaId::from_index(0).index(), 0);
        assert_eq!(FormulaId::from_index(41).index(), 41);
        assert!(FormulaId::new(0).is_none(), "0 is not a valid arena index");
        assert_eq!(FormulaId::new(1).unwrap().index(), 0);
    }
}
