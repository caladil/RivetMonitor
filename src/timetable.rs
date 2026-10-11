//! **When a backup is due** — `SPEC-01` §3.1.
//!
//! A schedule is zero or more weekly times on this PC's own clock —
//! `["Tue 04:00", "Fri 04:00"]` — and an empty one means *manual only*: nothing
//! is fetched until the operator asks (`SPEC-01` §3.7).
//!
//! # Due means "nothing has succeeded since the last time that has passed"
//!
//! Not *"it is 04:00 now"*. A pass runs every few minutes, and a pass that
//! happened to be running at 04:00 is not what makes 04:00 count — Rivet
//! Monitor can be off at the appointed minute for reasons that have nothing to
//! do with the PC sleeping: a Windows Update reboot, a restart of this program,
//! a configuration it refused until the operator fixed it. Asking *"has a
//! backup succeeded since the most recent scheduled time?"* fires on the first
//! pass after the time, however late that pass is, and cannot be missed.
//!
//! **Late is the same question one time further back**: no success since the
//! scheduled time before the most recent one, so two scheduled backups in a
//! row have passed without a success. One missed is a reboot; two is a backup
//! that has stopped.
//!
//! # Times are converted by Windows, not by arithmetic
//!
//! A local time becomes an instant through [`crate::clock::Zone`], which on
//! Windows asks the operating system for that DATE's offset. A scheduled time
//! the week before a daylight-saving change is therefore the instant it really
//! was, and not one an hour off — which would read a Friday 04:00 backup as
//! having happened before Friday 04:00 and fetch it again on Monday.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::clock::Zone;

/// How far back [`recent`] looks: two weeks and a day, so even a schedule of
/// one time a week has two occurrences behind it.
const LOOK_BACK_DAYS: i64 = 15;

const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// One weekly time — `Tue 04:00`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Slot {
    /// Monday is 0.
    pub weekday: u8,
    /// Minutes after local midnight.
    pub minute: u32,
}

impl std::str::FromStr for Slot {
    type Err = String;

    /// **Strict, and the refusal says what would have been accepted.** A time
    /// that was quietly read as something else is a backup at an hour the
    /// operator did not choose; one that was quietly dropped is no backup.
    fn from_str(text: &str) -> Result<Self, String> {
        let shape = || {
            format!(
                "{text:?} is not a schedule time. Write a day and a 24-hour time, like \"Tue \
                 04:00\": the day is one of {}, and the time is HH:MM from 00:00 to 23:59 \
                 (SPEC-01 §2.2).",
                DAYS.join(", ")
            )
        };
        let mut parts = text.split_whitespace();
        let (Some(day), Some(time), None) = (parts.next(), parts.next(), parts.next()) else {
            return Err(shape());
        };
        let weekday = DAYS
            .iter()
            .position(|d| d.eq_ignore_ascii_case(day))
            .ok_or_else(shape)?;
        let bytes = time.as_bytes();
        if bytes.len() != 5 || bytes[2] != b':' {
            return Err(shape());
        }
        let number = |range: std::ops::Range<usize>| {
            time.get(range)
                .filter(|s| s.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|s| s.parse::<u32>().ok())
        };
        let (Some(hour), Some(minute)) = (number(0..2), number(3..5)) else {
            return Err(shape());
        };
        if hour > 23 || minute > 59 {
            return Err(shape());
        }
        Ok(Self {
            weekday: u8::try_from(weekday).map_err(|_| shape())?,
            minute: hour * 60 + minute,
        })
    }
}

impl fmt::Display for Slot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {:02}:{:02}",
            DAYS[usize::from(self.weekday % 7)],
            self.minute / 60,
            self.minute % 60
        )
    }
}

/// Zero or more weekly times. **Empty is manual only**, never "the default".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Schedule(pub Vec<Slot>);

impl Schedule {
    /// Nothing is scheduled: the application is backed up only when asked.
    #[must_use]
    pub fn manual(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for Schedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.manual() {
            return f.write_str("manual only");
        }
        let mut sorted = self.0.clone();
        sorted.sort();
        let text: Vec<String> = sorted.iter().map(Slot::to_string).collect();
        f.write_str(&text.join(", "))
    }
}

impl<'de> Deserialize<'de> for Schedule {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let texts = Vec::<String>::deserialize(deserializer)?;
        let mut slots = Vec::with_capacity(texts.len());
        for text in &texts {
            slots.push(text.parse::<Slot>().map_err(serde::de::Error::custom)?);
        }
        Ok(Self(slots))
    }
}

impl Serialize for Schedule {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let texts: Vec<String> = self.0.iter().map(Slot::to_string).collect();
        texts.serialize(serializer)
    }
}

/// Monday-is-0 weekday of a day number. 1970-01-01 was a Thursday.
#[must_use]
pub fn weekday(day: i64) -> u8 {
    u8::try_from((day + 3).rem_euclid(7)).unwrap_or(0)
}

