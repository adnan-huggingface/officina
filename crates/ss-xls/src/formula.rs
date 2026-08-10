//! Parsed expressions: BIFF's RPN token stream back into formula text.
//!
//! A .xls does not store `=SUM(A1:A5)`. It stores a postfix stream of *parse
//! thing* tokens — `ptgArea`, `ptgAttr`, `ptgFuncVar` — which is what Excel
//! evaluates directly. Getting text back means running the stream on a stack of
//! strings.
//!
//! Two things make that reconstruction exact rather than approximate. Excel
//! emits `ptgParen` wherever the user typed a bracket, so no precedence
//! analysis is needed and none is done: brackets come back where they were and
//! nowhere else. And every operand token records whether each half of an
//! address was relative, so `$B$4` and `B4` are distinguishable, which they
//! have to be — the difference is invisible until something is copied.
//!
//! Where a token cannot be read with certainty the whole formula is abandoned
//! and the cell keeps the value Excel cached in it. A cell showing the right
//! number with no formula behind it is a limitation; a cell showing a formula
//! that is subtly not the one in the file is a lie.

use ss_model::CellRef;

use crate::func::{self, VAR};
use crate::record::{f64_at, u16_at};

/// What a `FORMULA` record's expression turned out to be.
pub(crate) enum Parsed {
    Text(String),
    /// `ptgExp`: this cell has no expression of its own. It is a follower of a
    /// shared formula, or a cell covered by an array formula, anchored at this
    /// address.
    Follows(CellRef),
    /// A token this reader does not decompile. The cached value stands alone.
    Unreadable,
}

/// What a formula needs from the rest of the workbook to name things.
pub(crate) struct Context<'a> {
    pub sheets: &'a [String],
    /// `EXTERNSHEET`, which is what a 3-D reference's `ixti` indexes:
    /// (supbook, first sheet, last sheet). The sheet numbers are `-1` when the
    /// sheet has since been deleted.
    pub xti: &'a [(u16, i16, i16)],
    /// Which `SUPBOOK` entries are this workbook rather than a linked one.
    pub internal: &'a [bool],
    pub names: &'a [String],
}

pub(crate) fn parse(rgce: &[u8], base: CellRef, ctx: &Context<'_>) -> Parsed {
    // A lone ptgExp is the whole expression; it means "see the other cell".
    if let (Some(0x01), 5) = (rgce.first().copied(), rgce.len()) {
        if let (Some(row), Some(col)) = (u16_at(rgce, 1), u16_at(rgce, 3)) {
            return Parsed::Follows(CellRef::new(row as u32, col as u32));
        }
    }
    match run(rgce, base, ctx) {
        Some(text) => Parsed::Text(text),
        None => Parsed::Unreadable,
    }
}

