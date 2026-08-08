//! Date and time functions.
//!
//! Everything here runs on serial numbers; [`crate::datetime`] holds the
//! conversion and the 1900 leap-year bug it has to reproduce. What is left is
//! Excel's own conventions on top of the calendar, and they are not derivable:
//! `DAYS` takes its arguments end-first, `DATEDIF` is undocumented, `WEEKNUM`
//! has eleven numbering schemes, and `YEARFRAC` has five day-count bases of
//! which the first is a 30/360 convention with a special case for February.

use ss_model::CellError;

use crate::ast::Expr;
use crate::eval::Evaluator;
use crate::value::{text_to_number, Operand, Value};
use ss_model::datetime::{self, days_in_month, from_serial, DateTime};

use super::{arity, visit_args, FnImpl, Source};

pub(super) fn lookup(name: &str) -> Option<FnImpl> {
    Some(match name {
        "DATE" => date,
        "TIME" => time,
        "TODAY" => |ev: &mut Evaluator, a: &[Expr]| {
            if arity(a, 0, Some(0)) {
                Operand::number(ev.context().now().floor())
            } else {
                Operand::error(CellError::Value)
            }
        },
        "NOW" => |ev: &mut Evaluator, a: &[Expr]| {
            if arity(a, 0, Some(0)) {
                Operand::number(ev.context().now())
            } else {
                Operand::error(CellError::Value)
            }
        },
        "YEAR" => |ev: &mut Evaluator, a: &[Expr]| part(ev, a, |d| f64::from(d.year)),
        "MONTH" => |ev: &mut Evaluator, a: &[Expr]| part(ev, a, |d| f64::from(d.month)),
        "DAY" => |ev: &mut Evaluator, a: &[Expr]| part(ev, a, |d| f64::from(d.day)),
        "HOUR" => |ev: &mut Evaluator, a: &[Expr]| part(ev, a, |d| f64::from(d.hour)),
        "MINUTE" => |ev: &mut Evaluator, a: &[Expr]| part(ev, a, |d| f64::from(d.minute)),
        "SECOND" => |ev: &mut Evaluator, a: &[Expr]| part(ev, a, |d| f64::from(d.second)),
        "WEEKDAY" => weekday,
        "WEEKNUM" => weeknum,
        "ISOWEEKNUM" => isoweeknum,
        "EDATE" => |ev: &mut Evaluator, a: &[Expr]| shift_months(ev, a, false),
        "EOMONTH" => |ev: &mut Evaluator, a: &[Expr]| shift_months(ev, a, true),
        "DAYS" => days,
        "DAYS360" => days360,
        "YEARFRAC" => yearfrac,
        "DATEDIF" => datedif,
        "DATEVALUE" => datevalue,
        "TIMEVALUE" => timevalue,
        "NETWORKDAYS" => |ev: &mut Evaluator, a: &[Expr]| networkdays(ev, a, false),
        "NETWORKDAYS.INTL" => |ev: &mut Evaluator, a: &[Expr]| networkdays(ev, a, true),
        "WORKDAY" => |ev: &mut Evaluator, a: &[Expr]| workday(ev, a, false),
        "WORKDAY.INTL" => |ev: &mut Evaluator, a: &[Expr]| workday(ev, a, true),
        _ => return None,
    })
}

/// Coerces an argument to a serial number.
///
/// Text is tried as a number first and then as a written date, so
/// `YEAR("2024-03-01")` works — Excel accepts date text anywhere a serial is
/// wanted. A negative serial is `#NUM!`: there is no date before 1900.
fn serial_arg(ev: &mut Evaluator, expr: &Expr) -> Result<f64, CellError> {
    let v = ev.eval_scalar(expr);
    serial_of(&v)
}

fn serial_of(v: &Value) -> Result<f64, CellError> {
    let n = match v {
        Value::Text(s) => match text_to_number(s).or_else(|| parse_datetime_text(s)) {
            Some(n) => n,
            None => return Err(CellError::Value),
        },
        // A boolean is not a date, even though it is a number everywhere else.
        Value::Bool(_) => return Err(CellError::Value),
        other => other.to_number()?,
    };
    if !(0.0..=datetime::MAX_SERIAL + 1.0).contains(&n) {
        return Err(CellError::Num);
    }
    Ok(n)
}

