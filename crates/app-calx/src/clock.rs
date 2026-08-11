//! The wall clock, read in the user's own time zone.
//!
//! Ctrl+; types today's date and Ctrl+Shift+; the current time, and both are
//! wrong an evening's worth of the day if they come from UTC. The `time`
//! crate asks the platform for the local offset; when it cannot answer —
//! which happens on some multi-threaded Unixes — UTC is the honest fallback
//! rather than a guess at the zone.

/// (year, month, day, hour, minute), local time.
pub fn local() -> (i32, u32, u32, u32, u32) {
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    (
        now.year(),
        u32::from(u8::from(now.month())),
        u32::from(now.day()),
        u32::from(now.hour()),
        u32::from(now.minute()),
    )
}

/// Today, the way it would be typed: `8/11/2026`.
pub fn date_text() -> String {
    let (year, month, day, _, _) = local();
    format!("{month}/{day}/{year}")
}

/// Now, the way it would be typed: `14:35`.
pub fn time_text() -> String {
    let (_, _, _, hour, minute) = local();
    format!("{hour}:{minute:02}")
}
