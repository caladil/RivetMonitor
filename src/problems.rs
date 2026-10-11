//! **Everything wrong right now, in plain words** — `SPEC-01` §5.8.
//!
//! Two records hold what is standing: `state.toml` for the backup pass's
//! conditions and `monitor-state.toml` for the monitor's (`crate::conditions`
//! says why they are two). Each maps a condition's KEY to when it began, and
//! holds no prose on purpose — a stored sentence would be a second copy of one
//! that changes with the build. So the sentence is derived here, from the key,
//! every time it is shown.
//!
//! Both records are READ here and written nowhere: their writers are the backup
//! pass and the monitor, as they always were.

use std::path::Path;

/// One standing condition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The condition's key, as the record holds it — `offers/dev-01`.
    pub key: String,
    /// When it began, epoch seconds.
    pub since: u64,
}

/// **Everything standing in both records, newest first.**
///
/// A record that will not read is itself a problem, named as one, rather than a
/// silent empty list: *"nothing is wrong"* must never be what an unreadable
/// file looks like.
#[must_use]
pub fn standing(config_path: &Path, now: u64) -> Vec<Problem> {
    let mut all = Vec::new();
    match crate::ledger::Ledger::load(config_path) {
        Ok(ledger) => all.extend(ledger.standing),
        Err(_) => all.push((UNREADABLE_LEDGER.to_string(), now)),
    }
    match crate::conditions::read_standing(config_path) {
        Ok(standing) => all.extend(standing),
        Err(_) => all.push((UNREADABLE_MONITOR.to_string(), now)),
    }
    let mut problems: Vec<Problem> = all
        .into_iter()
        .map(|(key, since)| Problem { key, since })
        .collect();
    problems.sort_by(|a, b| b.since.cmp(&a.since).then_with(|| a.key.cmp(&b.key)));
    problems
}

const UNREADABLE_LEDGER: &str = "unreadable/state.toml";
const UNREADABLE_MONITOR: &str = "unreadable/monitor-state.toml";

/// **A condition key as a sentence** — the catalogue in `SPEC-01` §5.5 and
/// `SPEC-02` §6, one line each.
///
/// A key this does not know is shown as itself: an unfamiliar key on the menu
/// is ugly, and a problem missing from the menu is wrong.
#[must_use]
pub fn describe(key: &str) -> String {
    let parts: Vec<&str> = key.split('/').collect();
    let host = parts.get(1).copied().unwrap_or("?");
    let rest = || parts.get(2..).map(|p| p.join("/")).unwrap_or_default();
    match parts[0] {
        "config" => "The config file won't load, so nothing is running".to_string(),
        "refusal" => format!("Setup is incomplete ({}), so no backups run", rest_or(&parts, 1)),
        "monitor-refusal" => format!(
            "Health monitoring is set up wrong ({}), so nothing is recorded",
            rest_or(&parts, 1)
        ),
        "register" => "The fleet server can't be reached".to_string(),
        "offers" => format!("{host} isn't answering"),
        "staging" => format!("Can't prepare the folder for {host}'s backups"),
        "fetch" => format!("Backups from {host} are failing"),
        "place" => format!("A backup of {} from {host} couldn't be stored", rest()),
        "ledger" => "The record of past backups couldn't be written".to_string(),
        "retention" => "Old backups couldn't be tidied away".to_string(),
        "override" => format!("A backup override for {host} matches nothing"),
        "late" => format!("{} on {host} is overdue for a backup", rest()),
        "health" => format!("{host} says an application is unhealthy"),
        "reach" => format!("{host} isn't answering health checks"),
        "log-stalled" => format!("{host}'s log has gone quiet"),
        "card-stalled" => format!("{host}'s SD card may have stopped writing"),
        "frozen" => format!("{host}'s screen looks frozen"),
        "power" => format!("{host} is short of power"),
        "capture" => format!("Can't record {host}'s {}", rest()),
        "unreadable" => format!("Rivet Monitor can't read its own {host}"),
        _ => key.to_string(),
    }
}

fn rest_or(parts: &[&str], from: usize) -> String {
    let joined = parts.get(from..).map(|p| p.join("/")).unwrap_or_default();
    if joined.is_empty() { "a setting".to_string() } else { joined }
}