/// The whole-day part of a serial, which is what the date-only functions use.
fn day_arg(ev: &mut Evaluator, expr: &Expr) -> Result<f64, CellError> {
    serial_arg(ev, expr).map(f64::trunc)
}

fn part(ev: &mut Evaluator, args: &[Expr], f: impl Fn(&DateTime) -> f64) -> Operand {
    if !arity(args, 1, Some(1)) {
        return Operand::error(CellError::Value);
    }
    match serial_arg(ev, &args[0]) {
        Ok(serial) => match from_serial(serial) {
            Some(d) => Operand::number(f(&d)),
            None => Operand::error(CellError::Num),
        },
        Err(e) => Operand::error(e),
    }
}

/// `DATE(year, month, day)`.
///
/// The year rule is Excel's, not the calendar's: a year below 1900 is an
/// *offset* from 1900, so `DATE(24,1,1)` is 1 January 1924 and not 24 AD. Month
/// and day are offsets too — `DATE(2024,13,1)` is January 2025 and
/// `DATE(2024,3,0)` is the last day of February.
fn date(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 3, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let (year, month, day) = match (
        ev.eval_number(&args[0]),
        ev.eval_number(&args[1]),
        ev.eval_number(&args[2]),
    ) {
        (Ok(y), Ok(m), Ok(d)) => (y.trunc(), m.trunc(), d.trunc()),
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => return Operand::error(e),
    };
    if !(0.0..10_000.0).contains(&year) {
        return Operand::error(CellError::Num);
    }
    let year = year as i32 + if year < 1900.0 { 1900 } else { 0 };
    match datetime::to_serial_normalized(year, month as i64, day as i64) {
        Some(s) => Operand::number(s),
        None => Operand::error(CellError::Num),
    }
}

/// `TIME(hour, minute, second)` — a fraction of a day.
///
/// Hours past 24 wrap rather than overflow: `TIME(27,0,0)` is 3 AM, because the
/// result is a time of day and has nowhere to put the extra day.
fn time(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 3, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let (h, m, s) = match (
        ev.eval_number(&args[0]),
        ev.eval_number(&args[1]),
        ev.eval_number(&args[2]),
    ) {
        (Ok(h), Ok(m), Ok(s)) => (h.trunc(), m.trunc(), s.trunc()),
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => return Operand::error(e),
    };
    let seconds = h * 3_600.0 + m * 60.0 + s;
    if seconds < 0.0 {
        return Operand::error(CellError::Num);
    }
    Operand::number((seconds / 86_400.0) % 1.0)
}

/// `WEEKDAY(serial, [type])`. Eleven numbering schemes, because the first three
/// were not enough and 2010 added a set that starts each day at 1.
fn weekday(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let serial = match day_arg(ev, &args[0]) {
        Ok(s) => s,
        Err(e) => return Operand::error(e),
    };
    let kind = match optional_number(ev, args.get(1), 1.0) {
        Ok(n) => n.trunc() as i64,
        Err(e) => return Operand::error(e),
    };
    // 0 = Sunday.
    let dow = i64::from(datetime::weekday_from_serial(serial));
    let n = match kind {
        1 => dow + 1,
        2 => (dow + 6) % 7 + 1,
        3 => (dow + 6) % 7,
        // 11..=17 start the week on Monday..Sunday respectively.
        11..=17 => (dow + 7 - (kind - 10) % 7) % 7 + 1,
        _ => return Operand::error(CellError::Num),
    };
    Operand::number(n as f64)
}

