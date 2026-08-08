//! Engineering functions: base conversion, bitwise operations, and the error
//! function.
//!
//! The base-conversion family has one thing worth knowing: **the results are
//! text, and negatives are two's complement in ten places.** `DEC2BIN(-1)` is
//! `"1111111111"`, not `"-1"`, and `BIN2DEC("1111111111")` is −1. The range is
//! ten digits in every base, which is why `DEC2BIN` tops out at 511 and
//! `DEC2HEX` at 2^39−1: the tenth digit is the sign.

use ss_model::CellError;

use crate::ast::Expr;
use crate::eval::Evaluator;
use crate::value::{Operand, Value};

use super::{arity, scalar_args, FnImpl};

pub(super) fn lookup(name: &str) -> Option<FnImpl> {
    Some(match name {
        "DEC2BIN" => |ev: &mut Evaluator, a: &[Expr]| from_decimal(ev, a, 2),
        "DEC2OCT" => |ev: &mut Evaluator, a: &[Expr]| from_decimal(ev, a, 8),
        "DEC2HEX" => |ev: &mut Evaluator, a: &[Expr]| from_decimal(ev, a, 16),
        "BIN2DEC" => |ev: &mut Evaluator, a: &[Expr]| to_decimal(ev, a, 2),
        "OCT2DEC" => |ev: &mut Evaluator, a: &[Expr]| to_decimal(ev, a, 8),
        "HEX2DEC" => |ev: &mut Evaluator, a: &[Expr]| to_decimal(ev, a, 16),
        "BIN2OCT" => |ev: &mut Evaluator, a: &[Expr]| convert_base(ev, a, 2, 8),
        "BIN2HEX" => |ev: &mut Evaluator, a: &[Expr]| convert_base(ev, a, 2, 16),
        "OCT2BIN" => |ev: &mut Evaluator, a: &[Expr]| convert_base(ev, a, 8, 2),
        "OCT2HEX" => |ev: &mut Evaluator, a: &[Expr]| convert_base(ev, a, 8, 16),
        "HEX2BIN" => |ev: &mut Evaluator, a: &[Expr]| convert_base(ev, a, 16, 2),
        "HEX2OCT" => |ev: &mut Evaluator, a: &[Expr]| convert_base(ev, a, 16, 8),
        "BITAND" => |ev: &mut Evaluator, a: &[Expr]| bitwise(ev, a, Bit::And),
        "BITOR" => |ev: &mut Evaluator, a: &[Expr]| bitwise(ev, a, Bit::Or),
        "BITXOR" => |ev: &mut Evaluator, a: &[Expr]| bitwise(ev, a, Bit::Xor),
        "BITLSHIFT" => |ev: &mut Evaluator, a: &[Expr]| bitwise(ev, a, Bit::Left),
        "BITRSHIFT" => |ev: &mut Evaluator, a: &[Expr]| bitwise(ev, a, Bit::Right),
        "DELTA" => delta,
        "GESTEP" => gestep,
        "ERF" => erf_fn,
        "ERFC" => erfc_fn,
        "CONVERT" => convert,
        _ => return None,
    })
}

/// How many digits every base-conversion result has room for, and the sign bit.
const PLACES: u32 = 10;

fn limits(base: u32) -> (i64, i64) {
    // Ten digits, the top one being the sign: 2^39 in hex, 2^29 in octal, 2^9
    // in binary.
    let bits = PLACES * (base as f64).log2() as u32;
    let half = 1i64 << (bits - 1);
    (-half, half - 1)
}

