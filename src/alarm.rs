//! **What is worth telling the operator about, and exactly once** —
//! `SPEC-01` §5.5.
//!
//! # The failure this module exists to prevent
//!
//! TempMon's tray raised a Windows notification on every poll saying that
//! `https://rivet-01.local/tempmon/api/v1/backup` answered `403`. One fact,
//! hundreds of notifications, over hours, while the maintainer was trying to
//! work. That is what a monitor that announces a **state** does. This one
//! announces a **change**: a condition that was not there and now is, or was
//! there and now is not.
//!
//! # A condition that was not looked at has not cleared
//!
//! The second failure, and the worse one, because it destroys trust rather than
//! patience: a pass that stops early — the register would not load, the tool is
//! gone — has not examined a single application. *"It did not complain"* is not
//! the same statement as *"it looked and it was fine"*, and a monitor that said
//! **resolved** because it had stopped looking would be lying at the exact
//! moment the operator most needs it not to.
//!
//! So [`Pass::evaluated`](crate::schedule::Pass::evaluated) records every check
//! that actually ran, and a standing condition clears **only** when its own
//! check ran and found nothing. A condition whose check did not run this pass
//! stays standing, silently, until something can say either way.
//!
//! # What reaches the desktop is decided elsewhere
//!
//! [`Announcement`]s are produced and logged. Which standing conditions become
//! a Windows notification, and when, is `crate::notify`'s — `SPEC-01` §5.5.4 —
//! and it reads the standing records rather than these announcements, because
//! a notification waits for a problem to LAST and an announcement does not.

use std::collections::{BTreeMap, BTreeSet};

use crate::ledger::{self, Ledger};
use crate::schedule::Pass;

/// Which way a condition moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// It was not there last pass and it is now.
    Raised,
    /// It was there and its own check has now run and found nothing.
    Cleared,
}

/// A change worth telling somebody about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announcement {
    /// The condition's stable key.
    pub key: String,
    /// Which way it went.
    pub change: Change,
    /// One line, for a notification's title or a log.
    pub summary: String,
}

/// **What is true after one pass**: every condition that was checked, and the
/// message for each one that is present.
#[derive(Debug, Default)]
pub struct Reading {
    /// Conditions that are present, and what to say about each.
    pub present: BTreeMap<String, String>,
    /// Every condition whose check ran, present or not. A key here and not in
    /// `present` is a condition that was **looked at and was fine**.
    pub evaluated: BTreeSet<String>,
    /// Key prefixes the pass enumerated exhaustively. A standing key under one
    /// of these and not in `present` is gone, even though no check named it —
    /// which is how a condition whose subject the operator DELETED clears.
    pub completed: BTreeSet<String>,
}

/// The condition raised when the configuration file itself will not load.
///
/// It is the only condition that exists **outside** a pass, because it is the
/// one that stops a pass from happening at all.
pub const CONFIG: &str = "config";

impl Reading {
    /// A reading of exactly one condition, for the caller that has no pass.
    #[must_use]
    pub fn of_one(key: &str, message: String) -> Self {
        Self {
            present: [(key.to_owned(), message)].into_iter().collect(),
            evaluated: [key.to_owned()].into_iter().collect(),
            completed: BTreeSet::new(),
        }
    }

    /// **Can this pass say anything about `key` at all?**
    ///
    /// Either its own check ran, or the pass enumerated the whole set it
    /// belongs to. Anything else is a question this pass did not answer, and a
    /// question that was not answered must never read as an answer of *no*.
    fn covers(&self, key: &str) -> bool {
        self.evaluated.contains(key)
            || self
                .completed
                .iter()
                .any(|prefix| key.starts_with(prefix.as_str()))
    }
}