/// `WEEKNUM(serial, [type])` — the week containing 1 January is week 1, and the
/// type says which day starts a week. Type 21 is the ISO scheme instead.
fn weeknum(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let serial = match day_arg(ev, &args[0]) {
        Ok(s) => s,
        Err(e) => return Operand::error(e),
    };
    let kind = match optional_number(ev, args.get(1), 1.0) {
        Ok(n) => n.trunc() as i64,
        Err(e) => return Operand::error(e),
    };
    if kind == 21 {
        return iso_week(serial);
    }
    // Which weekday the week starts on, as 0 = Sunday.
    let start = match kind {
        1 => 0,
        2 => 1,
        11..=17 => (kind - 10) % 7,
        _ => return Operand::error(CellError::Num),
    };
    let Some(d) = from_serial(serial) else {
        return Operand::error(CellError::Num);
    };
    let Some(jan1) = datetime::to_serial(d.year, 1, 1) else {
        return Operand::error(CellError::Num);
    };
    let offset = (i64::from(datetime::weekday_from_serial(jan1)) + 7 - start) % 7;
    let day_of_year = (serial - jan1) as i64;
    Operand::number(((day_of_year + offset) / 7 + 1) as f64)
}

fn isoweeknum(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 1, Some(1)) {
        return Operand::error(CellError::Value);
    }
    match day_arg(ev, &args[0]) {
        Ok(s) => iso_week(s),
        Err(e) => Operand::error(e),
    }
}

/// The ISO 8601 week number: weeks start on Monday, and week 1 is the one
/// holding the first Thursday of the year.
///
/// The Thursday rule is what makes early January sometimes belong to the
/// previous year's week 52 or 53, which no amount of arithmetic on 1 January
/// will reproduce.
fn iso_week(serial: f64) -> Operand {
    // Days since Monday, so shifting to the week's Thursday is a fixed step.
    let dow = (i64::from(datetime::weekday_from_serial(serial)) + 6) % 7;
    let thursday = serial - dow as f64 + 3.0;
    let Some(d) = from_serial(thursday) else {
        return Operand::error(CellError::Num);
    };
    let Some(jan1) = datetime::to_serial(d.year, 1, 1) else {
        return Operand::error(CellError::Num);
    };
    Operand::number(((thursday - jan1) as i64 / 7 + 1) as f64)
}

/// `EDATE` and `EOMONTH` — the same month arithmetic, differing only in whether
/// the day is kept or pushed to the end of the month.
///
/// A day that does not exist in the target month is clamped: `EDATE("2024-01-31",1)`
/// is 29 February, not 2 March.
fn shift_months(ev: &mut Evaluator, args: &[Expr], to_end: bool) -> Operand {
    if !arity(args, 2, Some(2)) {
        return Operand::error(CellError::Value);
    }
    let serial = match day_arg(ev, &args[0]) {
        Ok(s) => s,
        Err(e) => return Operand::error(e),
    };
    let months = match ev.eval_number(&args[1]) {
        Ok(n) => n.trunc() as i64,
        Err(e) => return Operand::error(e),
    };
    let Some(d) = from_serial(serial) else {
        return Operand::error(CellError::Num);
    };

    let total = i64::from(d.year) * 12 + i64::from(d.month) - 1 + months;
    let (year, month) = (total.div_euclid(12), total.rem_euclid(12) + 1);
    let Ok(year) = i32::try_from(year) else {
        return Operand::error(CellError::Num);
    };
    let last = days_in_month(year, month as u32);
    let day = if to_end { last } else { d.day.min(last) };

    match datetime::to_serial_normalized(year, month, i64::from(day)) {
        Some(s) => Operand::number(s),
        None => Operand::error(CellError::Num),
    }
}

/// `DAYS(end, start)`. The end date comes **first**, unlike every other
/// two-date function here.
fn days(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, Some(2)) {
        return Operand::error(CellError::Value);
    }
    match (day_arg(ev, &args[0]), day_arg(ev, &args[1])) {
        (Ok(end), Ok(start)) => Operand::number(end - start),
        (Err(e), _) | (_, Err(e)) => Operand::error(e),
    }
}

