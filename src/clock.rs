//! **Dates, without a date library** — `SPEC-02` §5.1.
//!
//! The monitor names a file by the PC's LOCAL date at capture and stamps every
//! line with the PC's UTC time. Both are a few lines of civil arithmetic
//! (Howard Hinnant's `days_from_civil`, public domain), and the local date is
//! read from Windows itself rather than derived from a timezone table this
//! program would have to keep right.
//!
//! **The board's clock is never used here.** A Pi has no RTC; the PC's clock is
//! the one both halves of every line agree on.

/// Seconds since the Unix epoch, now.
#[must_use]
pub fn now() -> u64 {
    crate::ledger::now()
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
#[must_use]
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = i64::from(month);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date `days` after 1970-01-01.
#[must_use]
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `2026-09-15T08:49:03Z`.
#[must_use]
pub fn utc_stamp(seconds: u64) -> String {
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let rest = seconds % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        (rest % 3_600) / 60,
        rest % 60
    )
}

/// Read back a [`utc_stamp`]. Anything else is `None`.
#[must_use]
pub fn parse_stamp(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    if bytes.len() != 20 || bytes[10] != b'T' || bytes[19] != b'Z' {
        return None;
    }
    let days = parse_date(&text[..10])?;
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<u64>().ok();
    if bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    let (h, mi, s) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if h > 23 || mi > 59 || s > 60 {
        return None;
    }
    u64::try_from(days)
        .ok()
        .map(|days| days * 86_400 + h * 3_600 + mi * 60 + s)
}

/// `YYYY-MM-DD` as days since the epoch, or `None` for anything else.
#[must_use]
pub fn parse_date(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    if !text
        .char_indices()
        .all(|(at, c)| at == 4 || at == 7 || c.is_ascii_digit())
    {
        return None;
    }
    let year: i64 = text[..4].parse().ok()?;
    let month: u32 = text[5..7].parse().ok()?;
    let day: u32 = text[8..10].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, month, day);
    // A date like 2026-02-31 does not survive the round trip.
    (civil_from_days(days) == (year, month, day)).then_some(days)
}

