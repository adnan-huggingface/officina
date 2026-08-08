//! Serial dates, and the leap year that never happened.
//!
//! A date in a spreadsheet is a number: 1 is 1 January 1900, and the fractional
//! part is the time of day. Converting between that number and a calendar date
//! would be ordinary except for one thing.
//!
//! **1900 was not a leap year, and Excel thinks it was.** Serial 60 is
//! 29 February 1900, a day that does not exist. Lotus 1-2-3 had the bug, Excel
//! copied it for file compatibility in 1985, and it has been load-bearing ever
//! since — every serial from 61 onwards is offset by that phantom day, so
//! removing the bug would move every date in every existing file. The result is
//! that serials 1–59 and serials 61 onwards use *different* epochs, and 60 maps
//! to nothing at all.
//!
//! Getting this wrong is invisible: dates after 1 March 1900 are all shifted by
//! exactly one day, which looks like a plausible off-by-one rather than a
//! calendar bug.

/// Serial 25569 is 1 January 1970, which is where the civil-date algorithms
/// below count from.
const UNIX_EPOCH_SERIAL: i64 = 25569;

/// The same constant where a serial is being built rather than indexed.
pub const UNIX_EPOCH_SERIAL_F64: f64 = 25569.0;

/// The last serial the 1900 system can express: 31 December 9999.
pub const MAX_SERIAL: f64 = 2_958_465.0;

/// A calendar date and time, as the date functions want to see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTime {
    pub year: i32,
    pub month: u32,
    /// Zero for serial 0, which Excel presents as the impossible 0 January 1900.
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// Splits a serial number into a date and a time.
///
/// `None` for a negative serial or one past the end of the supported range —
/// both of which are `#NUM!` to every function that calls this.
pub fn from_serial(serial: f64) -> Option<DateTime> {
    if !(0.0..MAX_SERIAL + 1.0).contains(&serial) {
        return None;
    }
    let mut days = serial.floor() as i64;
    let fraction = serial - days as f64;

    // Rounding to the second first is what makes `HOUR` agree with what the
    // cell displays: a serial arrived at by arithmetic is rarely exact, and
    // truncating 11:59:59.9999999 would report 11, not 12.
    let mut seconds = (fraction * 86_400.0).round() as i64;
    if seconds >= 86_400 {
        seconds -= 86_400;
        days += 1;
    }

    let (year, month, day) = civil_from_serial(days)?;
    Some(DateTime {
        year,
        month,
        day,
        hour: (seconds / 3_600) as u32,
        minute: (seconds / 60 % 60) as u32,
        second: (seconds % 60) as u32,
    })
}

/// The calendar date for a whole-day serial.
fn civil_from_serial(days: i64) -> Option<(i32, u32, u32)> {
    match days {
        // Excel's zero date. `YEAR(0)` is 1900 and `DAY(0)` is 0, so it is not
        // 31 December 1899 by another name — it is a day with no number.
        0 => Some((1900, 1, 0)),
        // The phantom. No civil date maps here, so it is spelled out.
        60 => Some((1900, 2, 29)),
        // Before the phantom, serials count from 31 December 1899...
        1..=59 => Some(civil_from_days(days - UNIX_EPOCH_SERIAL + 1)),
        // ...and after it, from 30 December 1899.
        _ => (days > 60).then(|| civil_from_days(days - UNIX_EPOCH_SERIAL)),
    }
}

/// The serial for a calendar date, or `None` before 1900 or past 9999.
///
/// A day inside the gap the phantom leap day creates — 29 February 1900 — is
/// accepted and maps to serial 60, because that is the serial Excel stores for
/// it and refusing would make `DATE(1900,2,29)` an error where Excel gives a
/// number.
pub fn to_serial(year: i32, month: u32, day: u32) -> Option<f64> {
    if year == 1900 && month == 2 && day == 29 {
        return Some(60.0);
    }
    let days = days_from_civil(i64::from(year), i64::from(month), i64::from(day));
    let serial = days + UNIX_EPOCH_SERIAL;
    // Everything up to 28 February 1900 predates the phantom and needs the
    // other epoch; 1 March 1900 onwards keeps this one.
    let serial = if serial <= 60 { serial - 1 } else { serial };
    (1..=MAX_SERIAL as i64)
        .contains(&serial)
        .then_some(serial as f64)
}

/// `to_serial` after normalizing an out-of-range month or day, which is what
/// `DATE(2024,13,1)` and `DATE(2024,1,32)` both rely on.
pub fn to_serial_normalized(year: i32, month: i64, day: i64) -> Option<f64> {
    // Months roll into years first, so the day arithmetic below has a real
    // month to start from.
    let total = i64::from(year) * 12 + month - 1;
    let year = total.div_euclid(12);
    let month = total.rem_euclid(12) + 1;
    let year = i32::try_from(year).ok()?;

    // Day 0 is the last day of the previous month, and day 32 in January is
    // 1 February — so the day is applied as an offset rather than checked.
    let base = to_serial(year, month as u32, 1)?;
    let serial = base + (day - 1) as f64;
    (1.0..=MAX_SERIAL).contains(&serial).then_some(serial)
}