/// `DAYS360(start, end, [european])` — a year of twelve thirty-day months, from
/// bond markets that wanted every month's interest to be the same.
fn days360(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let (start, end) = match (day_arg(ev, &args[0]), day_arg(ev, &args[1])) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return Operand::error(e),
    };
    let european = match args.get(2) {
        Some(e) => match ev.eval_bool(e) {
            Ok(b) => b,
            Err(e) => return Operand::error(e),
        },
        None => false,
    };
    let (Some(a), Some(b)) = (from_serial(start), from_serial(end)) else {
        return Operand::error(CellError::Num);
    };

    let (mut d1, mut d2) = (a.day, b.day);
    let (mut m2, mut y2) = (b.month, b.year);
    if european {
        d1 = d1.min(30);
        d2 = d2.min(30);
    } else {
        // The US convention. Note that only day 31 triggers it: 28 February is
        // the last day of its month and is still counted as the 28th.
        if d1 == 31 {
            d1 = 30;
        }
        if d2 == 31 {
            if d1 == 30 {
                d2 = 30;
            } else {
                // Past the 30th of a month that has no 31st equivalent, so the
                // end rolls forward to the first of the next.
                d2 = 1;
                m2 += 1;
                if m2 > 12 {
                    m2 = 1;
                    y2 += 1;
                }
            }
        }
    }
    let total = i64::from(y2 - a.year) * 360
        + (i64::from(m2) - i64::from(a.month)) * 30
        + (i64::from(d2) - i64::from(d1));
    Operand::number(total as f64)
}

/// `YEARFRAC(start, end, [basis])` — the fraction of a year between two dates,
/// under one of five day-count conventions.
fn yearfrac(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 2, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let (mut start, mut end) = match (day_arg(ev, &args[0]), day_arg(ev, &args[1])) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return Operand::error(e),
    };
    let basis = match optional_number(ev, args.get(2), 0.0) {
        Ok(n) => n.trunc() as i64,
        Err(e) => return Operand::error(e),
    };
    // The result is a magnitude; the arguments in the other order give the same
    // answer rather than a negative one.
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }
    let (Some(a), Some(b)) = (from_serial(start), from_serial(end)) else {
        return Operand::error(CellError::Num);
    };

    let fraction = match basis {
        0 => thirty_360(&a, &b, false) / 360.0,
        1 => (end - start) / actual_year_length(&a, &b, start, end),
        2 => (end - start) / 360.0,
        3 => (end - start) / 365.0,
        4 => thirty_360(&a, &b, true) / 360.0,
        _ => return Operand::error(CellError::Num),
    };
    Operand::number(fraction)
}

/// Days between two dates on a 30/360 basis.
///
/// The US variant's February rule is the part that surprises: the last day of
/// February counts as the 30th, so a bond from 28 February to 31 August accrues
/// exactly half a year.
fn thirty_360(a: &DateTime, b: &DateTime, european: bool) -> f64 {
    let (mut d1, mut d2) = (a.day, b.day);
    if european {
        d1 = d1.min(30);
        d2 = d2.min(30);
    } else {
        let end_of_feb = |d: &DateTime| d.month == 2 && d.day == days_in_month(d.year, 2);
        if end_of_feb(a) && end_of_feb(b) {
            d2 = 30;
        }
        if end_of_feb(a) {
            d1 = 30;
        }
        if d2 == 31 && d1 >= 30 {
            d2 = 30;
        }
        if d1 == 31 {
            d1 = 30;
        }
    }
    (i64::from(b.year - a.year) * 360
        + (i64::from(b.month) - i64::from(a.month)) * 30
        + (i64::from(d2) - i64::from(d1))) as f64
}

/// The denominator for basis 1, which is "actual/actual" and therefore has to
/// decide what a year is.
fn actual_year_length(a: &DateTime, b: &DateTime, start: f64, end: f64) -> f64 {
    if a.year == b.year {
        return if datetime::is_leap(a.year) {
            366.0
        } else {
            365.0
        };
    }
    // Under a year but spanning New Year: 366 only if a real 29 February falls
    // inside the interval.
    if b.year == a.year + 1 && (b.month, b.day) <= (a.month, a.day) {
        for year in [a.year, b.year] {
            if let Some(leap_day) = datetime::to_serial(year, 2, 29) {
                if datetime::is_leap(year) && (start..=end).contains(&leap_day) {
                    return 366.0;
                }
            }
        }
        return 365.0;
    }
    // Longer than a year: the average length of the calendar years it touches.
    let years = i64::from(b.year - a.year) + 1;
    let total: i64 = (a.year..=b.year)
        .map(|y| if datetime::is_leap(y) { 366 } else { 365 })
        .sum();
    total as f64 / years as f64
}