fn run(rgce: &[u8], base: CellRef, ctx: &Context<'_>) -> Option<String> {
    let mut stack: Vec<String> = Vec::new();
    let mut at = 0usize;

    while at < rgce.len() {
        let ptg = rgce[at];
        at += 1;
        // Operand tokens come in three classes — reference, value, array —
        // which change how Excel evaluates them and not at all how they are
        // written. Fold the class away and there is one token to handle.
        let token = if ptg >= 0x40 {
            (ptg & 0x3F) | 0x20
        } else {
            ptg
        };

        match token {
            0x01 | 0x02 => return None, // ptgExp/ptgTbl inside a larger expression
            0x03..=0x11 => {
                let op = match token {
                    0x03 => "+",
                    0x04 => "-",
                    0x05 => "*",
                    0x06 => "/",
                    0x07 => "^",
                    0x08 => "&",
                    0x09 => "<",
                    0x0A => "<=",
                    0x0B => "=",
                    0x0C => ">=",
                    0x0D => ">",
                    0x0E => "<>",
                    0x0F => " ", // intersection
                    0x10 => ",", // union
                    _ => ":",    // range
                };
                let right = stack.pop()?;
                let left = stack.pop()?;
                stack.push(format!("{left}{op}{right}"));
            }
            0x12 => {
                let value = stack.pop()?;
                stack.push(format!("+{value}"));
            }
            0x13 => {
                let value = stack.pop()?;
                stack.push(format!("-{value}"));
            }
            0x14 => {
                let value = stack.pop()?;
                stack.push(format!("{value}%"));
            }
            0x15 => {
                let value = stack.pop()?;
                stack.push(format!("({value})"));
            }
            0x16 => stack.push(String::new()), // a skipped argument
            0x17 => {
                let (text, used) = crate::string::short(rgce, at)?;
                at += used;
                stack.push(format!("\"{}\"", text.replace('"', "\"\"")));
            }
            0x19 => {
                let grbit = *rgce.get(at)?;
                at += 1;
                if grbit & 0x04 != 0 {
                    // fAttrChoose: a jump table, one offset per case plus one.
                    let cases = u16_at(rgce, at)? as usize;
                    at += 2 + (cases + 1) * 2;
                } else {
                    at += 2;
                }
                if grbit & 0x10 != 0 {
                    // fAttrSum: the one-argument SUM, written as an attribute
                    // rather than as a function call.
                    let value = stack.pop()?;
                    stack.push(format!("SUM({value})"));
                }
            }
            0x1C => {
                let code = *rgce.get(at)?;
                at += 1;
                stack.push(error_code(code).to_string());
            }
            0x1D => {
                let value = *rgce.get(at)?;
                at += 1;
                stack.push(if value == 0 { "FALSE" } else { "TRUE" }.to_string());
            }
            0x1E => {
                stack.push(u16_at(rgce, at)?.to_string());
                at += 2;
            }
            0x1F => {
                stack.push(ss_model::numfmt::format_general(f64_at(rgce, at)?));
                at += 8;
            }
            // ptgArray. The constant itself is not here — it is appended after
            // the token stream, in a section this reader does not walk — so the
            // formula is abandoned rather than written with the array missing.
            0x20 => return None,
            0x21 => {
                let index = u16_at(rgce, at)?;
                at += 2;
                let (name, arity) = func::lookup(index)?;
                if arity == VAR {
                    return None; // see `func`: guessing the count is worse
                }
                let args = pop(&mut stack, arity as usize)?;
                stack.push(format!("{name}({})", args.join(",")));
            }
            0x22 => {
                let count = (*rgce.get(at)? & 0x7F) as usize;
                let index = u16_at(rgce, at + 1)? & 0x7FFF;
                at += 3;
                let mut args = pop(&mut stack, count)?;
                if index == 255 {
                    // A call to something not in the built-in table: the first
                    // operand is the name.
                    if args.is_empty() {
                        return None;
                    }
                    let name = args.remove(0);
                    stack.push(format!("{name}({})", args.join(",")));
                } else {
                    let (name, _) = func::lookup(index)?;
                    stack.push(format!("{name}({})", args.join(",")));
                }
            }
            0x23 => {
                let index = u16_at(rgce, at)? as usize;
                at += 4;
                stack.push(ctx.names.get(index.checked_sub(1)?)?.clone());
            }
            0x24 | 0x2C => {
                let relative = token == 0x2C;
                stack.push(reference(rgce, at, base, relative)?);
                at += 4;
            }
            0x25 | 0x2D => {
                stack.push(area(rgce, at, base, token == 0x2D)?);
                at += 8;
            }
            // Reference subexpressions: a header, then the tokens themselves,
            // which are read as usual. Nothing is pushed for the header.
            0x26..=0x28 => at += 6,
            0x29 | 0x2E | 0x2F => at += 2,
            0x2A => {
                at += 4;
                stack.push("#REF!".to_string());
            }
            0x2B => {
                at += 8;
                stack.push("#REF!".to_string());
            }
            0x39 => {
                // A name qualified by a workbook. Resolving the prefix is what
                // rejects a name defined in a *linked* workbook, whose own name
                // list is not read.
                sheet_prefix(u16_at(rgce, at)?, ctx)?;
                let index = u16_at(rgce, at + 2)? as usize;
                at += 6;
                stack.push(ctx.names.get(index.checked_sub(1)?)?.clone());
            }
            0x3A | 0x3C => {
                let prefix = sheet_prefix(u16_at(rgce, at)?, ctx)?;
                let cell = if token == 0x3C {
                    "#REF!".to_string()
                } else {
                    reference(rgce, at + 2, base, false)?
                };
                at += 6;
                stack.push(format!("{prefix}{cell}"));
            }
            0x3B | 0x3D => {
                let prefix = sheet_prefix(u16_at(rgce, at)?, ctx)?;
                let range = if token == 0x3D {
                    "#REF!".to_string()
                } else {
                    area(rgce, at + 2, base, false)?
                };
                at += 10;
                stack.push(format!("{prefix}{range}"));
            }
            _ => return None,
        }
    }

    // Exactly one value should be left. Anything else means the stream was
    // read wrongly, and a partial formula is worse than none.
    match stack.len() {
        1 => stack.pop(),
        _ => None,
    }
}