/// Read one pass, plus the ledger it just updated, as conditions.
///
/// The ledger is taken **after** the pass rather than the values the pass
/// started with, so an application that was late and has just been backed up is
/// not reported late.
#[must_use]
pub fn read(pass: &Pass, ledger: &Ledger, now: u64) -> Reading {
    let mut reading = Reading {
        evaluated: pass.evaluated.clone(),
        completed: pass.completed.clone(),
        ..Reading::default()
    };

    for refusal in &pass.refusals {
        reading.present.insert(
            format!("refusal/{}", refusal.key),
            format!("Rivet Monitor is not backing anything up: {}", refusal.why),
        );
    }
    for alert in &pass.alerts {
        reading
            .present
            .insert(alert.key.clone(), alert.message.clone());
    }

    // Lateness, per application. **Only for applications this pass actually
    // watched** — one that was not reachable is not in `watched`, so its
    // lateness is neither raised nor cleared and the machine being off does not
    // read as the backup having recovered.
    for watched in &pass.watched {
        let key = format!("late/{}/{}", watched.host, watched.service);
        reading.evaluated.insert(key.clone());

        let last = ledger.last_success(&watched.host, &watched.service);
        // **Two scheduled times in a row have passed without a success** —
        // one missed is a reboot at the wrong minute, two is a backup that has
        // stopped. Never backed up on a schedule is late from the first pass
        // that saw it: it is the case most likely to be a misconfiguration,
        // and the one a quiet monitor would hide for the longest. Manual only
        // is never late. `Watched::late` is the one rule, and the icon uses it
        // too.
        if watched.late(last) {
            let how_long = last.map_or_else(
                || String::from("it has never been backed up"),
                |then| {
                    format!(
                        "two scheduled backups have passed without one, and the last was {}",
                        ledger::ago(then, now)
                    )
                },
            );
            reading.present.insert(
                key,
                format!(
                    "{}/{} is overdue: {how_long}.",
                    watched.host, watched.service
                ),
            );
        }
    }

    // **Health, as the machine folded it** — `Rivet ADR-045` §4.
    //
    // Only the MACHINE's own word is a condition, and that is deliberate.
    // `Rivet ADR-045` §4.3 puts the decision about whether a degraded application
    // matters in the machine's own document, as `critical`, and the supervisor
    // has already applied it. A monitor that raised a condition per degraded
    // service would be overruling that from the outside — and would be back to
    // an author's opinion of their own importance, which is the thing the
    // vocabulary took away.
    for (host, health) in &pass.health {
        let key = format!("health/{host}");
        reading.evaluated.insert(key.clone());
        if health.machine == "healthy" {
            continue;
        }
        let named: Vec<String> = health
            .services
            .iter()
            .filter(|s| s.critical && s.health != "healthy")
            .map(|s| match &s.caption {
                Some(caption) => format!("{} is {} — {caption}", s.name, s.health),
                None => format!("{} is {}", s.name, s.health),
            })
            .collect();
        let detail = if named.is_empty() {
            String::from("no critical service named a reason")
        } else {
            named.join("; ")
        };
        reading
            .present
            .insert(key, format!("{host} is {}: {detail}.", health.machine));
    }

    reading
}

/// **Compare what is true now with what was standing**, and say what changed.
///
/// `standing` is the persisted set of conditions the operator has already been
/// told about, keyed to when each was first announced. It is updated in place,
/// so the caller writes it back and a restart announces nothing it has already
/// announced.
///
/// A condition that is present stays standing whether or not its check ran.
/// A condition that is standing and whose check did **not** run stays standing.
/// Only a check that ran and found nothing clears one.
pub fn reconcile(
    standing: &mut BTreeMap<String, u64>,
    reading: &Reading,
    now: u64,
) -> Vec<Announcement> {
    let mut announcements = Vec::new();

    for (key, message) in &reading.present {
        if !standing.contains_key(key) {
            standing.insert(key.clone(), now);
            announcements.push(Announcement {
                key: key.clone(),
                change: Change::Raised,
                summary: message.clone(),
            });
        }
    }

    let cleared: Vec<String> = standing
        .keys()
        .filter(|key| reading.covers(key) && !reading.present.contains_key(*key))
        .cloned()
        .collect();
    for key in cleared {
        let since = standing.remove(&key);
        announcements.push(Announcement {
            summary: recovered(&key, since, now),
            key,
            change: Change::Cleared,
        });
    }

    announcements
}