/// `DATEDIF(start, end, unit)` — undocumented by Microsoft for thirty years and
/// present in every version, because Lotus had it.
///
/// The three two-letter units are the interesting ones: they measure a component
/// while ignoring the larger ones, which is how you get "3 years, 2 months and
/// 5 days" out of three calls.
fn datedif(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    if !arity(args, 3, Some(3)) {
        return Operand::error(CellError::Value);
    }
    let (start, end) = match (day_arg(ev, &args[0]), day_arg(ev, &args[1])) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return Operand::error(e),
    };
    let unit = match ev.eval_text(&args[2]) {
        Ok(u) => u.to_ascii_uppercase(),
        Err(e) => return Operand::error(e),
    };
    // Excel refuses a reversed interval rather than reporting a negative one.
    if start > end {
        return Operand::error(CellError::Num);
    }
    let (Some(a), Some(b)) = (from_serial(start), from_serial(end)) else {
        return Operand::error(CellError::Num);
    };

    // Whole months between the two dates, which the year and month units are
    // both built from.
    let mut months = i64::from(b.year - a.year) * 12 + i64::from(b.month) - i64::from(a.month);
    if b.day < a.day {
        months -= 1;
    }

    let out = match unit.as_str() {
        "Y" => (months / 12) as f64,
        "M" => months as f64,
        "D" => end - start,
        // Days, ignoring months and years: the day-of-month difference, borrowing
        // from the month before the end date when it goes negative.
        "MD" => {
            if b.day >= a.day {
                f64::from(b.day - a.day)
            } else {
                let previous = if b.month == 1 { 12 } else { b.month - 1 };
                let year = if b.month == 1 { b.year - 1 } else { b.year };
                f64::from(days_in_month(year, previous) - a.day + b.day)
            }
        }
        // Months, ignoring years.
        "YM" => (months % 12) as f64,
        // Days, ignoring years: the end date moved back to the start's year.
        "YD" => {
            let same_year = if (b.month, b.day) >= (a.month, a.day) {
                a.year
            } else {
                a.year + 1
            };
            let Some(anchor) =
                datetime::to_serial_normalized(same_year, i64::from(b.month), i64::from(b.day))
            else {
                return Operand::error(CellError::Num);
            };
            let Some(from) = datetime::to_serial(a.year, a.month, a.day) else {
                return Operand::error(CellError::Num);
            };
            anchor - from
        }
        _ => return Operand::error(CellError::Num),
    };
    Operand::number(out)
}

fn datevalue(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    text_to_serial(ev, args, f64::trunc)
}

fn timevalue(ev: &mut Evaluator, args: &[Expr]) -> Operand {
    // `TIMEVALUE("2024-01-01 13:00")` is the time alone; the date is discarded.
    text_to_serial(ev, args, |s| s - s.trunc())
}

fn text_to_serial(ev: &mut Evaluator, args: &[Expr], f: impl Fn(f64) -> f64) -> Operand {
    if !arity(args, 1, Some(1)) {
        return Operand::error(CellError::Value);
    }
    let text = match ev.eval_text(&args[0]) {
        Ok(t) => t,
        Err(e) => return Operand::error(e),
    };
    match parse_datetime_text(&text) {
        Some(serial) => Operand::number(f(serial)),
        None => Operand::error(CellError::Value),
    }
}

/// Which weekdays do not count as working days, as a bit per day with Sunday at
/// bit 0.
fn weekend_mask(v: &Value) -> Option<u8> {
    if let Value::Text(s) = v {
        // A seven-character mask, Monday first, `1` for a day off.
        let bytes = s.as_bytes();
        if bytes.len() != 7 || !bytes.iter().all(|b| matches!(b, b'0' | b'1')) {
            return None;
        }
        // All seven off would leave no working days at all, which Excel rejects.
        if bytes.iter().all(|&b| b == b'1') {
            return None;
        }
        let mut mask = 0u8;
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'1' {
                // Position 0 is Monday; bit 0 is Sunday.
                mask |= 1 << ((i + 1) % 7);
            }
        }
        return Some(mask);
    }
    let code = v.to_number().ok()?.trunc() as i64;
    Some(match code {
        // Pairs of consecutive days, starting Saturday/Sunday.
        1..=7 => {
            let first = (code + 5) % 7; // 1 -> Saturday
            (1 << first) | (1 << ((first + 1) % 7))
        }
        // Single days, starting Sunday.
        11..=17 => 1 << (code - 11),
        _ => return None,
    })
}