/// "9 h", "12 min", "2 d", "now" — short, because it sits on a menu line.
#[must_use]
pub fn lasted(since: u64, now: u64) -> String {
    let seconds = now.saturating_sub(since);
    match seconds {
        s if s >= 86_400 => format!("{} d", s / 86_400),
        s if s >= 3_600 => format!("{} h", s / 3_600),
        s if s >= 60 => format!("{} min", s / 60),
        _ => "now".to_string(),
    }
}

/// The menu line for one problem.
#[must_use]
pub fn line(problem: &Problem, now: u64) -> String {
    format!("\u{26A0} {} ({})", describe(&problem.key), lasted(problem.since, now))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **Every key in the two catalogues has a sentence of its own**, so none
    /// reaches the menu as raw text. The list is `SPEC-01` §5.5's table and
    /// `SPEC-02` §6's six.
    #[test]
    fn every_catalogued_key_reads_as_a_sentence() {
        for key in [
            "config",
            "refusal/tool.path",
            "monitor-refusal/monitor.folder",
            "register",
            "offers/dev-01",
            "staging/dev-01",
            "fetch/dev-01",
            "place/rivet-01/photos-rivet",
            "ledger/rivet-01/photos-rivet",
            "retention/rivet-01/photos-rivet",
            "override/rivet-01/photos-rivet/unmatched",
            "late/dev-01/notes-rivet",
            "health/rivet-01",
            "reach/rivet-01",
            "log-stalled/rivet-01",
            "card-stalled/rivet-01",
            "frozen/rivet-01",
            "power/rivet-01",
            "capture/rivet-01/thermal",
        ] {
            let said = describe(key);
            assert_ne!(said, key, "{key} has no sentence");
            assert!(!said.contains('?'), "{key} lost its machine: {said}");
        }
    }

    #[test]
    fn the_sentences_name_the_machine_and_the_application() {
        assert_eq!(describe("offers/dev-01"), "dev-01 isn't answering");
        assert_eq!(
            describe("late/dev-01/notes-rivet"),
            "notes-rivet on dev-01 is overdue for a backup"
        );
        assert_eq!(describe("frozen/rivet-01"), "rivet-01's screen looks frozen");
    }

    #[test]
    fn an_unknown_key_is_shown_as_itself_rather_than_dropped() {
        assert_eq!(describe("something-new/rivet-01"), "something-new/rivet-01");
    }

    #[test]
    fn how_long_is_short() {
        assert_eq!(lasted(0, 30), "now");
        assert_eq!(lasted(0, 12 * 60), "12 min");
        assert_eq!(lasted(0, 9 * 3_600 + 5), "9 h");
        assert_eq!(lasted(0, 2 * 86_400), "2 d");
        assert_eq!(lasted(100, 50), "now", "a clock that went backwards is no time at all");
    }

    /// **Both records, newest first, and an unreadable one is a problem** —
    /// never an empty list that reads as "nothing is wrong".
    #[test]
    fn both_records_are_read_newest_first_and_an_unreadable_one_is_named() {
        let dir = std::env::temp_dir().join(format!("rivet-monitor-problems-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("config.toml");
        assert!(standing(&config, 1_000).is_empty(), "no records at all is nothing wrong");

        std::fs::write(dir.join("state.toml"), "[standing]\n\"offers/dev-01\" = 100\n").unwrap();
        std::fs::write(
            dir.join("monitor-state.toml"),
            "[standing]\n\"frozen/rivet-01\" = 50\n\"reach/rivet-01\" = 200\n",
        )
        .unwrap();
        let keys: Vec<String> = standing(&config, 1_000).into_iter().map(|p| p.key).collect();
        assert_eq!(keys, ["reach/rivet-01", "offers/dev-01", "frozen/rivet-01"]);

        std::fs::write(dir.join("monitor-state.toml"), "this is not toml [").unwrap();
        let keys: Vec<String> = standing(&config, 1_000).into_iter().map(|p| p.key).collect();
        assert!(keys.contains(&UNREADABLE_MONITOR.to_string()), "{keys:?}");
        assert!(keys.contains(&"offers/dev-01".to_string()), "{keys:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