/// Days in a month, honouring the Gregorian leap rule — and the phantom.
pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        // 1900 is not a leap year, but Excel's calendar says otherwise and
        // `EOMONTH(DATE(1900,2,1),0)` returns the 29th.
        2 if year == 1900 => 29,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

pub const fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Day of the week as 0 = Sunday, from a serial.
///
/// Excel derives the weekday from the serial arithmetically, phantom day and
/// all, so it believes 1 January 1900 was a Sunday. It was a Monday. Every date
/// from 1 March 1900 onwards is back in step with the real calendar, because
/// the extra day has been absorbed by then — so this is wrong exactly where
/// Excel is wrong and right everywhere anyone cares.
pub fn weekday_from_serial(serial: f64) -> u32 {
    let days = serial.floor() as i64;
    (days - 1).rem_euclid(7) as u32
}

/// Howard Hinnant's civil-from-days, counting from 1970-01-01.
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    ((if m <= 2 { y + 1 } else { y }) as i32, m as u32, d as u32)
}

/// The inverse, and the reason both are here rather than pulled from a crate:
/// they are twenty lines, exact for every year, and free of a dependency whose
/// leap-second and time-zone handling we would then have to explain away.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ymd(serial: f64) -> (i32, u32, u32) {
        let d = from_serial(serial).expect("in range");
        (d.year, d.month, d.day)
    }

    #[test]
    fn the_phantom_leap_day_sits_at_serial_sixty() {
        // The three serials that define the whole quirk.
        assert_eq!(ymd(59.0), (1900, 2, 28));
        assert_eq!(ymd(60.0), (1900, 2, 29), "a day that never existed");
        assert_eq!(ymd(61.0), (1900, 3, 1));
    }

    #[test]
    fn serials_either_side_of_the_phantom_use_different_epochs() {
        assert_eq!(ymd(1.0), (1900, 1, 1));
        assert_eq!(to_serial(1900, 1, 1), Some(1.0));
        assert_eq!(to_serial(1900, 2, 28), Some(59.0));
        assert_eq!(to_serial(1900, 2, 29), Some(60.0), "Excel stores this");
        assert_eq!(to_serial(1900, 3, 1), Some(61.0));
    }

    #[test]
    fn ordinary_dates_round_trip() {
        // The one everybody knows: 2008-01-01 is 39448.
        assert_eq!(to_serial(2008, 1, 1), Some(39448.0));
        assert_eq!(ymd(39448.0), (2008, 1, 1));
        assert_eq!(to_serial(1970, 1, 1), Some(25569.0));
        assert_eq!(to_serial(2000, 2, 29), Some(36585.0), "a real leap day");
        assert_eq!(ymd(45000.0), (2023, 3, 15));
        assert_eq!(to_serial(9999, 12, 31), Some(MAX_SERIAL));
    }

    #[test]
    fn the_zero_serial_is_a_day_with_no_number() {
        assert_eq!(ymd(0.0), (1900, 1, 0));
    }

    #[test]
    fn out_of_range_serials_are_rejected() {
        assert_eq!(from_serial(-1.0), None);
        assert_eq!(from_serial(MAX_SERIAL + 1.0), None);
        assert_eq!(to_serial(1899, 12, 31), None);
        assert_eq!(to_serial(10000, 1, 1), None);
    }

    #[test]
    fn the_time_of_day_is_the_fraction() {
        let noon = from_serial(1.5).expect("in range");
        assert_eq!((noon.hour, noon.minute, noon.second), (12, 0, 0));
        let t =
            from_serial(1.0 + (13.0 * 3600.0 + 45.0 * 60.0 + 30.0) / 86400.0).expect("in range");
        assert_eq!((t.hour, t.minute, t.second), (13, 45, 30));
    }

    #[test]
    fn a_time_that_rounds_up_to_midnight_lands_on_the_next_day() {
        // Arithmetic rarely produces an exact serial; truncating here would
        // report 23:59:59 for something the cell displays as the next day.
        let t = from_serial(43000.9999999).expect("in range");
        assert_eq!((t.hour, t.minute, t.second), (0, 0, 0));
        assert_eq!((t.year, t.month, t.day), (2017, 9, 23));
    }

    #[test]
    fn weekdays_come_from_the_serial_not_the_calendar() {
        // Excel calls 1 January 1900 a Sunday; the calendar says Monday.
        assert_eq!(weekday_from_serial(1.0), 0);
        assert_eq!(weekday_from_serial(to_serial(2024, 1, 1).unwrap()), 1);
        assert_eq!(weekday_from_serial(to_serial(2024, 1, 7).unwrap()), 0);
    }

    #[test]
    fn out_of_range_months_and_days_roll_over() {
        assert_eq!(to_serial_normalized(2024, 13, 1), to_serial(2025, 1, 1));
        assert_eq!(to_serial_normalized(2024, 1, 32), to_serial(2024, 2, 1));
        assert_eq!(to_serial_normalized(2024, 3, 0), to_serial(2024, 2, 29));
        assert_eq!(to_serial_normalized(2024, 0, 1), to_serial(2023, 12, 1));
    }

    #[test]
    fn february_1900_has_twenty_nine_days_because_excel_says_so() {
        assert_eq!(days_in_month(1900, 2), 29);
        assert_eq!(days_in_month(1901, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(2100, 2), 28);
    }
}