fn pop(stack: &mut Vec<String>, count: usize) -> Option<Vec<String>> {
    if stack.len() < count {
        return None;
    }
    Some(stack.split_off(stack.len() - count))
}

/// A `ptgRef`: one row, then one column field.
fn reference(data: &[u8], at: usize, base: CellRef, relative: bool) -> Option<String> {
    Some(address(
        u16_at(data, at)?,
        u16_at(data, at + 2)?,
        base,
        relative,
    ))
}

/// A `ptgArea`, which is **not** two `ptgRef`s: both rows come first and then
/// both columns. Read as a pair of addresses, every range in the workbook comes
/// out with its second row for a column.
fn area(data: &[u8], at: usize, base: CellRef, relative: bool) -> Option<String> {
    let from = address(u16_at(data, at)?, u16_at(data, at + 4)?, base, relative);
    let to = address(u16_at(data, at + 2)?, u16_at(data, at + 6)?, base, relative);
    Some(format!("{from}:{to}"))
}

/// One address, with `$` where the file says the reference was anchored.
///
/// In an ordinary formula the stored numbers are absolute whichever way the
/// anchor flags are set. In a *shared* formula they are offsets from the cell
/// the expression is being read for, which is what `relative` selects.
///
/// The two offsets are signed and they are **not the same width**. A row offset
/// is sixteen bits, because a sheet has 65,536 rows. A column offset is *eight*,
/// because a sheet has 256 columns — so a step one column to the left is `FF`,
/// and sign-extending the fourteen bits the field nominally has reads it as two
/// hundred and fifty-five columns to the right. That produces an address near
/// the far edge of the sheet, which is empty, so the formula comes back
/// well-formed and pointing at nothing.
fn address(row: u16, col_field: u16, base: CellRef, relative: bool) -> String {
    let col_rel = col_field & 0x4000 != 0;
    let row_rel = col_field & 0x8000 != 0;
    let col_bits = col_field & 0x3FFF;

    let row_number = if relative && row_rel {
        ((base.row as i64 + sign_extend(row, 16)) as u32) & 0xFFFF
    } else {
        row as u32
    };
    let col_number = if relative && col_rel {
        ((base.col as i64 + sign_extend(col_bits & 0xFF, 8)) as u32) & 0xFF
    } else {
        col_bits as u32
    };

    format!(
        "{}{}{}{}",
        if col_rel { "" } else { "$" },
        ss_model::cell::column_name(col_number),
        if row_rel { "" } else { "$" },
        row_number + 1
    )
}

fn sign_extend(value: u16, bits: u32) -> i64 {
    let shift = 16 - bits;
    ((value << shift) as i16 >> shift) as i64
}

/// `Sheet1!`, `'Old Data'!`, `Sheet1:Sheet3!`, or `#REF!` for a sheet that has
/// been deleted. `None` when the reference points into another workbook, which
/// this reader does not follow.
fn sheet_prefix(ixti: u16, ctx: &Context<'_>) -> Option<String> {
    let (supbook, first, last) = *ctx.xti.get(ixti as usize)?;
    if !ctx.internal.get(supbook as usize).copied().unwrap_or(false) {
        return None;
    }
    if first < 0 || last < 0 {
        return Some("#REF!".to_string());
    }
    let one = ctx.sheets.get(first as usize)?;
    Some(if first == last {
        format!("{}!", quoted(one))
    } else {
        let other = ctx.sheets.get(last as usize)?;
        format!("{}:{}!", quoted(one), quoted(other))
    })
}