const DEFAULT_WEEKEND: u8 = 0b0100_0001; // Saturday and Sunday

/// Reads the optional trailing arguments the `NETWORKDAYS`/`WORKDAY` family
/// shares: a weekend specification for the `.INTL` forms, then holidays.
fn working_days_options(
    ev: &mut Evaluator,
    args: &[Expr],
    intl: bool,
) -> Result<(u8, Vec<f64>), CellError> {
    let mut mask = DEFAULT_WEEKEND;
    let mut rest = &args[2..];
    if intl {
        if let Some(first) = rest.first() {
            if !matches!(first, Expr::Missing) {
                let v = ev.eval_scalar(first);
                if let Value::Error(e) = v {
                    return Err(e);
                }
                mask = weekend_mask(&v).ok_or(CellError::Num)?;
            }
            rest = &rest[1..];
        }
    }

    let mut holidays = Vec::new();
    let mut err = None;
    visit_args(ev, rest, &mut |v, source| {
        if err.is_some() {
            return;
        }
        match (v, source) {
            (Value::Error(e), _) => err = Some(*e),
            // Blanks inside a holiday range are empty cells, not 0 January 1900.
            (Value::Blank, Source::Inside) => {}
            _ => match serial_of(v) {
                Ok(s) => holidays.push(s.trunc()),
                Err(e) => err = Some(e),
            },
        }
    });
    match err {
        Some(e) => Err(e),
        None => Ok((mask, holidays)),
    }
}

fn is_working(serial: f64, mask: u8, holidays: &[f64]) -> bool {
    let dow = datetime::weekday_from_serial(serial);
    mask & (1 << dow) == 0 && !holidays.contains(&serial)
}

/// `NETWORKDAYS(start, end, [holidays])` — working days in a closed interval,
/// counting both endpoints.
fn networkdays(ev: &mut Evaluator, args: &[Expr], intl: bool) -> Operand {
    if !arity(args, 2, None) {
        return Operand::error(CellError::Value);
    }
    let (start, end) = match (day_arg(ev, &args[0]), day_arg(ev, &args[1])) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return Operand::error(e),
    };
    let (mask, holidays) = match working_days_options(ev, args, intl) {
        Ok(o) => o,
        Err(e) => return Operand::error(e),
    };

    // A reversed interval counts backwards rather than erroring.
    let sign = if end < start { -1.0 } else { 1.0 };
    let (from, to) = if sign < 0.0 {
        (end, start)
    } else {
        (start, end)
    };
    let mut count = 0.0;
    let mut day = from;
    while day <= to {
        if is_working(day, mask, &holidays) {
            count += 1.0;
        }
        day += 1.0;
    }
    Operand::number(sign * count)
}

/// `WORKDAY(start, days, [holidays])` — the date that many working days away.
fn workday(ev: &mut Evaluator, args: &[Expr], intl: bool) -> Operand {
    if !arity(args, 2, None) {
        return Operand::error(CellError::Value);
    }
    let start = match day_arg(ev, &args[0]) {
        Ok(s) => s,
        Err(e) => return Operand::error(e),
    };
    let count = match ev.eval_number(&args[1]) {
        Ok(n) => n.trunc(),
        Err(e) => return Operand::error(e),
    };
    let (mask, holidays) = match working_days_options(ev, args, intl) {
        Ok(o) => o,
        Err(e) => return Operand::error(e),
    };
    let step = if count < 0.0 { -1.0 } else { 1.0 };
    let mut remaining = count.abs();
    let mut day = start;
    while remaining > 0.0 {
        day += step;
        if !(0.0..=datetime::MAX_SERIAL).contains(&day) {
            return Operand::error(CellError::Num);
        }
        if is_working(day, mask, &holidays) {
            remaining -= 1.0;
        }
    }
    Operand::number(day)
}