/// **What to say when a condition goes away.**
///
/// The raised message is the failure's own words and is not kept — a standing
/// record that stored prose would be a second copy of a sentence that changes
/// with the build. So the recovery is written from the key, which is stable,
/// and it says what is true again rather than echoing what was wrong.
fn recovered(key: &str, since: Option<u64>, now: u64) -> String {
    let parts: Vec<&str> = key.split('/').collect();
    let what = match parts.as_slice() {
        [CONFIG, ..] => String::from("the configuration file loads again"),
        ["refusal", ..] => String::from("Rivet Monitor is backing up again"),
        ["monitor-refusal", ..] => String::from("health monitoring's settings are accepted again"),
        ["register", ..] => String::from("the fleet register can be read again"),
        ["offers", host, ..] => format!("{host} is answering again"),
        ["fetch" | "staging", host, ..] => format!("backups from {host} are working again"),
        ["health", host, ..] => format!("{host} says it is healthy again"),
        ["late", host, service, ..] => format!("{host}/{service} has been backed up again"),
        ["place" | "ledger" | "retention", host, service, ..] => {
            format!("{host}/{service} is being stored again")
        }
        ["override", host, service, ..] => {
            format!("the override for {host}/{service} is no longer wrong")
        }
        // The monitor's — `SPEC-02` §6.
        ["reach", machine] => format!("{machine} is answering captures again"),
        ["log-stalled", machine] => format!("{machine}'s journal is returning new records again"),
        ["card-stalled", machine] => {
            format!("{machine} is saving its clock to the card again, so its writes are reaching it")
        }
        ["frozen", machine] => format!("{machine}'s display is no longer frozen"),
        ["power", machine] => format!("{machine} reports no under-voltage since it booted"),
        ["capture", machine, "register"] => format!("{machine} is in the register again"),
        ["capture", machine, signal, "service"] => {
            format!("every `services` name on {machine}/{signal} is running")
        }
        ["capture", machine, signal] => format!("{machine}/{signal} is being captured again"),
        _ => format!("{key} is fixed"),
    };
    match since {
        Some(since) if now > since => format!("{what} — it had been that way {}.", elapsed(since, now)),
        _ => format!("{what}."),
    }
}

/// "for 3 days", "for 4 hours" — how long a condition stood.
fn elapsed(since: u64, now: u64) -> String {
    let seconds = now.saturating_sub(since);
    let days = seconds / 86_400;
    if days > 0 {
        return format!("for {days} day{}", if days == 1 { "" } else { "s" });
    }
    let hours = seconds / 3_600;
    if hours > 0 {
        return format!("for {hours} hour{}", if hours == 1 { "" } else { "s" });
    }
    let minutes = seconds / 60;
    if minutes > 0 {
        return format!("for {minutes} minute{}", if minutes == 1 { "" } else { "s" });
    }
    String::from("only briefly")
}