/// **The two most recent scheduled instants at or before `now`**, newest
/// first. Both `None` for a manual schedule.
#[must_use]
pub fn recent(schedule: &Schedule, zone: &dyn Zone, now: u64) -> (Option<u64>, Option<u64>) {
    let (today, minute_now) = zone.local(now);
    let mut instants: Vec<u64> = Vec::new();
    for back in 0..=LOOK_BACK_DAYS {
        let day = today - back;
        for slot in &schedule.0 {
            if slot.weekday != weekday(day) || (back == 0 && slot.minute > minute_now) {
                continue;
            }
            if let Some(at) = zone.utc(day, slot.minute)
                && at <= now
            {
                instants.push(at);
            }
        }
    }
    instants.sort_unstable_by(|a, b| b.cmp(a));
    instants.dedup();
    (instants.first().copied(), instants.get(1).copied())
}

/// **Is this application due?** — `SPEC-01` §3.1.
///
/// Manual only is never due on its own. Otherwise: never backed up is due at
/// once, and a backup is due when nothing has succeeded since the most recent
/// scheduled time.
#[must_use]
pub fn due(schedule: &Schedule, last_success: Option<u64>, zone: &dyn Zone, now: u64) -> bool {
    if schedule.manual() {
        return false;
    }
    match last_success {
        None => true,
        Some(then) => recent(schedule, zone, now).0.is_some_and(|slot| then < slot),
    }
}