/// Sheet names go in single quotes unless every character is one a bare name
/// may contain; an apostrophe inside is doubled.
fn quoted(name: &str) -> String {
    let plain = !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.');
    if plain {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

pub(crate) fn error_code(code: u8) -> &'static str {
    match code {
        0x00 => "#NULL!",
        0x07 => "#DIV/0!",
        0x0F => "#VALUE!",
        0x17 => "#REF!",
        0x1D => "#NAME?",
        0x24 => "#NUM!",
        0x2A => "#N/A",
        _ => "#N/A",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context<'a>(
        sheets: &'a [String],
        xti: &'a [(u16, i16, i16)],
        internal: &'a [bool],
    ) -> Context<'a> {
        Context {
            sheets,
            xti,
            internal,
            names: &[],
        }
    }

    fn text(rgce: &[u8]) -> Option<String> {
        let sheets = Vec::new();
        let ctx = context(&sheets, &[], &[]);
        match parse(rgce, CellRef::new(0, 0), &ctx) {
            Parsed::Text(t) => Some(t),
            _ => None,
        }
    }

    /// ptgRef for an absolute-or-not address.
    fn ref_token(row: u16, col: u16, row_rel: bool, col_rel: bool) -> Vec<u8> {
        let field = col | if col_rel { 0x4000 } else { 0 } | if row_rel { 0x8000 } else { 0 };
        let mut out = vec![0x24];
        out.extend_from_slice(&row.to_le_bytes());
        out.extend_from_slice(&field.to_le_bytes());
        out
    }

    #[test]
    fn a_sum_over_a_range_comes_back_as_one_wrote_it() {
        // =SUM(A1:B2): ptgArea, then ptgFuncVar with one argument, function 4.
        let mut rgce = vec![0x25];
        rgce.extend_from_slice(&[0, 0, 1, 0, 0x00, 0xC0, 0x01, 0xC0]);
        rgce.extend_from_slice(&[0x22, 1, 4, 0]);
        assert_eq!(text(&rgce).as_deref(), Some("SUM(A1:B2)"));
    }

    #[test]
    fn the_dollar_signs_survive() {
        assert_eq!(
            text(&ref_token(3, 1, false, false)).as_deref(),
            Some("$B$4"),
            "an anchored reference"
        );
        assert_eq!(
            text(&ref_token(3, 1, true, true)).as_deref(),
            Some("B4"),
            "a relative one"
        );
        assert_eq!(
            text(&ref_token(3, 1, false, true)).as_deref(),
            Some("B$4"),
            "anchored by row only"
        );
    }

    #[test]
    fn brackets_come_back_where_they_were_and_nowhere_else() {
        // =(1+2)*3 — the ptgParen is what tells us the user typed brackets.
        let mut rgce = vec![0x1E, 1, 0, 0x1E, 2, 0, 0x03, 0x15];
        rgce.extend_from_slice(&[0x1E, 3, 0, 0x05]);
        assert_eq!(text(&rgce).as_deref(), Some("(1+2)*3"));

        // =1+2*3 has no ptgParen, and must not gain any.
        let rgce = vec![0x1E, 1, 0, 0x1E, 2, 0, 0x1E, 3, 0, 0x05, 0x03];
        assert_eq!(text(&rgce).as_deref(), Some("1+2*3"));
    }

    #[test]
    fn a_string_literal_keeps_its_doubled_quotes() {
        let mut rgce = vec![0x17, 3, 0];
        rgce.extend_from_slice(b"a\"b");
        assert_eq!(text(&rgce).as_deref(), Some("\"a\"\"b\""));
    }

    #[test]
    fn the_one_argument_sum_hidden_in_an_attribute_is_still_a_sum() {
        // ptgArea then ptgAttr with fAttrSum, which is how Excel writes
        // =SUM(A1:A5) with a single range argument.
        let mut rgce = vec![0x25];
        rgce.extend_from_slice(&[0, 0, 4, 0, 0x00, 0xC0, 0x00, 0xC0]);
        rgce.extend_from_slice(&[0x19, 0x10, 0, 0]);
        assert_eq!(text(&rgce).as_deref(), Some("SUM(A1:A5)"));
    }

    #[test]
    fn a_three_dimensional_reference_names_its_sheet() {
        let sheets = vec!["Data".to_string(), "My Sheet".to_string()];
        let xti = [(0u16, 0i16, 0i16), (0, 1, 1), (0, 0, 1), (0, -1, -1)];
        let internal = [true];
        let ctx = context(&sheets, &xti, &internal);
        let token = |ixti: u16| {
            let mut out = vec![0x3A];
            out.extend_from_slice(&ixti.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0xC000u16.to_le_bytes());
            out
        };
        let render = |ixti| match parse(&token(ixti), CellRef::new(0, 0), &ctx) {
            Parsed::Text(t) => t,
            _ => "<unreadable>".to_string(),
        };
        assert_eq!(render(0), "Data!A1");
        assert_eq!(render(1), "'My Sheet'!A1", "a name with a space is quoted");
        assert_eq!(render(2), "Data:'My Sheet'!A1");
        assert_eq!(render(3), "#REF!A1", "the sheet was deleted");
    }

    #[test]
    fn a_reference_into_another_workbook_gives_up_rather_than_guessing() {
        let sheets = vec!["Data".to_string()];
        let ctx = context(&sheets, &[(1, 0, 0)], &[true, false]);
        let mut token = vec![0x3A];
        token.extend_from_slice(&[0, 0, 0, 0, 0x00, 0xC0]);
        assert!(matches!(
            parse(&token, CellRef::new(0, 0), &ctx),
            Parsed::Unreadable
        ));
    }

    #[test]
    fn a_shared_formulas_offsets_are_resolved_against_the_cell_reading_it() {
        // ptgRefN with a relative offset of -1 row, 0 columns, read for C5.
        let mut rgce = vec![0x2C];
        rgce.extend_from_slice(&(-1i16 as u16).to_le_bytes());
        rgce.extend_from_slice(&0xC000u16.to_le_bytes());
        let sheets = Vec::new();
        let ctx = context(&sheets, &[], &[]);
        let out = match parse(&rgce, CellRef::new(4, 2), &ctx) {
            Parsed::Text(t) => t,
            _ => "<unreadable>".to_string(),
        };
        assert_eq!(out, "C4");
    }

    #[test]
    fn a_shared_formulas_column_offset_is_eight_bits_and_the_rows_sixteen() {
        // Taken from a workbook Microsoft ships: a `ptgRefN` of FF C0 with a
        // row offset of 7, read for C11, is `B18` — one column *left*. Read as
        // a fourteen-bit offset it is 255 columns right, which lands in the far
        // corner of the sheet where there is nothing, so the formula comes back
        // looking perfectly reasonable and pointing at a blank.
        let mut rgce = vec![0x4C];
        rgce.extend_from_slice(&7u16.to_le_bytes());
        rgce.extend_from_slice(&0xC0FFu16.to_le_bytes());
        let sheets = Vec::new();
        let ctx = context(&sheets, &[], &[]);
        let out = match parse(&rgce, CellRef::new(10, 2), &ctx) {
            Parsed::Text(t) => t,
            _ => "<unreadable>".to_string(),
        };
        assert_eq!(out, "B18");
    }

    #[test]
    fn a_cell_that_only_points_at_another_says_so() {
        let rgce = vec![0x01, 2, 0, 1, 0];
        let sheets = Vec::new();
        let ctx = context(&sheets, &[], &[]);
        match parse(&rgce, CellRef::new(9, 9), &ctx) {
            Parsed::Follows(at) => assert_eq!(at, CellRef::new(2, 1)),
            _ => panic!("a ptgExp is not an expression"),
        }
    }

    #[test]
    fn an_unreadable_token_abandons_the_whole_formula() {
        // ptgArray: the constant lives outside the token stream.
        let rgce = vec![0x20, 0, 0, 0, 0, 0, 0, 0];
        assert!(text(&rgce).is_none());
    }
}