/// `YYYY-MM-DD` for a day number.
#[must_use]
pub fn date_name(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// **Today, on this PC's own calendar**, as a day number.
#[cfg(windows)]
#[must_use]
pub fn local_today() -> i64 {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;

    // SAFETY: SYSTEMTIME is plain data and GetLocalTime fills every field.
    let mut time: SYSTEMTIME = unsafe { std::mem::zeroed() };
    // SAFETY: `time` is a valid, writable SYSTEMTIME.
    unsafe { GetLocalTime(&mut time) };
    days_from_civil(
        i64::from(time.wYear),
        u32::from(time.wMonth),
        u32::from(time.wDay),
    )
}

/// Off Windows there is no tray and no local-calendar call here; UTC stands in.
#[cfg(not(windows))]
#[must_use]
pub fn local_today() -> i64 {
    local_now().0
}

/// **Today and the time of day, on this PC's own clock, from ONE reading** —
/// the day number and `HHMMSS` a stored screenshot is named by (`SPEC-02`
/// §5.1). One reading, so a capture a moment before midnight is not filed
/// under one day and named by the next.
#[cfg(windows)]
#[must_use]
pub fn local_now() -> (i64, String) {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;

    // SAFETY: SYSTEMTIME is plain data and GetLocalTime fills every field.
    let mut time: SYSTEMTIME = unsafe { std::mem::zeroed() };
    // SAFETY: `time` is a valid, writable SYSTEMTIME.
    unsafe { GetLocalTime(&mut time) };
    (
        days_from_civil(
            i64::from(time.wYear),
            u32::from(time.wMonth),
            u32::from(time.wDay),
        ),
        format!("{:02}{:02}{:02}", time.wHour, time.wMinute, time.wSecond),
    )
}

/// Off Windows, UTC stands in.
#[cfg(not(windows))]
#[must_use]
pub fn local_now() -> (i64, String) {
    let at = now();
    (
        i64::try_from(at / 86_400).unwrap_or(0),
        format!("{:02}{:02}{:02}", at % 86_400 / 3_600, at % 3_600 / 60, at % 60),
    )
}

/// **Local time, both ways** — what a backup schedule is written in
/// (`SPEC-01` §3.1).
///
/// A trait so the schedule can be proved against a zone a test controls,
/// daylight-saving change included, rather than against whatever this PC's
/// zone happens to be.
pub trait Zone {
    /// The local day number and minute of day at a UTC instant.
    fn local(&self, utc: u64) -> (i64, u32);
    /// The UTC instant of a local day and minute, at THAT date's offset.
    /// `None` when the zone cannot say.
    fn utc(&self, day: i64, minute: u32) -> Option<u64>;
}

/// A zone that is always the same distance from UTC — the tests' zone, and
/// UTC itself off Windows.
pub struct Fixed(pub i64);

impl Zone for Fixed {
    fn local(&self, utc: u64) -> (i64, u32) {
        let local = i64::try_from(utc).unwrap_or(0) + self.0;
        (
            local.div_euclid(86_400),
            u32::try_from(local.rem_euclid(86_400) / 60).unwrap_or(0),
        )
    }

    fn utc(&self, day: i64, minute: u32) -> Option<u64> {
        u64::try_from(day * 86_400 + i64::from(minute) * 60 - self.0).ok()
    }
}

/// **This PC's own zone, as Windows keeps it.** Each conversion asks Windows
/// for the offset in force on that date, so a time on the far side of a
/// daylight-saving change is the instant it really was.
pub struct Local;

#[cfg(windows)]
fn system_time(day: i64, minute: u32) -> Option<windows_sys::Win32::Foundation::SYSTEMTIME> {
    let (year, month, date) = civil_from_days(day);
    Some(windows_sys::Win32::Foundation::SYSTEMTIME {
        wYear: u16::try_from(year).ok()?,
        wMonth: u16::try_from(month).ok()?,
        wDayOfWeek: 0,
        wDay: u16::try_from(date).ok()?,
        wHour: u16::try_from(minute / 60).ok()?,
        wMinute: u16::try_from(minute % 60).ok()?,
        wSecond: 0,
        wMilliseconds: 0,
    })
}

#[cfg(windows)]
impl Zone for Local {
    fn local(&self, utc: u64) -> (i64, u32) {
        use windows_sys::Win32::System::Time::SystemTimeToTzSpecificLocalTime;

        let fallback = Fixed(0).local(utc);
        let day = i64::try_from(utc / 86_400).unwrap_or(0);
        let Some(universal) = system_time(day, u32::try_from(utc % 86_400 / 60).unwrap_or(0))
        else {
            return fallback;
        };
        // SAFETY: SYSTEMTIME is plain data, filled on success.
        let mut local: windows_sys::Win32::Foundation::SYSTEMTIME = unsafe { std::mem::zeroed() };
        // SAFETY: a null zone means the zone currently in force; both pointers
        // are valid for the call.
        if unsafe { SystemTimeToTzSpecificLocalTime(std::ptr::null(), &universal, &mut local) } == 0
        {
            return fallback;
        }
        (
            days_from_civil(
                i64::from(local.wYear),
                u32::from(local.wMonth),
                u32::from(local.wDay),
            ),
            u32::from(local.wHour) * 60 + u32::from(local.wMinute),
        )
    }

    fn utc(&self, day: i64, minute: u32) -> Option<u64> {
        use windows_sys::Win32::System::Time::TzSpecificLocalTimeToSystemTime;

        let local = system_time(day, minute)?;
        // SAFETY: SYSTEMTIME is plain data, filled on success.
        let mut universal: windows_sys::Win32::Foundation::SYSTEMTIME =
            unsafe { std::mem::zeroed() };
        // SAFETY: a null zone means the zone currently in force; both pointers
        // are valid for the call.
        if unsafe { TzSpecificLocalTimeToSystemTime(std::ptr::null(), &local, &mut universal) } == 0
        {
            return None;
        }
        let days = days_from_civil(
            i64::from(universal.wYear),
            u32::from(universal.wMonth),
            u32::from(universal.wDay),
        );
        u64::try_from(days).ok().map(|days| {
            days * 86_400 + u64::from(universal.wHour) * 3_600 + u64::from(universal.wMinute) * 60
        })
    }
}

/// Off Windows, UTC stands in.
#[cfg(not(windows))]
impl Zone for Local {
    fn local(&self, utc: u64) -> (i64, u32) {
        Fixed(0).local(utc)
    }

    fn utc(&self, day: i64, minute: u32) -> Option<u64> {
        Fixed(0).utc(day, minute)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn a_stamp_round_trips_and_names_the_right_day() {
        // 2026-09-15T08:49:03Z
        let at = 1_789_462_143;
        assert_eq!(utc_stamp(at), "2026-09-15T08:49:03Z");
        assert_eq!(parse_stamp("2026-09-15T08:49:03Z"), Some(at));
        assert_eq!(date_name(days_from_civil(2026, 9, 15)), "2026-09-15");
    }

    #[test]
    fn a_date_that_is_not_a_date_is_refused() {
        assert_eq!(parse_date("2026-02-31"), None);
        assert_eq!(parse_date("notes-2026"), None);
        assert_eq!(parse_date("2026-9-15"), None);
        assert!(parse_date("2024-02-29").is_some());
        assert_eq!(parse_stamp("2026-09-15 08:49:03"), None);
    }

    /// **Windows' own conversion, both ways, on this PC's real zone** — on
    /// every day of the year, so both of this year's daylight-saving changes are
    /// crossed in whichever zone the test runs in, and now. The small hours are
    /// avoided: on a spring-forward day some of them do not exist, and what
    /// Windows does with them is its own business.
    #[cfg(windows)]
    #[test]
    fn this_pcs_zone_round_trips_a_local_time() {
        for day in days_from_civil(2026, 1, 1)..days_from_civil(2027, 1, 1) {
            for minute in [4 * 60 + 30, 12 * 60, 21 * 60 + 15, 23 * 60 + 59] {
                let utc = Local.utc(day, minute).expect("Windows converts it");
                assert_eq!(Local.local(utc), (day, minute), "day {day} minute {minute}");
            }
        }
        let now = now() / 60 * 60;
        let (day, minute) = Local.local(now);
        assert_eq!(Local.utc(day, minute), Some(now));
    }

    #[test]
    fn days_and_dates_agree_across_a_leap_year() {
        for days in [-1, 0, 59, 60, 365, 19_782, 20_711, 40_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
    }
}