fn optional_number(ev: &mut Evaluator, arg: Option<&Expr>, default: f64) -> Result<f64, CellError> {
    match arg {
        None | Some(Expr::Missing) => Ok(default),
        Some(e) => ev.eval_number(e),
    }
}

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// Parses a written date, a written time, or both.
///
/// Deliberately narrow. Excel's own parser is locale-driven and will read almost
/// anything the user's regional settings describe; matching that needs the
/// locale plumbing that arrives with C11's number formats. What is here covers
/// ISO dates, the two slash orders, and spelled month names — which is what
/// files actually contain — and refuses the rest rather than guessing wrong.
fn parse_datetime_text(text: &str) -> Option<f64> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    // A date and a time can be written together, separated by space or `T`.
    let (date_part, time_part) = split_date_and_time(t);

    let date = match date_part {
        Some(d) => Some(parse_date(d)?),
        None => None,
    };
    let time = match time_part {
        Some(t) => Some(parse_time(t)?),
        None => None,
    };
    match (date, time) {
        (Some(d), Some(t)) => Some(d + t),
        (Some(d), None) => Some(d),
        // A bare time is a serial below 1, which is how Excel stores one.
        (None, Some(t)) => Some(t),
        (None, None) => None,
    }
}

/// Splits "2024-01-01 13:00" into its two halves. A colon is what marks a time.
fn split_date_and_time(t: &str) -> (Option<&str>, Option<&str>) {
    let Some(colon) = t.find(':') else {
        return (Some(t), None);
    };
    // Walk back to the separator before the hour.
    match t[..colon].rfind([' ', 'T', 't']) {
        Some(cut) if !t[..cut].trim().is_empty() => {
            (Some(t[..cut].trim()), Some(t[cut + 1..].trim()))
        }
        _ => (None, Some(t)),
    }
}

fn parse_date(text: &str) -> Option<f64> {
    let fields: Vec<&str> = text
        .split(['-', '/', ',', ' ', '.'])
        .filter(|f| !f.is_empty())
        .collect();
    if fields.len() < 2 || fields.len() > 3 {
        return None;
    }

    // A spelled month can appear in any of the three positions.
    let named = fields.iter().position(|f| month_number(f).is_some());
    let (year, month, day) = match named {
        Some(index) => {
            let month = month_number(fields[index])?;
            let rest: Vec<u32> = fields
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != index)
                .map(|(_, f)| f.parse::<u32>().ok())
                .collect::<Option<_>>()?;
            match rest.as_slice() {
                // "Mar 2024" — the first of the month.
                [only] if *only > 31 => (*only, month, 1),
                [only] => (current_century(*only), month, 1),
                // "1 Mar 2024" and "Mar 1, 2024" both land here.
                [a, b] if *b > 31 => (*b, month, *a),
                [a, b] => (current_century(*b), month, *a),
                _ => return None,
            }
        }
        None => {
            let n: Vec<u32> = fields
                .iter()
                .map(|f| f.parse::<u32>().ok())
                .collect::<Option<_>>()?;
            match n.as_slice() {
                // ISO: a four-digit year leads.
                [y, m, d] if *y > 31 => (*y, *m, *d),
                // Otherwise month-first, which is what Excel writes on a US
                // machine and what nearly every file in the wild contains.
                [m, d, y] => (current_century(*y), *m, *d),
                _ => return None,
            }
        }
    };
    if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year as i32, month) {
        return None;
    }
    datetime::to_serial(year as i32, month, day)
}

/// Excel's two-digit year rule: 00–29 are this century, 30–99 the last.
fn current_century(year: u32) -> u32 {
    match year {
        0..=29 => 2000 + year,
        30..=99 => 1900 + year,
        _ => year,
    }
}

fn month_number(field: &str) -> Option<u32> {
    let f = field.trim_end_matches('.').to_ascii_lowercase();
    if f.len() < 3 {
        return None;
    }
    MONTHS
        .iter()
        .position(|m| m.starts_with(&f) && f.len() <= m.len())
        .map(|i| i as u32 + 1)
}