/// **The instant a success must be at or after, or the application is late** —
/// `SPEC-01` §5.5.3. The scheduled time before the most recent one; `None` for
/// manual only, which is never late.
#[must_use]
pub fn late_unless_since(schedule: &Schedule, zone: &dyn Zone, now: u64) -> Option<u64> {
    if schedule.manual() {
        return None;
    }
    // A schedule with any time in it has two behind it inside the look-back,
    // so this is `None` only if the zone could convert neither — and then the
    // earliest instant there is stands in, which makes "never" the only late.
    Some(recent(schedule, zone, now).1.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;
    use crate::clock::{Fixed, days_from_civil};

    fn slot(text: &str) -> Slot {
        text.parse().expect("a valid slot")
    }

    fn schedule(texts: &[&str]) -> Schedule {
        Schedule(texts.iter().map(|t| slot(t)).collect())
    }

    /// Epoch seconds of a UTC civil time.
    fn at(y: i64, mo: u32, d: u32, h: u64, mi: u64) -> u64 {
        u64::try_from(days_from_civil(y, mo, d)).unwrap() * 86_400 + h * 3_600 + mi * 60
    }

    #[test]
    fn a_slot_reads_a_day_and_a_24_hour_time() {
        assert_eq!(slot("Tue 04:00"), Slot { weekday: 1, minute: 240 });
        assert_eq!(slot("sun 23:59"), Slot { weekday: 6, minute: 1_439 });
        assert_eq!(slot("  Mon   00:00 "), Slot { weekday: 0, minute: 0 });
        assert_eq!(slot("Fri 01:00").to_string(), "Fri 01:00");
    }

    /// **Anything that is not exactly a day and HH:MM is refused**, naming
    /// what would have been accepted.
    #[test]
    fn a_slot_that_is_not_exactly_a_day_and_a_time_is_refused() {
        for bad in [
            "Tues 04:00", "Tue 4:00", "Tue 4am", "Tue 24:00", "Tue 12:60", "Tue", "04:00",
            "Tue 04:00 Fri", "Tue 04-00", "Tue +4:00", "",
        ] {
            let why = bad.parse::<Slot>().expect_err(bad);
            assert!(why.contains("Mon, Tue, Wed"), "{bad:?}: {why}");
        }
    }

    #[test]
    fn a_schedule_in_the_file_parses_and_an_empty_one_is_manual() {
        #[derive(Deserialize)]
        struct T {
            schedule: Schedule,
        }
        let t: T = toml::from_str("schedule = [\"Fri 04:00\", \"Tue 04:00\"]").unwrap();
        assert_eq!(t.schedule.to_string(), "Tue 04:00, Fri 04:00");
        let t: T = toml::from_str("schedule = []").unwrap();
        assert!(t.schedule.manual());
        assert_eq!(t.schedule.to_string(), "manual only");
        let bad = toml::from_str::<T>("schedule = [\"Tue 4am\"]").err().unwrap();
        assert!(bad.to_string().contains("Tue 4am"), "{bad}");
    }

    #[test]
    fn the_epoch_was_a_thursday() {
        assert_eq!(weekday(0), 3);
        // 2026-10-02 is a Friday.
        assert_eq!(weekday(days_from_civil(2026, 10, 2)), 4);
    }

    /// A twice-weekly schedule, on the clock: Tuesday and Friday at 04:00.
    #[test]
    fn due_once_each_scheduled_time_has_passed_and_not_again() {
        let s = schedule(&["Tue 04:00", "Fri 04:00"]);
        let utc = Fixed(0);
        // Backed up Tuesday 2026-09-29 at 04:02.
        let tuesday = at(2026, 9, 29, 4, 2);
        // Thursday: nothing since Tuesday 04:00 is owed.
        assert!(!due(&s, Some(tuesday), &utc, at(2026, 10, 1, 12, 0)));
        // Friday 03:59: still not.
        assert!(!due(&s, Some(tuesday), &utc, at(2026, 10, 2, 3, 59)));
        // Friday 04:00: due.
        assert!(due(&s, Some(tuesday), &utc, at(2026, 10, 2, 4, 0)));
        // Backed up at 04:03; due no longer.
        assert!(!due(&s, Some(at(2026, 10, 2, 4, 3)), &utc, at(2026, 10, 2, 4, 5)));
    }

    /// **The point of measuring from the last scheduled time**: Rivet Monitor
    /// was not running at 04:00 Friday — a Windows Update reboot — and the
    /// first pass after it comes back, at 09:17, is the one that fetches.
    #[test]
    fn a_time_passed_while_this_program_was_not_running_is_caught_up() {
        let s = schedule(&["Fri 04:00"]);
        let last = at(2026, 9, 25, 4, 1);
        assert!(due(&s, Some(last), &Fixed(0), at(2026, 10, 2, 9, 17)));
    }

    #[test]
    fn manual_only_is_never_due_and_never_late() {
        let s = Schedule::default();
        assert!(!due(&s, None, &Fixed(0), at(2026, 10, 2, 4, 0)));
        assert_eq!(late_unless_since(&s, &Fixed(0), at(2026, 10, 2, 4, 0)), None);
    }

    #[test]
    fn never_backed_up_on_a_schedule_is_due_at_once() {
        assert!(due(&schedule(&["Sun 12:00"]), None, &Fixed(0), at(2026, 10, 2, 4, 0)));
    }

    /// **Late is two scheduled times missed in a row**, not one.
    #[test]
    fn late_is_the_time_before_the_most_recent() {
        let s = schedule(&["Tue 04:00", "Fri 04:00"]);
        let now = at(2026, 10, 2, 10, 0); // Friday
        assert_eq!(
            late_unless_since(&s, &Fixed(0), now),
            Some(at(2026, 9, 29, 4, 0)),
            "Tuesday's time: a success since then is not late, even if Friday's was missed"
        );
        // One time a week looks back two weeks.
        let once = schedule(&["Mon 21:00"]);
        assert_eq!(
            recent(&once, &Fixed(0), now),
            (Some(at(2026, 9, 28, 21, 0)), Some(at(2026, 9, 21, 21, 0)))
        );
    }

    /// The inane schedule, because it is allowed.
    #[test]
    fn four_times_a_week_are_four_times_a_week() {
        let s = schedule(&["Mon 21:00", "Tue 15:00", "Fri 01:00", "Sun 12:00"]);
        let now = at(2026, 10, 2, 0, 59); // Friday, a minute before 01:00
        assert_eq!(
            recent(&s, &Fixed(0), now),
            (Some(at(2026, 9, 29, 15, 0)), Some(at(2026, 9, 28, 21, 0)))
        );
        assert_eq!(recent(&s, &Fixed(0), now + 60).0, Some(at(2026, 10, 2, 1, 0)));
    }

    /// **A local time is the PC's**: Friday 04:00 at UTC+9 is Thursday 19:00 UTC.
    #[test]
    fn a_scheduled_time_is_local_not_utc() {
        let s = schedule(&["Fri 04:00"]);
        let utc_plus_9 = Fixed(9 * 3_600);
        assert_eq!(recent(&s, &utc_plus_9, at(2026, 10, 1, 19, 0)).0, Some(at(2026, 10, 1, 19, 0)));
        assert_ne!(recent(&s, &utc_plus_9, at(2026, 10, 1, 18, 59)).0, Some(at(2026, 10, 1, 19, 0)));
    }

    /// **Across a daylight-saving change, each time is converted at its own
    /// date's offset.** A zone that moves from UTC+2 to UTC+1 at 01:00 UTC on
    /// 2026-10-25: Friday 04:00 before it is 02:00 UTC, and must not be read on
    /// Monday at Monday's offset as 03:00 — which would make a backup taken at
    /// 02:02 UTC look older than the time it answered, and fetch it again.
    #[test]
    fn a_daylight_saving_change_between_two_times_does_not_move_the_older_one() {
        struct Shifting;
        impl Zone for Shifting {
            fn local(&self, utc: u64) -> (i64, u32) {
                let offset: i64 = if utc < at(2026, 10, 25, 1, 0) { 2 } else { 1 };
                let local = i64::try_from(utc).unwrap() + offset * 3_600;
                (local.div_euclid(86_400), u32::try_from(local.rem_euclid(86_400) / 60).unwrap())
            }
            fn utc(&self, day: i64, minute: u32) -> Option<u64> {
                let offset: i64 = if day < days_from_civil(2026, 10, 25) { 2 } else { 1 };
                u64::try_from(day * 86_400 + i64::from(minute) * 60 - offset * 3_600).ok()
            }
        }
        let s = schedule(&["Fri 04:00"]);
        let friday = at(2026, 10, 23, 2, 0);
        let monday = at(2026, 10, 26, 15, 0);
        assert_eq!(recent(&s, &Shifting, monday).0, Some(friday));
        assert!(!due(&s, Some(friday + 120), &Shifting, monday));
    }
}