/// **One pass, settled**: read it, compare it with what the operator has
/// already been told, persist the new standing record, and hand back what
/// changed.
///
/// Called only by a pass that **acted** — `run-once` and the tray's worker.
/// `check` fetches nothing and runs fewer checks, so letting it settle would
/// let a diagnostic the operator typed consume the announcement that the pass
/// doing the work was about to make.
///
/// The second half of the return is a save failure. It matters more than it
/// looks: a standing record that was not written means the next pass announces
/// everything again, which is the behaviour this module exists to prevent, so
/// it is surfaced rather than swallowed.
pub fn settle(pass: &Pass, ledger: &mut Ledger, now: u64) -> (Vec<Announcement>, Option<String>) {
    let reading = read(pass, ledger, now);
    let announcements = reconcile(&mut ledger.standing, &reading, now);
    let failure = if announcements.is_empty() {
        None
    } else {
        ledger.save().err()
    };
    (announcements, failure)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;
    use crate::schedule::{Alert, Watched};

    fn reading(present: &[(&str, &str)], evaluated: &[&str]) -> Reading {
        Reading {
            present: present
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            evaluated: evaluated.iter().map(|k| (*k).to_owned()).collect(),
            completed: BTreeSet::new(),
        }
    }

    fn enumerated(prefixes: &[&str]) -> Reading {
        Reading {
            completed: prefixes.iter().map(|p| (*p).to_owned()).collect(),
            ..Reading::default()
        }
    }

    /// **The whole point.** A condition that has not changed produces nothing,
    /// however many passes run over it.
    #[test]
    fn a_standing_condition_is_announced_once_and_never_again() {
        let mut standing = BTreeMap::new();
        let now = reading(&[("fetch/rivet-01", "403")], &["fetch/rivet-01"]);

        let first = reconcile(&mut standing, &now, 100);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].change, Change::Raised);

        for tick in 1..50 {
            assert!(
                reconcile(&mut standing, &now, 100 + tick).is_empty(),
                "pass {tick} announced an unchanged fact"
            );
        }
    }

    /// The recovery is worth a word, so the operator knows to stop worrying.
    #[test]
    fn a_condition_that_clears_is_announced_once() {
        let mut standing = BTreeMap::new();
        reconcile(
            &mut standing,
            &reading(&[("fetch/rivet-01", "403")], &["fetch/rivet-01"]),
            100,
        );

        let fixed = reading(&[], &["fetch/rivet-01"]);
        let announced = reconcile(&mut standing, &fixed, 200);
        assert_eq!(announced.len(), 1);
        assert_eq!(announced[0].change, Change::Cleared);
        assert!(standing.is_empty());
        assert!(reconcile(&mut standing, &fixed, 300).is_empty());
    }

    /// **The one that would destroy trust.** A pass that never got as far as
    /// this check must not report it fixed.
    #[test]
    fn a_check_that_did_not_run_never_clears_a_condition() {
        let mut standing = BTreeMap::new();
        reconcile(
            &mut standing,
            &reading(&[("fetch/rivet-01", "403")], &["fetch/rivet-01"]),
            100,
        );

        // The register would not load: nothing about rivet-01 was examined.
        let blind = reading(&[("register", "the register could not be read")], &["register"]);
        let announced = reconcile(&mut standing, &blind, 200);

        assert_eq!(announced.len(), 1, "only the new problem");
        assert_eq!(announced[0].key, "register");
        assert_eq!(announced[0].change, Change::Raised);
        assert!(
            standing.contains_key("fetch/rivet-01"),
            "the unexamined condition is still standing"
        );
    }

    /// And it clears on the pass that can finally see it again.
    #[test]
    fn it_clears_on_the_pass_that_can_see_it_again() {
        let mut standing = BTreeMap::new();
        reconcile(
            &mut standing,
            &reading(&[("fetch/rivet-01", "403")], &["fetch/rivet-01"]),
            100,
        );
        reconcile(&mut standing, &reading(&[("register", "down")], &["register"]), 200);

        let seeing = reading(&[], &["register", "fetch/rivet-01"]);
        let announced = reconcile(&mut standing, &seeing, 300);
        assert_eq!(announced.len(), 2);
        assert!(announced.iter().all(|a| a.change == Change::Cleared));
        assert!(standing.is_empty());
    }

    /// **The condition whose subject the operator DELETED.**
    ///
    /// He removes the bad override from the file. The check that raised the
    /// condition never runs again, so `evaluated` can never name it — and
    /// without an enumerated prefix it would stand for ever, silently, and a
    /// re-added override would raise nothing because the key was still there.
    #[test]
    fn a_condition_whose_subject_was_deleted_clears_on_the_enumerated_prefix() {
        let mut standing = BTreeMap::new();
        reconcile(
            &mut standing,
            &reading(
                &[("override/rivet-01/gone-rivet/no-machine", "no such machine")],
                &["override/rivet-01/gone-rivet/no-machine"],
            ),
            100,
        );

        let announced = reconcile(&mut standing, &enumerated(&["override/"]), 200);
        assert_eq!(announced.len(), 1);
        assert_eq!(announced[0].change, Change::Cleared);
        assert!(standing.is_empty());
    }

    /// **The recovery says what is true again**, not what was wrong, and it
    /// says how long it had been that way.
    #[test]
    fn a_recovery_reads_as_a_sentence_and_carries_how_long_it_stood() {
        let mut standing = BTreeMap::new();
        let broken = reading(
            &[("late/rivet-01/tempmon-rivet", "overdue")],
            &["late/rivet-01/tempmon-rivet"],
        );
        reconcile(&mut standing, &broken, 0);

        let fixed = reading(&[], &["late/rivet-01/tempmon-rivet"]);
        let announced = reconcile(&mut standing, &fixed, 86_400 * 3);
        assert_eq!(
            announced[0].summary,
            "rivet-01/tempmon-rivet has been backed up again — it had been that way for 3 days."
        );
    }

    /// **The condition that exists outside a pass.** A configuration file that
    /// will not load stops every backup, and the operator who mistyped a key is
    /// not watching the icon.
    #[test]
    fn an_unreadable_configuration_announces_once_and_clears_when_it_parses() {
        let mut standing = BTreeMap::new();
        let broken = Reading::of_one(CONFIG, String::from("unknown key `foler`"));

        assert_eq!(reconcile(&mut standing, &broken, 100).len(), 1);
        assert!(reconcile(&mut standing, &broken, 130).is_empty());

        // A pass that runs at all is a configuration that loaded.
        let mut running = Pass::default();
        running.evaluated.insert(String::from(CONFIG));
        let announced = reconcile(
            &mut standing,
            &read(&running, &Ledger::default(), 200),
            200,
        );
        assert_eq!(announced.len(), 1);
        assert_eq!(announced[0].change, Change::Cleared);
        assert!(announced[0].summary.starts_with("the configuration file loads again"));
    }

    /// A refusal the operator has FIXED clears, even though the check that
    /// raised it — a missing key — is one that stops being asked.
    #[test]
    fn a_refusal_that_was_fixed_clears_on_the_enumerated_prefix() {
        let mut standing = BTreeMap::new();
        reconcile(
            &mut standing,
            &reading(
                &[("refusal/backups.folder", "no folder is configured")],
                &["refusal/backups.folder"],
            ),
            100,
        );

        let announced = reconcile(&mut standing, &enumerated(&["refusal/"]), 200);
        assert_eq!(announced.len(), 1);
        assert_eq!(announced[0].change, Change::Cleared);
    }

    /// A key with no shape this build knows still produces something readable
    /// rather than nothing.
    #[test]
    fn an_unrecognised_key_still_says_something() {
        assert_eq!(recovered("something/new", None, 0), "something/new is fixed.");
    }

    /// **The monitor's conditions recover in sentences too** (`SPEC-02` §6),
    /// and `capture/<machine>/register` is not read as a signal named
    /// "register".
    #[test]
    fn the_monitors_recoveries_read_as_sentences() {
        for (key, said) in [
            ("reach/rivet-01", "rivet-01 is answering captures again."),
            ("log-stalled/rivet-01", "rivet-01's journal is returning new records again."),
            (
                "card-stalled/rivet-01",
                "rivet-01 is saving its clock to the card again, so its writes are reaching it.",
            ),
            ("frozen/rivet-01", "rivet-01's display is no longer frozen."),
            ("power/rivet-01", "rivet-01 reports no under-voltage since it booted."),
            ("capture/rivet-01/register", "rivet-01 is in the register again."),
            ("capture/rivet-01/status", "rivet-01/status is being captured again."),
            (
                "capture/rivet-01/status/service",
                "every `services` name on rivet-01/status is running.",
            ),
        ] {
            assert_eq!(recovered(key, None, 0), said, "{key}");
        }
    }

    /// An enumerated prefix clears only what it covers.
    #[test]
    fn an_enumerated_prefix_does_not_reach_past_itself() {
        let mut standing = BTreeMap::new();
        reconcile(
            &mut standing,
            &reading(
                &[
                    ("override/a/b/no-machine", "gone"),
                    ("fetch/rivet-01", "403"),
                ],
                &["override/a/b/no-machine", "fetch/rivet-01"],
            ),
            100,
        );

        reconcile(&mut standing, &enumerated(&["override/"]), 200);
        assert!(standing.contains_key("fetch/rivet-01"));
        assert!(!standing.contains_key("override/a/b/no-machine"));
    }

    /// A restart announces nothing it has already announced, because the
    /// standing set is persisted rather than rebuilt.
    #[test]
    fn a_restart_re_announces_nothing() {
        let mut standing = BTreeMap::new();
        let now = reading(&[("fetch/rivet-01", "403")], &["fetch/rivet-01"]);
        reconcile(&mut standing, &now, 100);

        let text = toml::to_string(&standing).expect("a map of numbers");
        let mut reloaded: BTreeMap<String, u64> = toml::from_str(&text).expect("it round-trips");

        assert!(reconcile(&mut reloaded, &now, 400).is_empty());
    }

    // -- the reading itself ------------------------------------------------

    fn pass_with(alerts: &[(&str, &str)], watched: &[(&str, &str, Option<u64>)]) -> Pass {
        let mut pass = Pass::default();
        for (key, message) in alerts {
            pass.alerts.push(Alert {
                key: (*key).to_owned(),
                message: (*message).to_owned(),
            });
            pass.evaluated.insert((*key).to_owned());
        }
        for (host, service, late_unless_since) in watched {
            pass.watched.push(Watched {
                host: (*host).to_owned(),
                service: (*service).to_owned(),
                late_unless_since: *late_unless_since,
            });
        }
        pass
    }

    /// **Late is no success since the scheduled time before the most recent
    /// one** — the same rule the icon uses.
    #[test]
    fn an_application_is_late_only_without_a_success_since_the_time_before_last() {
        let pass = pass_with(&[], &[("rivet-01", "tempmon-rivet", Some(5_000))]);
        let mut ledger = Ledger::default();
        let key = crate::ledger::key("rivet-01", "tempmon-rivet");

        ledger.success.insert(key.clone(), 5_000);
        let inside = read(&pass, &ledger, 9_000);
        assert!(!inside.present.contains_key("late/rivet-01/tempmon-rivet"));
        assert!(inside.evaluated.contains("late/rivet-01/tempmon-rivet"));

        ledger.success.insert(key, 4_999);
        let past = read(&pass, &ledger, 9_000);
        assert!(past.present.contains_key("late/rivet-01/tempmon-rivet"));
    }

    /// **Manual only is never late**, even never backed up — but it is still
    /// evaluated, so a lateness it raised under an older schedule clears.
    #[test]
    fn manual_only_is_never_late_and_still_clears() {
        let pass = pass_with(&[], &[("rivet-01", "aura-rivet", None)]);
        let seen = read(&pass, &Ledger::default(), 10);
        assert!(!seen.present.contains_key("late/rivet-01/aura-rivet"));
        assert!(seen.evaluated.contains("late/rivet-01/aura-rivet"));
    }

    /// **Never backed up is late from the first pass that saw it** — the case
    /// most likely to be a misconfiguration, and the one a quiet monitor would
    /// hide for the longest.
    #[test]
    fn never_backed_up_is_late_immediately() {
        let pass = pass_with(&[], &[("rivet-01", "notes-rivet", Some(0))]);
        let seen = read(&pass, &Ledger::default(), 10);
        assert!(
            seen.present
                .get("late/rivet-01/notes-rivet")
                .expect("it is late")
                .contains("never been backed up")
        );
    }

    // -- health -------------------------------------------------------------

    fn with_health(machine: &str, services: &[(&str, &str, Option<&str>, bool)]) -> Pass {
        let mut pass = Pass::default();
        pass.health.push((
            String::from("rivet-01"),
            crate::tool::Health {
                machine: machine.to_owned(),
                services: services
                    .iter()
                    .map(|(name, health, caption, critical)| crate::tool::ServiceHealth {
                        name: (*name).to_owned(),
                        health: (*health).to_owned(),
                        caption: caption.map(str::to_owned),
                        critical: *critical,
                    })
                    .collect(),
            },
        ));
        pass
    }

    /// **A healthy machine raises nothing**, and the check still counts as run
    /// so a condition that has cleared can clear.
    #[test]
    fn a_healthy_machine_is_evaluated_and_absent() {
        let pass = with_health("healthy", &[("tempmon-rivet", "degraded", None, false)]);
        let seen = read(&pass, &Ledger::default(), 0);
        assert!(seen.evaluated.contains("health/rivet-01"));
        assert!(!seen.present.contains_key("health/rivet-01"));
    }

    /// **The machine's own word is the condition, and the caption is carried.**
    /// Only `critical` services are named, because the machine's document has
    /// already decided which ones reach the fold — `Rivet ADR-045` §4.3.
    #[test]
    fn a_degraded_machine_names_its_critical_services_and_their_captions() {
        let pass = with_health(
            "degraded",
            &[
                (
                    "tempmon-rivet",
                    "degraded",
                    Some("no broadcast from GVH5075_2222"),
                    true,
                ),
                ("notes-rivet", "degraded", Some("ignore me"), false),
            ],
        );
        let message = read(&pass, &Ledger::default(), 0).present["health/rivet-01"].clone();
        assert!(message.contains("rivet-01 is degraded"));
        assert!(message.contains("no broadcast from GVH5075_2222"));
        assert!(
            !message.contains("ignore me"),
            "a service the document does not call critical is not this machine's problem: \
             {message}"
        );
    }

    /// A machine that folded to a bad word with nothing critical to blame says
    /// so rather than producing an empty sentence.
    #[test]
    fn a_machine_with_no_critical_service_to_name_still_says_something() {
        let pass = with_health("unknown", &[("tempmon-rivet", "unknown", None, false)]);
        let message = read(&pass, &Ledger::default(), 0).present["health/rivet-01"].clone();
        assert!(message.contains("rivet-01 is unknown"));
        assert!(message.contains("no critical service named a reason"));
    }

    /// **A machine that did not answer is neither raised nor cleared.** It is
    /// absent from `pass.health` entirely, so the condition is not evaluated —
    /// and a machine that is merely switched off must not read as its health
    /// having recovered.
    #[test]
    fn a_machine_that_did_not_answer_leaves_its_health_condition_standing() {
        let mut standing = BTreeMap::new();
        reconcile(
            &mut standing,
            &read(&with_health("degraded", &[]), &Ledger::default(), 0),
            0,
        );
        assert!(standing.contains_key("health/rivet-01"));

        // The next pass could not reach the machine at all.
        let announced = reconcile(&mut standing, &read(&Pass::default(), &Ledger::default(), 60), 60);
        assert!(announced.is_empty());
        assert!(standing.contains_key("health/rivet-01"));
    }

    /// And it clears, in a sentence about what is true rather than what was
    /// wrong.
    #[test]
    fn a_machine_that_becomes_healthy_says_so() {
        let mut standing = BTreeMap::new();
        reconcile(
            &mut standing,
            &read(&with_health("degraded", &[]), &Ledger::default(), 0),
            0,
        );
        let announced = reconcile(
            &mut standing,
            &read(&with_health("healthy", &[]), &Ledger::default(), 60),
            60,
        );
        assert_eq!(announced.len(), 1);
        assert_eq!(announced[0].change, Change::Cleared);
        assert!(announced[0].summary.starts_with("rivet-01 says it is healthy again"));
    }

    /// A machine that did not answer is not in `watched`, so its lateness is
    /// neither raised nor cleared — the machine being off must not read as the
    /// backup having recovered.
    #[test]
    fn an_application_that_was_not_reached_is_not_evaluated_for_lateness() {
        let quiet = read(&Pass::default(), &Ledger::default(), 1_000_000);
        assert!(quiet.evaluated.is_empty());
        assert!(quiet.present.is_empty());
    }
}