fn parse_time(text: &str) -> Option<f64> {
    let t = text.trim().to_ascii_uppercase();
    let (body, meridiem) = if let Some(b) = t.strip_suffix("AM") {
        (b.trim(), Some(false))
    } else if let Some(b) = t.strip_suffix("PM") {
        (b.trim(), Some(true))
    } else {
        (t.as_str(), None)
    };

    let fields: Vec<&str> = body.split(':').collect();
    if fields.len() < 2 || fields.len() > 3 {
        return None;
    }
    let hour: f64 = fields[0].trim().parse().ok()?;
    let minute: f64 = fields[1].trim().parse().ok()?;
    let second: f64 = match fields.get(2) {
        Some(s) => s.trim().parse().ok()?,
        None => 0.0,
    };
    if minute >= 60.0 || second >= 60.0 || hour < 0.0 || minute < 0.0 || second < 0.0 {
        return None;
    }
    let hour = match meridiem {
        // 12 AM is midnight and 12 PM is noon, which is the one case where the
        // twelve-hour clock does not simply add twelve.
        Some(pm) => {
            if hour > 12.0 {
                return None;
            }
            (hour % 12.0) + if pm { 12.0 } else { 0.0 }
        }
        None => hour,
    };
    Some((hour * 3_600.0 + minute * 60.0 + second) / 86_400.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_dates_parse_in_the_orders_files_use() {
        let expect = datetime::to_serial(2024, 3, 1).unwrap();
        for text in [
            "2024-03-01",
            "3/1/2024",
            "3-1-2024",
            "1 March 2024",
            "1-Mar-2024",
            "March 1, 2024",
            "Mar 1 2024",
        ] {
            assert_eq!(parse_datetime_text(text), Some(expect), "{text}");
        }
    }

    #[test]
    fn two_digit_years_split_at_thirty() {
        assert_eq!(
            parse_datetime_text("1/1/29"),
            datetime::to_serial(2029, 1, 1)
        );
        assert_eq!(
            parse_datetime_text("1/1/30"),
            datetime::to_serial(1930, 1, 1)
        );
    }

    #[test]
    fn times_parse_with_and_without_a_meridiem() {
        assert_eq!(parse_time("12:00"), Some(0.5));
        // The twelve-hour clock's one irregularity: 12 AM is midnight.
        assert_eq!(parse_time("12:00 AM"), Some(0.0));
        assert_eq!(parse_time("12:00 PM"), Some(0.5));
        assert_eq!(parse_time("1:30 PM"), Some((13.5 * 3600.0) / 86400.0));
        assert_eq!(parse_time("00:00:30"), Some(30.0 / 86400.0));
        assert_eq!(parse_time("13:00 PM"), None);
        assert_eq!(parse_time("1:60"), None);
    }

    #[test]
    fn a_date_and_time_together_add_up() {
        let noon = datetime::to_serial(2024, 3, 1).unwrap() + 0.5;
        assert_eq!(parse_datetime_text("2024-03-01 12:00"), Some(noon));
        assert_eq!(parse_datetime_text("2024-03-01T12:00:00"), Some(noon));
    }

    #[test]
    fn text_that_is_not_a_date_is_refused_rather_than_guessed() {
        assert_eq!(parse_datetime_text("hello"), None);
        assert_eq!(parse_datetime_text("2024-13-01"), None, "no month 13");
        assert_eq!(parse_datetime_text("2024-02-30"), None, "no such day");
        assert_eq!(parse_datetime_text("1899-12-31"), None, "before the epoch");
        assert_eq!(parse_datetime_text(""), None);
    }

    #[test]
    fn weekend_codes_and_masks_agree() {
        // Code 1 and the string form should describe the same two days off.
        assert_eq!(weekend_mask(&Value::Number(1.0)), Some(DEFAULT_WEEKEND));
        assert_eq!(weekend_mask(&Value::text("0000011")), Some(DEFAULT_WEEKEND));
        // Code 11 is Sunday only: bit 0.
        assert_eq!(weekend_mask(&Value::Number(11.0)), Some(0b0000_0001));
        assert_eq!(weekend_mask(&Value::text("0000001")), Some(0b0000_0001));
        assert_eq!(weekend_mask(&Value::Number(8.0)), None);
        assert_eq!(weekend_mask(&Value::text("1111111")), None, "no days left");
        assert_eq!(weekend_mask(&Value::text("00000")), None);
    }
}