fn from_decimal(ev: &mut Evaluator, args: &[Expr], base: u32) -> Operand {
    if !arity(args, 1, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let values = match scalar_args(ev, args) {
        Ok(v) => v,
        Err(e) => return Operand::error(e),
    };
    let number = match values[0].to_number() {
        Ok(n) => n.trunc() as i64,
        Err(e) => return Operand::error(e),
    };
    let (low, high) = limits(base);
    if number < low || number > high {
        return Operand::error(CellError::Num);
    }

    let text = if number < 0 {
        // Two's complement in exactly ten digits, which is what makes
        // `BIN2DEC("1111111111")` come back as −1.
        let bits = PLACES * (base as f64).log2() as u32;
        let wrapped = (1i128 << bits) + i128::from(number);
        to_base(wrapped as u64, base, PLACES as usize)
    } else {
        to_base(number as u64, base, 0)
    };

    match values.get(1) {
        None | Some(Value::Blank) => Operand::text(text),
        Some(value) => {
            let places = match value.to_number() {
                Ok(p) => p.trunc() as i64,
                Err(e) => return Operand::error(e),
            };
            if places < 0 || places as usize > 10 || (places as usize) < text.len() {
                return Operand::error(CellError::Num);
            }
            // `places` only pads a positive number; a negative one is already
            // ten digits wide and Excel ignores the argument.
            if number < 0 {
                Operand::text(text)
            } else {
                Operand::text(format!("{:0>width$}", text, width = places as usize))
            }
        }
    }
}

fn to_decimal(ev: &mut Evaluator, args: &[Expr], base: u32) -> Operand {
    if !arity(args, 1, Some(1)) {
        return Operand::error(CellError::Value);
    }
    match parse_base(ev, &args[0], base) {
        Ok(n) => Operand::number(n as f64),
        Err(e) => Operand::error(e),
    }
}

fn convert_base(ev: &mut Evaluator, args: &[Expr], from: u32, to: u32) -> Operand {
    if !arity(args, 1, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let number = match parse_base(ev, &args[0], from) {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    // Routed through the decimal form so the two's-complement rule is written
    // once rather than six times.
    let literal = Expr::Number(number as f64);
    let mut rest: Vec<Expr> = vec![literal];
    if let Some(places) = args.get(1) {
        rest.push(places.clone());
    }
    from_decimal(ev, &rest, to)
}

fn parse_base(ev: &mut Evaluator, arg: &Expr, base: u32) -> Result<i64, CellError> {
    let value = ev.eval_scalar(arg);
    let text = match &value {
        Value::Error(e) => return Err(*e),
        Value::Blank => return Ok(0),
        // A number reaches here when the cell holds one: `BIN2DEC(1010)`.
        Value::Number(n) => super::super::value::format_general(*n),
        other => other.to_text()?,
    };
    let text = text.trim();
    if text.is_empty() {
        return Ok(0);
    }
    if text.len() > PLACES as usize {
        return Err(CellError::Num);
    }
    let raw = i64::from_str_radix(text, base).map_err(|_| CellError::Num)?;
    let bits = PLACES * (base as f64).log2() as u32;
    let half = 1i64 << (bits - 1);
    // The top digit is the sign, so anything at or above half the range is
    // negative — which is the whole reason the ten-digit width is fixed.
    Ok(if text.len() == PLACES as usize && raw >= half {
        raw - (half << 1)
    } else {
        raw
    })
}

fn to_base(mut value: u64, base: u32, width: usize) -> String {
    const DIGITS: &[u8] = b"0123456789ABCDEF";
    if value == 0 {
        return "0".to_string();
    }
    let mut out = Vec::new();
    while value > 0 {
        out.push(DIGITS[(value % u64::from(base)) as usize]);
        value /= u64::from(base);
    }
    while out.len() < width {
        out.push(b'0');
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

#[derive(Clone, Copy)]
enum Bit {
    And,
    Or,
    Xor,
    Left,
    Right,
}

/// The bitwise family, which is defined over 48-bit non-negative integers.
fn bitwise(ev: &mut Evaluator, args: &[Expr], op: Bit) -> Operand {
    if !arity(args, 2, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let values = match scalar_args(ev, args) {
        Ok(v) => v,
        Err(e) => return Operand::error(e),
    };
    let mut numbers = [0u64; 2];
    for (index, value) in values.iter().enumerate().take(2) {
        let n = match value.to_number() {
            Ok(n) => n,
            Err(e) => return Operand::error(e),
        };
        if n.fract() != 0.0 || !(0.0..281_474_976_710_656.0).contains(&n) {
            // 2^48. Excel's limit, and the reason is that a double holds it
            // exactly and one bit more would not.
            if matches!(op, Bit::Left | Bit::Right) && index == 1 {
                if n.fract() != 0.0 || n.abs() > 53.0 {
                    return Operand::error(CellError::Num);
                }
            } else {
                return Operand::error(CellError::Num);
            }
        }
        numbers[index] = n.abs() as u64;
    }
    let shift = values
        .get(1)
        .and_then(|v| v.to_number().ok())
        .unwrap_or(0.0);
    let result = match op {
        Bit::And => numbers[0] & numbers[1],
        Bit::Or => numbers[0] | numbers[1],
        Bit::Xor => numbers[0] ^ numbers[1],
        Bit::Left | Bit::Right => {
            let left = matches!(op, Bit::Left) == (shift >= 0.0);
            let by = shift.abs() as u32;
            if by > 53 {
                return Operand::error(CellError::Num);
            }
            if left {
                numbers[0] << by
            } else {
                numbers[0] >> by
            }
        }
    };
    if result >= 281_474_976_710_656 {
        return Operand::error(CellError::Num);
    }
    Operand::number(result as f64)
}

fn delta(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    compare_pair(ev, args, |a, b| a == b)
}

fn gestep(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    compare_pair(ev, args, |a, b| a >= b)
}

fn compare_pair(ev: &mut Evaluator, args: &[Expr], test: fn(f64, f64) -> bool) -> Operand {
    if !arity(args, 1, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let values = match scalar_args(ev, args) {
        Ok(v) => v,
        Err(e) => return Operand::error(e),
    };
    let first = match values[0].to_number() {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    let second = match values.get(1) {
        Some(Value::Blank) | None => 0.0,
        Some(value) => match value.to_number() {
            Ok(n) => n,
            Err(e) => return Operand::error(e),
        },
    };
    Operand::number(f64::from(test(first, second)))
}

fn erf_fn(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let values = match scalar_args(ev, args) {
        Ok(v) => v,
        Err(e) => return Operand::error(e),
    };
    let lower = match values[0].to_number() {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    match values.get(1) {
        None | Some(Value::Blank) => Operand::number(erf(lower)),
        Some(value) => match value.to_number() {
            Ok(upper) => Operand::number(erf(upper) - erf(lower)),
            Err(e) => Operand::error(e),
        },
    }
}

fn erfc_fn(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(1)) {
        return Operand::error(CellError::Value);
    }
    match ev.eval_scalar(&args[0]).to_number() {
        Ok(x) => Operand::number(1.0 - erf(x)),
        Err(e) => Operand::error(e),
    }
}

/// Abramowitz and Stegun 7.1.26, good to about 1.5e-7 — well inside what a
/// spreadsheet displays, and the same approximation Excel's own answer agrees
/// with to the digits anyone sees.
fn erf(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let y = 1.0
        - (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736) * t
            + 0.254_829_592)
            * t
            * (-x * x).exp();
    sign * y
}

/// `CONVERT(number, from, to)` for the units a spreadsheet actually sees.
///
/// Everything is defined against one base unit per dimension, so a conversion
/// is two multiplications and cannot accumulate the rounding a table of pairs
/// would. Units from different dimensions give `#N/A`, which is Excel's answer
/// and is why the dimension is part of the table.
fn convert(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 3, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let values = match scalar_args(ev, args) {
        Ok(v) => v,
        Err(e) => return Operand::error(e),
    };
    let number = match values[0].to_number() {
        Ok(n) => n,
        Err(e) => return Operand::error(e),
    };
    let (from, to) = match (values[1].to_text(), values[2].to_text()) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return Operand::error(e),
    };
    let (Some(from), Some(to)) = (unit(&from), unit(&to)) else {
        return Operand::error(CellError::NotAvailable);
    };
    if from.0 != to.0 {
        return Operand::error(CellError::NotAvailable);
    }
    // Temperature is affine rather than linear, so it cannot go through the
    // same multiply-and-divide as everything else.
    if from.0 == "temperature" {
        let kelvin = number * from.1 + from.2;
        return Operand::number((kelvin - to.2) / to.1);
    }
    Operand::number(number * from.1 / to.1)
}

/// (dimension, factor to the base unit, offset for affine scales).
fn unit(name: &str) -> Option<(&'static str, f64, f64)> {
    Some(match name {
        // Mass, base gram.
        "g" => ("mass", 1.0, 0.0),
        "kg" => ("mass", 1000.0, 0.0),
        "mg" => ("mass", 0.001, 0.0),
        "lbm" => ("mass", 453.592_37, 0.0),
        "ozm" => ("mass", 28.349_523_125, 0.0),
        "stone" => ("mass", 6_350.293_18, 0.0),
        "ton" => ("mass", 907_184.74, 0.0),
        // Distance, base metre.
        "m" => ("distance", 1.0, 0.0),
        "km" => ("distance", 1000.0, 0.0),
        "cm" => ("distance", 0.01, 0.0),
        "mm" => ("distance", 0.001, 0.0),
        "mi" => ("distance", 1609.344, 0.0),
        "Nmi" => ("distance", 1852.0, 0.0),
        "in" => ("distance", 0.0254, 0.0),
        "ft" => ("distance", 0.3048, 0.0),
        "yd" => ("distance", 0.9144, 0.0),
        "ang" => ("distance", 1e-10, 0.0),
        // Time, base second.
        "sec" | "s" => ("time", 1.0, 0.0),
        "mn" | "min" => ("time", 60.0, 0.0),
        "hr" => ("time", 3600.0, 0.0),
        "day" | "d" => ("time", 86400.0, 0.0),
        "yr" => ("time", 31_557_600.0, 0.0),
        // Temperature, base kelvin. `factor` scales and `offset` shifts.
        "K" | "kel" => ("temperature", 1.0, 0.0),
        "C" | "cel" => ("temperature", 1.0, 273.15),
        "F" | "fah" => ("temperature", 5.0 / 9.0, 255.372_222_222_222_2),
        // Energy, base joule.
        "J" => ("energy", 1.0, 0.0),
        "e" => ("energy", 1e-7, 0.0),
        "cal" => ("energy", 4.186_8, 0.0),
        "eV" => ("energy", 1.602_176_634e-19, 0.0),
        "Wh" => ("energy", 3600.0, 0.0),
        "BTU" | "btu" => ("energy", 1_055.055_852_62, 0.0),
        // Pressure, base pascal.
        "Pa" | "p" => ("pressure", 1.0, 0.0),
        "atm" | "at" => ("pressure", 101_325.0, 0.0),
        "mmHg" => ("pressure", 133.322_387_415, 0.0),
        "psi" => ("pressure", 6_894.757_293_168, 0.0),
        _ => return None,
    })
}
