//! **The monitor's conditions** — `SPEC-02` §6, through `SPEC-01` §5.5's
//! announcement layer.
//!
//! Six of them, each a way a machine fails that nobody would otherwise see:
//!
//! | key | raised when | cleared when |
//! | :--- | :--- | :--- |
//! | `reach/<machine>` | `reach-misses` consecutive `missed` attempts, across all signals | any `answered` attempt |
//! | `log-stalled/<machine>` | answered log captures returned no record for `log-stalled-minutes` | a log capture returns a record |
//! | `card-stalled/<machine>` | no *"the running clock is written to the card"* record for `card-stalled-minutes`, while answering | one of those records arrives |
//! | `frozen/<machine>` | a holder SEEN DRAWING whose frame timestamp is then unchanged for `frozen-minutes` | the frame advances, the holder changes, nobody holds the display, or the holder was never seen drawing |
//! | `power/<machine>` | `under_voltage_now` or `under_voltage_since_boot` | a reading with both false |
//! | `capture/<machine>/<signal>` (and `…/register`, `…/<signal>/service`) | something on THIS side stopped a capture | that same capture next succeeds |
//!
//! # Two halves: what an attempt says, and who writes it down
//!
//! A [`Watch`] is one machine's memory — its run of misses, when its journal
//! last returned a record, which frame it last saw — and turns each attempt into
//! an [`alarm::Reading`]: the conditions that attempt **checked**, and which of
//! them it found. It is pure, and that is where every §6 rule is tested.
//!
//! A [`Record`] is the one thing that persists which conditions have been
//! announced. **It has exactly one writer**, because everything that changes it
//! does so inside its lock, and the file is replaced before the lock is let go.
//!
//! # Why the monitor has a file of its own, and not a table in `state.toml`
//!
//! `SPEC-01` §5.5 put the announced set in `state.toml`, and the backup pass
//! rewrites that file WHOLE — load, change, rename over (`crate::ledger`). The
//! monitor runs on other threads, one per machine (`SPEC-02` §3.2). Two writers
//! of one whole-file record lose updates: whichever renames last silently
//! discards what the other wrote, and a lost entry is a condition announced
//! twice or a recovery never said. The log cursor left `state.toml` for exactly
//! this reason (`SPEC-02` §4.2's History).
//!
//! So the monitor's conditions live in `monitor-state.toml`, beside
//! `state.toml`. **The two key sets are disjoint** — the backup pass raises
//! `config`, `refusal/`, `register`, `offers/`, `fetch/`, `staging/`, `place/`,
//! `ledger/`, `retention/`, `override/`, `late/` and `health/`; the monitor
//! raises only the six above — so neither pass's enumerated prefixes can reach
//! the other's conditions, and the two files never need to agree about anything.
//! Inside the monitor, every worker and the supervisor settle through one
//! `Arc<Record>`, whose mutex spans the read, the reconcile and the rename.
//!
//! A single owner of BOTH records was the alternative, and it buys nothing: the
//! backup pass is already the only writer of `state.toml` in a process, and a
//! channel between the two would couple the timer `SPEC-02` §3.1 keeps apart.
//!
//! # What "acted" means for a monitor, which has no passes
//!
//! `SPEC-01` §5.5.4: only a pass that ACTED settles the record. The monitor's
//! unit of action is **one capture attempt that was actually made** — a line
//! written, or a write that failed — and it settles **only the conditions that
//! attempt checked**. The supervisor acts too, on each tick whose configuration
//! was accepted: it settles `capture/<machine>/register` against the register
//! it is capturing with, and clears the conditions whose subject the file no
//! longer has ([`Record::subjects`]). A paused worker, a refused configuration
//! or a register never read settles nothing. `check` runs no monitor at all, so
//! it settles nothing here either.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::alarm::{self, Announcement, Reading};
use crate::capture::Outcome;
use crate::clock;
use crate::config::{Conditions, ResolvedMachine, SIGNALS, Signal};
use crate::monitor::LogSummary;

/// The monitor's announcement record, beside the configuration file and
/// `state.toml`.
pub const FILE: &str = "monitor-state.toml";

/// `monitor-state.toml`, beside a configuration file.
#[must_use]
pub fn beside(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(FILE)
}

// ---------------------------------------------------------------------------
// What one attempt says
// ---------------------------------------------------------------------------

/// One capture attempt, as the conditions need it.
#[derive(Debug, Clone, Copy)]
pub struct Attempted<'a> {
    pub machine: &'a str,
    pub signal: Signal,
    /// The PC's UTC capture time.
    pub captured_at: u64,
    pub outcome: &'a Outcome,
    /// Whether its lines reached the disk, and why not — `SPEC-02` §5.5.
    pub stored: Result<(), &'a str>,
    /// For an answered log capture: what came back, BEFORE the filter.
    pub log: Option<&'a LogSummary>,
    /// Every `services` filter on this machine — `SPEC-02` §2.4.
    pub filters: &'a [(Signal, Vec<String>)],
}

/// **One screenshot observation** — fed by every answered screenshot attempt
/// (`crate::monitor`).
///
/// `SPEC-02` §4.3 says what the call must report: the holder, the frame's own
/// timestamp as the board gave it, and a distinct "no holder". The timestamp
/// is compared as a value from the board and never against the PC's clock; how
/// LONG it has been unchanged is measured on `captured_at`, which is the PC's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// The application holding the display, or `None` when nothing holds it.
    pub holder: Option<String>,
    /// The frame's timestamp, verbatim. `None` when the board did not say.
    pub stamp: Option<String>,
    pub captured_at: u64,
}

/// **The one sentence Rivet Monitor matches in a Rivet message** — `SPEC-02`
/// §6.9.
///
/// `rivet-sysd` writes the running clock to the card once an hour, **waits for
/// that write**, and emits this record only when the write succeeded
/// (`Rivet cmd/rivet-sysd/src/clock.rs`, `Clock::tick`). So the record is
/// evidence that a write reached the card, which is the fact §6.9 needs and
/// nothing else on the control plane reports.
///
/// **It is matched as a prefix, in this one place**, and a test pins the
/// string. Rivet rewording the message would silently switch the condition off
/// — `SPEC-02` §6.9 says so, and says what would remove the coupling.
pub const CLOCK_WRITTEN: &str = "the running clock is written to the card";

/// The service that emits it. An application may not claim this `svc`
/// (`rivet-core::journal`'s `SUPERVISOR_SVC`), so the match cannot be forged
/// by something a `.rvt` installed.
pub const CLOCK_SVC: &str = "rivet-sysd";

/// **What one log answer's records say about the card** — `SPEC-02` §6.9,
/// measured in the BOARD's own monotonic seconds, never against the PC's clock.
///
/// Read from the records **before §2.4's filter**, like §6.3's `returned`: a
/// `services` filter that did not name `rivet-sysd` would otherwise delete the
/// evidence and turn the condition off.
#[derive(Debug, Clone, PartialEq)]
pub struct CardClock {
    /// The boot the newest record in the answer belongs to.
    pub boot: String,
    /// The newest record's `mono` — seconds since that boot.
    pub newest_mono: f64,
    /// The newest clock-write record's `mono`, **in that same boot**, when the
    /// answer carried one.
    pub written_mono: Option<f64>,
}

impl CardClock {
    /// How long ago the card was last written, in seconds, by the board's own
    /// monotonic clock — or, when this answer carries no clock record, how long
    /// the board has been up (which is what the caller may use only when the
    /// answer covers the boot from its start).
    #[must_use]
    pub fn age_seconds(&self) -> u64 {
        let seconds = self.newest_mono - self.written_mono.unwrap_or(0.0);
        if seconds.is_finite() && seconds > 0.0 {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                seconds as u64
            }
        } else {
            0
        }
    }
}

/// **Scan one answer's records for the card's clock** — the newest record's
/// boot and `mono`, and the newest [`CLOCK_WRITTEN`] record inside that boot.
///
/// `None` when nothing in the answer could be read as a record with a `boot`
/// and a `mono`: there is then no board-side measurement to make, and §6.9
/// falls back to the stretch it has observed on this PC's clock.
///
/// Records arrive oldest first and a backfill may span several boots; only the
/// newest boot is measured, because `mono` restarts at every boot.
#[must_use]
pub fn card_clock(records: &[String]) -> Option<CardClock> {
    let read: Vec<(String, f64, Option<String>, Option<String>)> = records
        .iter()
        .filter_map(|text| {
            let value: serde_json::Value = serde_json::from_str(text).ok()?;
            let boot = value.get("boot")?.as_str()?.to_string();
            let mono = value.get("mono")?.as_f64()?;
            if !mono.is_finite() {
                return None;
            }
            Some((
                boot,
                mono,
                value.get("svc").and_then(serde_json::Value::as_str).map(str::to_string),
                value.get("msg").and_then(serde_json::Value::as_str).map(str::to_string),
            ))
        })
        .collect();
    let boot = read.last()?.0.clone();
    let mut newest_mono = f64::MIN;
    let mut written_mono: Option<f64> = None;
    for (this_boot, mono, svc, msg) in &read {
        if *this_boot != boot {
            continue;
        }
        if *mono > newest_mono {
            newest_mono = *mono;
        }
        let wrote = svc.as_deref() == Some(CLOCK_SVC)
            && msg.as_deref().is_some_and(|m| m.starts_with(CLOCK_WRITTEN));
        if wrote && written_mono.is_none_or(|held| *mono > held) {
            written_mono = Some(*mono);
        }
    }
    Some(CardClock {
        boot,
        newest_mono,
        written_mono,
    })
}

/// The frame last seen, and when it was FIRST seen unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Shown {
    holder: String,
    stamp: String,
    since: u64,
    /// **Whether this holder has been seen to advance its frame** while it has
    /// held the display. Only a holder that was drawing can be said to have
    /// stopped; one that never advanced may simply not write frames at all.
    drawn: bool,
}

/// **One machine's memory between attempts.**
///
/// Kept in memory only, and that is deliberate. What must survive a restart is
/// *which conditions were announced* — the [`Record`] — so a restart repeats
/// nothing. A run of misses or a quiet journal restarts from zero, which can
/// only DELAY a raise by one window; it can never clear a standing condition,
/// because a fresh `Watch` reports a condition as looked-at only when its
/// clearing evidence arrives (an answer, a record, a changed frame).
#[derive(Debug, Default)]
pub struct Watch {
    /// Consecutive `missed` attempts, across every signal — §6.2.
    misses: u32,
    /// Since when answered log captures have returned nothing — §6.3.
    quiet_since: Option<u64>,
    /// The frame last seen — §6.4. Read only by [`Watch::screenshot`].
    shown: Option<Shown>,
    /// **When the card was last written to, as this PC's clock places it** —
    /// §6.9. Placed by subtracting the board's own measurement of how old that
    /// record already was, so a backfill of a boot whose last write was 21
    /// hours ago says 21 hours ago, not "just now".
    card_written_at: Option<u64>,
    /// The boot the last log answer's newest record was in — §6.9. A change of
    /// it is a reboot this run OBSERVED, which means the records after it
    /// cover the new boot from its start.
    card_boot: Option<String>,
    /// Whether a clock-write record has been seen at all this run — §6.9, for
    /// the sentence it raises with.
    card_seen: bool,
}

fn raise(reading: &mut Reading, key: String, message: String) {
    reading.evaluated.insert(key.clone());
    reading.present.insert(key, message);
}

fn fine(reading: &mut Reading, key: String) {
    reading.evaluated.insert(key);
}

impl Watch {
    /// **What one attempt checked, and what it found.**
    #[must_use]
    pub fn attempt(&mut self, conditions: Conditions, seen: &Attempted<'_>) -> Reading {
        let Attempted {
            machine,
            signal,
            captured_at: now,
            outcome,
            stored,
            log,
            filters,
        } = *seen;
        let mut reading = Reading::default();

        // §6.6 — this side. A miss says nothing about this side, so it settles
        // nothing here unless its own line could not be written.
        let capture = format!("capture/{machine}/{signal}");
        match (outcome, stored) {
            (Outcome::Failed(why), _) => raise(
                &mut reading,
                capture,
                format!(
                    "{machine}/{signal} could not be captured, and the problem is on this side, \
                     not the machine's: {why} (SPEC-02 §6.6)"
                ),
            ),
            (_, Err(why)) => raise(
                &mut reading,
                capture,
                format!(
                    "{machine}/{signal} was attempted and its record could not be written: {why} \
                     (SPEC-02 §5.5)"
                ),
            ),
            (Outcome::Answered { .. }, Ok(())) => fine(&mut reading, capture),
            (Outcome::Missed(_), Ok(())) => {}
        }

        // §6.2 — consecutive misses across ALL signals, in the order attempted.
        // A failure neither counts nor resets: it says nothing about the machine.
        let reach = format!("reach/{machine}");
        match outcome {
            Outcome::Answered { .. } => {
                self.misses = 0;
                fine(&mut reading, reach);
            }
            Outcome::Missed(why) => {
                self.misses = self.misses.saturating_add(1);
                if conditions.reach_misses > 0 && self.misses >= conditions.reach_misses {
                    raise(
                        &mut reading,
                        reach,
                        format!(
                            "{machine} has not answered {} capture(s) in a row, across every \
                             signal it is asked for; the latest, {signal}: {why} (SPEC-02 §6.2)",
                            self.misses
                        ),
                    );
                }
            }
            Outcome::Failed(_) => {}
        }

        // §6.3 — judged on `returned`, BEFORE the filter, and only on a log
        // capture the machine answered: a machine that is not answering is
        // `reach/`'s to report, and this condition says nothing about it.
        if signal == Signal::Log
            && let (Outcome::Answered { .. }, Some(log)) = (outcome, log)
        {
            let key = format!("log-stalled/{machine}");
            if log.returned > 0 {
                self.quiet_since = Some(now);
                fine(&mut reading, key);
            } else {
                let since = *self.quiet_since.get_or_insert(now);
                let window = conditions.log_stalled_minutes.saturating_mul(60);
                if conditions.log_stalled_minutes > 0 && now.saturating_sub(since) >= window {
                    raise(
                        &mut reading,
                        key,
                        format!(
                            "{machine} is answering, and its journal has returned no new record \
                             since {} — {} minute(s) or more. Alive, but its journal stopped \
                             (SPEC-02 §6.3).",
                            clock::utc_stamp(since),
                            now.saturating_sub(since) / 60
                        ),
                    );
                }
            }
            self.card_stalled(machine, conditions, now, log, &mut reading);
        }

        if let Outcome::Answered { value, .. } = outcome {
            // §6.5 — the since-boot flag catches a dip between two readings.
            if signal == Signal::Thermal {
                let key = format!("power/{machine}");
                match under_voltage(value) {
                    Some((now_flag, boot_flag)) if now_flag || boot_flag => raise(
                        &mut reading,
                        key,
                        format!(
                            "{machine} reports UNDER-VOLTAGE ({}). A marginal supply is the \
                             leading cause of card corruption on a Pi, and the since-boot flag is \
                             the machine's own authority for this boot (SPEC-02 §6.5).",
                            match (now_flag, boot_flag) {
                                (true, true) => "now, and since boot",
                                (true, false) => "now",
                                _ => "at some point since boot, not at this reading",
                            }
                        ),
                    ),
                    Some(_) => fine(&mut reading, key),
                    // Unmeasured: neither raises nor clears.
                    None => {}
                }
            }
            // §2.4 — judged on each answered status.
            if signal == Signal::Status {
                for (filtered, missing) in unmatched_services(filters, value) {
                    let key = format!("capture/{machine}/{filtered}/service");
                    if missing.is_empty() {
                        fine(&mut reading, key);
                    } else {
                        raise(
                            &mut reading,
                            key,
                            format!(
                                "{machine}/{filtered}: `services` names {} and {machine}'s latest \
                                 status answer runs no service by that name. A renamed service \
                                 turns a filter into one that matches nothing (SPEC-02 §2.4).",
                                missing.join(", ")
                            ),
                        );
                    }
                }
            }
        }
        reading
    }

    /// **§6.9 — `card-stalled/<machine>`**, judged on one answered log capture.
    ///
    /// The fact it needs is *when this machine last got a write through to its
    /// card*, and the only place that fact is stated is `rivet-sysd`'s hourly
    /// record ([`CLOCK_WRITTEN`]), which is emitted **after** the write
    /// completes. So the condition is: **that record is older than
    /// `card-stalled-minutes` while the machine is still answering.**
    ///
    /// # Where the window is measured from, and what may clear it
    ///
    /// **The anchor is placed in PC time, from a board-side measurement.** An
    /// answer's records carry `mono`, the board's monotonic seconds since boot,
    /// so the distance between the newest clock record and the newest record of
    /// any kind is an elapsed duration measured entirely on the board. Taking
    /// that distance off `now` places the write on this PC's clock without ever
    /// comparing the two clocks — and it is what lets the FIRST capture judge
    /// honestly, because that capture is `get log latest all`, the whole current
    /// boot, hourly clock records and all.
    ///
    /// Three anchors, in order of what the answer supports:
    ///
    /// * **a clock record in this answer** — the anchor is when it was written,
    ///   and this is the only thing that CLEARS the condition;
    /// * **no clock record, in an answer that covers the boot from its start** —
    ///   the first capture of a run, or one whose records cross a reboot this
    ///   run observed — the anchor is the boot itself: nothing has been written
    ///   to the card in this whole boot, which after the window is exactly the
    ///   condition. (A board that has just booted is under the window, and
    ///   `rivet-sysd` writes nothing until its first hour — `clock.rs`'s own
    ///   test — so this does not raise on a fresh boot.)
    /// * **neither** — Rivet Monitor started with a cursor part-way through a
    ///   boot, so the backfill may hold no clock record and the boot's earlier
    ///   ones are behind the cursor. The anchor is then the first log capture
    ///   this run observed, exactly as §6.3 does it: the condition can still
    ///   raise once the run has watched for the whole window, and it can never
    ///   clear on ignorance. A restart can therefore delay a raise by up to one
    ///   window (§6.7 already says a restart may do that); it cannot cause one.
    ///
    /// **Nothing is marked evaluated unless a clock record was actually seen or
    /// the window has run out.** §6.1's rule: a condition whose check did not
    /// run has not cleared.
    fn card_stalled(
        &mut self,
        machine: &str,
        conditions: Conditions,
        now: u64,
        log: &LogSummary,
        reading: &mut Reading,
    ) {
        if conditions.card_stalled_minutes == 0 {
            return;
        }
        let key = format!("card-stalled/{machine}");
        let window = conditions.card_stalled_minutes.saturating_mul(60);
        let mut wrote_in_this_answer = false;
        if let Some(card) = &log.card {
            // An answer covers the boot from its start when it IS the boot (a
            // first capture), or when its records crossed into a boot this run
            // watched begin.
            let covers_the_boot =
                log.first || matches!(&self.card_boot, Some(previous) if previous != &card.boot);
            if card.written_mono.is_some() {
                self.card_written_at = Some(now.saturating_sub(card.age_seconds()));
                self.card_seen = true;
                wrote_in_this_answer = true;
            } else if covers_the_boot {
                self.card_written_at = Some(now.saturating_sub(card.age_seconds()));
                self.card_seen = false;
            }
            self.card_boot = Some(card.boot.clone());
        }
        let since = *self.card_written_at.get_or_insert(now);
        let age = now.saturating_sub(since);
        if age >= window {
            let how = if self.card_seen {
                format!(
                    "the last time it got one through was {}, {} minute(s) ago",
                    clock::utc_stamp(since),
                    age / 60
                )
            } else {
                format!(
                    "no such record has arrived since {}, {} minute(s) ago",
                    clock::utc_stamp(since),
                    age / 60
                )
            };
            raise(
                reading,
                key,
                format!(
                    "{machine} is answering captures and has not saved its clock to the card for \
                     {} minute(s): {how}. rivet-sysd writes the clock hourly and waits for the \
                     write, so this is a machine whose writes are no longer reaching its card \
                     while it runs on — the shape of the 2026-09-14 incident, where the screen \
                     froze 21 hours after the last durable write (SPEC-02 §6.9, ADR-002 §1).",
                    conditions.card_stalled_minutes
                ),
            );
        } else if wrote_in_this_answer {
            fine(reading, key);
        }
    }

    /// **§6.4 — one screenshot observation**, from an answered screenshot
    /// attempt whose answer could be read.
    #[must_use]
    pub fn screenshot(&mut self, machine: &str, conditions: Conditions, frame: &Frame) -> Reading {
        let key = format!("frozen/{machine}");
        let mut reading = Reading::default();
        match (&frame.holder, &frame.stamp) {
            // No holder never raises it, and clears it.
            (None, _) => {
                self.shown = None;
                fine(&mut reading, key);
            }
            // A holder with no timestamp cannot be judged either way.
            (Some(_), None) => {}
            (Some(holder), Some(stamp)) => match &self.shown {
                Some(shown) if shown.holder == *holder && shown.stamp == *stamp => {
                    let unchanged = frame.captured_at.saturating_sub(shown.since);
                    if conditions.frozen_minutes > 0
                        && unchanged >= conditions.frozen_minutes.saturating_mul(60)
                    {
                        if shown.drawn {
                            raise(
                                &mut reading,
                                key,
                                format!(
                                    "{machine}'s display is held by {holder}, which was drawing \
                                     and whose frame ({stamp}) has not changed for {} minute(s) \
                                     (SPEC-02 §6.4).",
                                    unchanged / 60
                                ),
                            );
                        } else {
                            // **Never seen drawing is not evidence of a freeze.**
                            // A holder that writes no frames — Aura since the
                            // compositor took the screen — leaves one old file
                            // that never changes. Judged, and not frozen.
                            fine(&mut reading, key);
                        }
                    }
                }
                // It advanced, or the holder changed: looked at, and not frozen.
                Some(shown) => {
                    let drawn = shown.holder == *holder;
                    fine(&mut reading, key);
                    self.shown = Some(Shown {
                        holder: holder.clone(),
                        stamp: stamp.clone(),
                        since: frame.captured_at,
                        drawn,
                    });
                }
                // First sight — nothing is known about what came before it, so
                // a condition standing from before a restart is not cleared.
                None => {
                    self.shown = Some(Shown {
                        holder: holder.clone(),
                        stamp: stamp.clone(),
                        since: frame.captured_at,
                        drawn: false,
                    });
                }
            },
        }
        reading
    }
}

/// `throttle.under_voltage_now` and `throttle.under_voltage_since_boot`, or
/// `None` when the flags were not measured — `throttle_unmeasured`, or no
/// readable pair at all. **Absent flags are not clear flags.**
#[must_use]
pub fn under_voltage(thermal: &serde_json::Value) -> Option<(bool, bool)> {
    if thermal.get("throttle_unmeasured").is_some() {
        return None;
    }
    let throttle = thermal.get("throttle")?;
    let now = throttle.get("under_voltage_now")?.as_bool()?;
    let boot = throttle.get("under_voltage_since_boot")?.as_bool()?;
    Some((now, boot))
}

/// Per filter, the names it holds that a `status` answer does not list.
#[must_use]
pub fn unmatched_services(
    filters: &[(Signal, Vec<String>)],
    status: &serde_json::Value,
) -> Vec<(Signal, Vec<String>)> {
    let running: BTreeSet<&str> = status
        .get("services")
        .and_then(serde_json::Value::as_array)
        .map(|listed| {
            listed
                .iter()
                .filter_map(|s| s.get("name").and_then(serde_json::Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    filters
        .iter()
        .map(|(signal, names)| {
            (
                *signal,
                names
                    .iter()
                    .filter(|name| !running.contains(name.as_str()))
                    .cloned()
                    .collect(),
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Which subjects still exist — SPEC-02 §6.1
// ---------------------------------------------------------------------------

/// What one configured machine can have conditions about.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Watched {
    conditions: Conditions,
    signals: BTreeSet<Signal>,
    filtered: BTreeSet<Signal>,
}

/// **Every subject the accepted configuration still has** — `SPEC-02` §6.1.
///
/// A standing condition whose subject is not here is gone: the machine entry
/// was removed, the signal it exists because of was removed, or the condition
/// was switched off with `0` or `false`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Subjects {
    machines: BTreeMap<String, Watched>,
}

impl Subjects {
    /// From the resolved `[[monitor.machine]]` entries. An empty list — or no
    /// `[monitor]` table at all — has no subjects.
    #[must_use]
    pub fn of(machines: &[ResolvedMachine]) -> Self {
        let mut out = Self::default();
        for machine in machines {
            out.machines
                .entry(machine.name.clone())
                .or_insert_with(|| Watched {
                    conditions: machine.conditions,
                    signals: machine.signals.iter().map(|s| s.signal).collect(),
                    filtered: machine
                        .signals
                        .iter()
                        .filter(|s| s.services.is_some())
                        .map(|s| s.signal)
                        .collect(),
                });
        }
        out
    }

    /// `None` when `key` names a subject that still exists; otherwise why it
    /// does not, in words.
    #[must_use]
    pub fn why_not(&self, key: &str) -> Option<String> {
        let parts: Vec<&str> = key.split('/').collect();
        let Some(name) = parts.get(1) else {
            return Some(format!("{key} is not a condition the monitor raises"));
        };
        let Some(watched) = self.machines.get(*name) else {
            return Some(format!("{name} is no longer a [[monitor.machine]]"));
        };
        let requires = |signal: Signal, off: bool, setting: &str| {
            if !watched.signals.contains(&signal) {
                Some(format!("{name} no longer captures {signal}"))
            } else if off {
                Some(format!("{setting} switches it off for {name}"))
            } else {
                None
            }
        };
        match parts.as_slice() {
            ["reach", _] if watched.conditions.reach_misses == 0 => {
                Some(format!("reach-misses = 0 switches it off for {name}"))
            }
            ["reach", _] if watched.signals.is_empty() => {
                Some(format!("{name} is no longer asked for any signal"))
            }
            ["reach", _] => None,
            ["log-stalled", _] => requires(
                Signal::Log,
                watched.conditions.log_stalled_minutes == 0,
                "log-stalled-minutes = 0",
            ),
            ["frozen", _] => requires(
                Signal::Screenshot,
                watched.conditions.frozen_minutes == 0,
                "frozen-minutes = 0",
            ),
            ["card-stalled", _] => requires(
                Signal::Log,
                watched.conditions.card_stalled_minutes == 0,
                "card-stalled-minutes = 0",
            ),
            ["power", _] => requires(Signal::Thermal, !watched.conditions.power, "power = false"),
            ["capture", _, "register"] => None,
            ["capture", _, signal] => match signal_named(signal) {
                Some(signal) if watched.signals.contains(&signal) => None,
                Some(signal) => Some(format!("{name} no longer captures {signal}")),
                None => Some(format!("{key} is not a condition the monitor raises")),
            },
            ["capture", _, signal, "service"] => match signal_named(signal) {
                Some(signal) if watched.filtered.contains(&signal) => None,
                Some(signal) => Some(format!("{name}/{signal} no longer has a `services` filter")),
                None => Some(format!("{key} is not a condition the monitor raises")),
            },
            _ => Some(format!("{key} is not a condition the monitor raises")),
        }
    }

    /// A reading with every key this configuration has no subject for removed
    /// — so a worker still holding an old plan cannot raise a condition the
    /// supervisor has just cleared as deleted.
    fn restrict(&self, reading: &Reading) -> Reading {
        Reading {
            present: reading
                .present
                .iter()
                .filter(|(key, _)| self.why_not(key).is_none())
                .map(|(key, message)| (key.clone(), message.clone()))
                .collect(),
            evaluated: reading
                .evaluated
                .iter()
                .filter(|key| self.why_not(key).is_none())
                .cloned()
                .collect(),
            completed: reading.completed.clone(),
        }
    }
}

fn signal_named(name: &str) -> Option<Signal> {
    SIGNALS.into_iter().find(|signal| signal.name() == name)
}

// ---------------------------------------------------------------------------
// The record — one writer
// ---------------------------------------------------------------------------

/// What settling produced.
#[derive(Debug, Default)]
pub struct Settled {
    /// Each condition that appeared or disappeared, once.
    pub announcements: Vec<Announcement>,
    /// Something the operator should read: the record could not be written or
    /// read. Said once per distinct problem.
    pub trouble: Option<String>,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct OnDisk {
    /// **Conditions already announced**, and when each was first announced —
    /// the same shape as `state.toml`'s `standing`.
    #[serde(default)]
    standing: BTreeMap<String, u64>,
}

#[derive(Debug)]
struct Inner {
    path: Option<PathBuf>,
    standing: BTreeMap<String, u64>,
    /// The file exists and could not be read. Nothing is settled while this
    /// is set: reading it as empty would announce every standing condition
    /// again, then overwrite the record of what really happened.
    unreadable: Option<String>,
    /// The subjects the supervisor last accepted. `None` until it has.
    subjects: Option<Subjects>,
    /// The trouble last said, so an unchanged problem is said once.
    said: Option<String>,
}

/// **The monitor's announcement record, and its only writer.**
#[derive(Debug)]
pub struct Record {
    inner: Mutex<Inner>,
}

/// **The monitor's standing conditions, read from beside `config_path`** — for
/// a reader that is not the [`Record`]'s owner (the tray's problems list,
/// `SPEC-01` §5.8). Read-only; the record's one writer is still the monitor.
///
/// # Errors
///
/// A record that exists and will not read.
pub fn read_standing(config_path: &Path) -> Result<BTreeMap<String, u64>, String> {
    load(&beside(config_path))
}

fn load(path: &Path) -> Result<BTreeMap<String, u64>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str::<OnDisk>(&text)
            .map(|disk| disk.standing)
            .map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

impl Inner {
    /// Retry a record that could not be read. `Some` is the trouble to report
    /// instead of settling.
    fn ensure_loaded(&mut self) -> Option<Settled> {
        self.unreadable.as_ref()?;
        let path = self.path.clone()?;
        match load(&path) {
            Ok(standing) => {
                self.standing = standing;
                self.unreadable = None;
                self.said = None;
                None
            }
            Err(why) => {
                let trouble = format!(
                    "the monitor's announcement record could not be read, so its conditions are \
                     NOT announced until it can be — reading it as empty would announce every \
                     standing condition again and then overwrite what really happened: {why}"
                );
                self.unreadable = Some(why);
                let fresh = self.said.as_deref() != Some(trouble.as_str());
                self.said = Some(trouble.clone());
                Some(Settled {
                    announcements: Vec::new(),
                    trouble: fresh.then_some(trouble),
                })
            }
        }
    }

    /// Write the record whole, through a temporary file. Called only with the
    /// lock held, which is what makes it the only writer.
    fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        let text = toml::to_string_pretty(&OnDisk {
            standing: self.standing.clone(),
        })
        .map_err(|e| format!("the record will not serialise: {e}"))?;
        let temporary = path.with_extension("toml.tmp");
        std::fs::write(&temporary, text.as_bytes())
            .map_err(|e| format!("cannot write {}: {e}", temporary.display()))?;
        std::fs::rename(&temporary, path)
            .map_err(|e| format!("cannot rename {}: {e}", temporary.display()))
    }

    fn reconcile(&mut self, reading: &Reading, now: u64) -> Settled {
        let announcements = alarm::reconcile(&mut self.standing, reading, now);
        let trouble = if announcements.is_empty() {
            None
        } else {
            self.save().err().map(|why| {
                format!(
                    "the monitor's record of what has already been announced could not be \
                     written, so a restart will repeat these: {why}"
                )
            })
        };
        Settled {
            announcements,
            trouble,
        }
    }
}

impl Record {
    /// Open the record at `path` — `None` keeps it in memory only, for a
    /// system with no configuration directory.
    #[must_use]
    pub fn open(path: Option<PathBuf>) -> Self {
        let (standing, unreadable) = match path.as_deref().map(load) {
            None => (BTreeMap::new(), None),
            Some(Ok(standing)) => (standing, None),
            Some(Err(why)) => (BTreeMap::new(), Some(why)),
        };
        Self {
            inner: Mutex::new(Inner {
                path,
                standing,
                unreadable,
                subjects: None,
                said: None,
            }),
        }
    }

    /// **The supervisor's settle: what the accepted configuration still has.**
    ///
    /// Every standing condition whose subject is gone clears, once, saying why.
    /// From here on a worker's reading about a subject that is not in
    /// `subjects` is dropped.
    pub fn subjects(&self, subjects: Subjects, now: u64) -> Settled {
        let Ok(mut guard) = self.inner.lock() else {
            return poisoned();
        };
        let inner = &mut *guard;
        if let Some(trouble) = inner.ensure_loaded() {
            inner.subjects = Some(subjects);
            return trouble;
        }
        let gone: BTreeMap<String, String> = inner
            .standing
            .keys()
            .filter_map(|key| subjects.why_not(key).map(|why| (key.clone(), why)))
            .collect();
        inner.subjects = Some(subjects);
        if gone.is_empty() {
            return Settled::default();
        }
        let reading = Reading {
            evaluated: gone.keys().cloned().collect(),
            ..Reading::default()
        };
        let mut settled = inner.reconcile(&reading, now);
        for one in &mut settled.announcements {
            if let Some(why) = gone.get(&one.key) {
                one.summary = format!("{} is no longer watched, so it is cleared: {why}.", one.key);
            }
        }
        settled
    }

    /// **A worker's (or the supervisor's) settle of what it checked.**
    ///
    /// Nothing is settled before the supervisor has accepted a configuration,
    /// and a key whose subject that configuration does not have is dropped.
    pub fn settle(&self, reading: &Reading, now: u64) -> Settled {
        let Ok(mut guard) = self.inner.lock() else {
            return poisoned();
        };
        let inner = &mut *guard;
        if let Some(trouble) = inner.ensure_loaded() {
            return trouble;
        }
        let Some(subjects) = &inner.subjects else {
            return Settled::default();
        };
        let restricted = subjects.restrict(reading);
        inner.reconcile(&restricted, now)
    }

    /// The conditions standing right now, for a test.
    #[cfg(test)]
    fn standing(&self) -> BTreeMap<String, u64> {
        self.inner
            .lock()
            .map(|inner| inner.standing.clone())
            .unwrap_or_default()
    }
}

impl Default for Record {
    /// A record kept in memory only.
    fn default() -> Self {
        Self::open(None)
    }
}

fn poisoned() -> Settled {
    Settled {
        announcements: Vec::new(),
        trouble: Some(
            "the monitor's announcement record was poisoned by a worker that panicked, so nothing \
             more is announced until Rivet Monitor restarts"
                .to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use std::sync::{Arc, Barrier};

    use super::*;
    use crate::alarm::Change;
    use crate::config::ResolvedSignal;
    use crate::ledger::Ledger;

    const ON: Conditions = Conditions {
        reach_misses: 3,
        log_stalled_minutes: 10,
        frozen_minutes: 5,
        card_stalled_minutes: 70,
        power: true,
    };

    const M: &str = "rivet-01";

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("rivet-monitor-tests")
            .join("conditions")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    fn answered(text: &str) -> Outcome {
        Outcome::Answered {
            raw: text.to_string(),
            value: serde_json::from_str(text).expect("a JSON answer"),
        }
    }

    fn missed() -> Outcome {
        Outcome::Missed("no answer within 20 s".to_string())
    }

    fn failed() -> Outcome {
        Outcome::Failed("cargo-rivet: `get` has no subject".to_string())
    }

    const STATUS: &str = r#"{"subject":"status","health":"healthy","services":[{"name":"aura-renderer"},{"name":"tempmon-rivet"}]}"#;

    fn attempt(watch: &mut Watch, signal: Signal, at: u64, outcome: &Outcome) -> Reading {
        watch.attempt(
            ON,
            &Attempted {
                machine: M,
                signal,
                captured_at: at,
                outcome,
                stored: Ok(()),
                log: None,
                filters: &[],
            },
        )
    }

    fn log_summary(returned: usize, stored: usize) -> LogSummary {
        LogSummary {
            returned,
            stored,
            unreadable_records: 0,
            first: false,
            boot_gone: false,
            cursor_moved: returned > 0,
            card: None,
        }
    }

    const LOG_ANSWER: &str = r#"{"subject":"log","records":[],"returned":0}"#;

    fn log_attempt(watch: &mut Watch, at: u64, returned: usize, stored: usize) -> Reading {
        let outcome = answered(LOG_ANSWER);
        let summary = log_summary(returned, stored);
        watch.attempt(
            ON,
            &Attempted {
                machine: M,
                signal: Signal::Log,
                captured_at: at,
                outcome: &outcome,
                stored: Ok(()),
                log: Some(&summary),
                filters: &[],
            },
        )
    }

    fn resolved(name: &str, conditions: Conditions, signals: &[Signal]) -> ResolvedMachine {
        ResolvedMachine {
            name: name.to_string(),
            conditions,
            signals: signals
                .iter()
                .map(|&signal| ResolvedSignal {
                    signal,
                    every_seconds: 60,
                    keep_days: 14,
                    keep_frames: signal.takes_keep_frames().then_some(3),
                    services: None,
                })
                .collect(),
        }
    }

    const EVERY: [Signal; 4] = [Signal::Status, Signal::Log, Signal::Thermal, Signal::Screenshot];

    /// A record in a scratch folder, accepting one machine capturing every
    /// signal a condition needs.
    fn record(name: &str) -> (Record, PathBuf) {
        let path = scratch(name).join(FILE);
        let record = Record::open(Some(path.clone()));
        record.subjects(Subjects::of(&[resolved(M, ON, &EVERY)]), 0);
        (record, path)
    }

    fn keys(settled: &Settled) -> Vec<(String, Change)> {
        settled
            .announcements
            .iter()
            .map(|a| (a.key.clone(), a.change))
            .collect()
    }

    // -- reach/ ---------------------------------------------------------------

    /// **§6.2: `reach-misses` consecutive misses, counted ACROSS signals.**
    #[test]
    fn reach_is_raised_after_the_configured_misses_across_every_signal() {
        let mut watch = Watch::default();
        let first = attempt(&mut watch, Signal::Status, 0, &missed());
        let second = attempt(&mut watch, Signal::Thermal, 30, &missed());
        assert!(!first.present.contains_key("reach/rivet-01"));
        assert!(!second.present.contains_key("reach/rivet-01"));
        assert!(
            !second.evaluated.contains("reach/rivet-01"),
            "a miss short of the threshold is not a clearance"
        );
        let third = attempt(&mut watch, Signal::Log, 60, &missed());
        assert!(third.present["reach/rivet-01"].contains("3 capture(s) in a row"));
    }

    /// **§6.2: a `failed` attempt neither counts nor resets.**
    #[test]
    fn a_failed_attempt_neither_counts_nor_resets_reach() {
        let mut watch = Watch::default();
        attempt(&mut watch, Signal::Status, 0, &missed());
        attempt(&mut watch, Signal::Thermal, 1, &missed());
        let between = attempt(&mut watch, Signal::Memory, 2, &failed());
        assert!(!between.present.contains_key("reach/rivet-01"), "a failure did not count");
        assert!(!between.evaluated.contains("reach/rivet-01"), "a failure did not reset");
        let third = attempt(&mut watch, Signal::Status, 3, &missed());
        assert!(
            third.present.contains_key("reach/rivet-01"),
            "the two misses before the failure still count"
        );
    }

    /// **§6.2: cleared by ANY answered attempt**, once.
    #[test]
    fn any_answered_attempt_clears_reach_once() {
        let (record, _) = record("reach-clears");
        let mut watch = Watch::default();
        let mut announced = Vec::new();
        for at in 0..6 {
            announced.extend(keys(&record.settle(&attempt(&mut watch, Signal::Status, at, &missed()), at)));
        }
        assert_eq!(announced, vec![("reach/rivet-01".to_string(), Change::Raised)]);

        let cleared = record.settle(&attempt(&mut watch, Signal::Thermal, 10, &answered(r#"{"subject":"thermal"}"#)), 10);
        assert_eq!(keys(&cleared), vec![("reach/rivet-01".to_string(), Change::Cleared)]);
        let again = record.settle(&attempt(&mut watch, Signal::Status, 11, &answered(STATUS)), 11);
        assert!(keys(&again).is_empty());
    }

    /// `reach-misses = 0` never raises.
    #[test]
    fn reach_misses_of_zero_never_raises() {
        let mut watch = Watch::default();
        let off = Conditions { reach_misses: 0, ..ON };
        for at in 0..20 {
            let outcome = missed();
            let reading = watch.attempt(
                off,
                &Attempted {
                    machine: M,
                    signal: Signal::Status,
                    captured_at: at,
                    outcome: &outcome,
                    stored: Ok(()),
                    log: None,
                    filters: &[],
                },
            );
            assert!(reading.present.is_empty(), "{reading:?}");
        }
    }

    // -- log-stalled/ -----------------------------------------------------------

    /// **§6.3: raised when answered log captures return nothing for the window.**
    #[test]
    fn log_stalled_is_raised_when_the_journal_returns_nothing_for_the_window() {
        let mut watch = Watch::default();
        assert!(log_attempt(&mut watch, 1_000, 7, 1).evaluated.contains("log-stalled/rivet-01"));
        for at in (1_060..1_600).step_by(60) {
            let reading = log_attempt(&mut watch, at, 0, 0);
            assert!(!reading.present.contains_key("log-stalled/rivet-01"), "at {at}");
            assert!(!reading.evaluated.contains("log-stalled/rivet-01"), "at {at}: quiet is not fine");
        }
        let stalled = log_attempt(&mut watch, 1_600, 0, 0);
        assert!(stalled.present["log-stalled/rivet-01"].contains("journal stopped"));
    }

    /// **§6.3: judged on `returned`, BEFORE the filter** — a narrow filter on a
    /// quiet service stores nothing for an hour and is not a stall.
    #[test]
    fn a_narrow_filter_cannot_fake_a_stall() {
        let mut watch = Watch::default();
        for at in (0..3_600).step_by(60) {
            let reading = log_attempt(&mut watch, at, 7, 0);
            assert!(!reading.present.contains_key("log-stalled/rivet-01"), "at {at}");
            assert!(reading.evaluated.contains("log-stalled/rivet-01"));
        }
    }

    /// **§6.3: not raised while the machine does not answer** — that is
    /// `reach/`'s, and a failed log capture says nothing about the journal.
    #[test]
    fn log_stalled_is_not_raised_while_the_machine_does_not_answer() {
        let mut watch = Watch::default();
        log_attempt(&mut watch, 0, 5, 5);
        for at in (60_u64..3_600).step_by(60) {
            let outcome = if at.is_multiple_of(120) { missed() } else { failed() };
            let reading = attempt(&mut watch, Signal::Log, at, &outcome);
            assert!(!reading.evaluated.contains("log-stalled/rivet-01"), "at {at}: {reading:?}");
        }
    }

    /// **§6.3: cleared when a new record arrives**, once.
    #[test]
    fn a_new_record_clears_log_stalled_once() {
        let (record, _) = record("log-stalled-clears");
        let mut watch = Watch::default();
        let mut announced = Vec::new();
        for at in (0..=1_200).step_by(60) {
            announced.extend(keys(&record.settle(&log_attempt(&mut watch, at, 0, 0), at)));
        }
        assert_eq!(announced, vec![("log-stalled/rivet-01".to_string(), Change::Raised)]);
        let cleared = record.settle(&log_attempt(&mut watch, 1_260, 3, 0), 1_260);
        assert_eq!(keys(&cleared), vec![("log-stalled/rivet-01".to_string(), Change::Cleared)]);
    }

    // -- card-stalled/ -----------------------------------------------------------

    const KEY: &str = "card-stalled/rivet-01";

    /// **The record `rivet-sysd` actually emits**, copied from
    /// `Rivet cmd/rivet-sysd/src/clock.rs` — the whole sentence, not the prefix,
    /// so a test that only ever matched the prefix could not pass by accident.
    const CLOCK_MSG: &str =
        "the running clock is written to the card, so the next boot has a floor to restore from (ADR-021 §3)";

    fn record_line(boot: &str, mono: f64, svc: &str, msg: &str) -> String {
        format!(
            r#"{{"ts":"2026-09-15T08:49:03Z","boot":"{boot}","mono":{mono},"sev":"info","svc":"{svc}","corr":"c","actor":"a","msg":{}}}"#,
            serde_json::Value::String(msg.to_string())
        )
    }

    /// A boot's worth of ordinary records, with a clock write at `wrote` when
    /// one is wanted, and the newest record at `newest`.
    fn card_records(boot: &str, wrote: Option<f64>, newest: f64) -> Vec<String> {
        let mut records = vec![record_line(boot, 0.5, "rivet-mcpd", "a control-plane call")];
        if let Some(mono) = wrote {
            records.push(record_line(boot, mono, CLOCK_SVC, CLOCK_MSG));
        }
        records.push(record_line(boot, newest, "rivet-mcpd", "a control-plane call"));
        records
    }

    fn card_attempt(
        watch: &mut Watch,
        conditions: Conditions,
        at: u64,
        records: &[String],
        first: bool,
    ) -> Reading {
        let outcome = answered(LOG_ANSWER);
        let summary = LogSummary {
            returned: records.len(),
            stored: records.len(),
            unreadable_records: 0,
            first,
            boot_gone: false,
            cursor_moved: !records.is_empty(),
            card: card_clock(records),
        };
        watch.attempt(
            conditions,
            &Attempted {
                machine: M,
                signal: Signal::Log,
                captured_at: at,
                outcome: &outcome,
                stored: Ok(()),
                log: Some(&summary),
                filters: &[],
            },
        )
    }

    /// **The string this whole condition hangs on** — `SPEC-02` §6.9. If Rivet
    /// rewords the message, THIS is the test that fails, and the condition is
    /// silently off until it is fixed.
    #[test]
    fn the_clock_record_is_matched_by_one_pinned_string() {
        assert_eq!(CLOCK_WRITTEN, "the running clock is written to the card");
        assert_eq!(CLOCK_SVC, "rivet-sysd");
        assert!(CLOCK_MSG.starts_with(CLOCK_WRITTEN), "the prefix must match the real record");

        let seen = card_clock(&card_records("b", Some(60.0), 120.0)).expect("a boot");
        assert_eq!(seen.written_mono, Some(60.0));
        assert_eq!(seen.age_seconds(), 60);

        // A reword, the same message from another service, and the WARN record
        // for a write that was not flushed: none of them is the evidence.
        for (svc, msg) in [
            (CLOCK_SVC, "the running clock was written to the card"),
            ("aura-renderer", CLOCK_MSG),
            (
                CLOCK_SVC,
                "the clock was written and committed and the directory entry could not be flushed",
            ),
        ] {
            let records = vec![
                record_line("b", 60.0, svc, msg),
                record_line("b", 120.0, "rivet-mcpd", "a control-plane call"),
            ];
            assert_eq!(
                card_clock(&records).expect("a boot").written_mono,
                None,
                "{svc}: {msg}"
            );
        }
    }

    /// **§6.9: raised when no clock record has arrived for the window, while
    /// the machine is still answering** — and cleared when one does, once each.
    #[test]
    fn card_stalled_is_raised_after_the_window_and_cleared_by_a_clock_record() {
        let (record, _) = record("card-stalled");
        let mut watch = Watch::default();
        // A first capture — the whole boot — carrying a write a minute ago.
        let start = card_attempt(&mut watch, ON, 0, &card_records("b", Some(60.0), 120.0), true);
        assert!(start.evaluated.contains(KEY), "a clock record is a check that ran");
        assert!(!start.present.contains_key(KEY));

        let mut announced = Vec::new();
        // The board answers every minute and never writes the clock again.
        for at in (60..=4_260).step_by(60) {
            #[allow(clippy::cast_precision_loss)]
            let records = card_records("b", None, 120.0 + at as f64);
            announced.extend(keys(&record.settle(&card_attempt(&mut watch, ON, at, &records, false), at)));
        }
        assert_eq!(announced, vec![(KEY.to_string(), Change::Raised)], "once, and not before");

        let cleared = record.settle(
            &card_attempt(&mut watch, ON, 4_320, &card_records("b", Some(4_400.0), 4_440.0), false),
            4_320,
        );
        assert_eq!(keys(&cleared), vec![(KEY.to_string(), Change::Cleared)]);
    }

    /// **§6.9's headline: the first capture judges the backfill.** The boot is
    /// 22 hours old and its last clock record is 21 hours behind its newest
    /// record — `ADR-002` §1's incident — so it raises at once rather than 70
    /// minutes later.
    #[test]
    fn a_backfill_whose_last_clock_record_is_hours_old_raises_at_once() {
        let mut watch = Watch::default();
        let records = card_records("b", Some(3_600.0), 79_200.0);
        let reading = card_attempt(&mut watch, ON, 1_789_462_143, &records, true);
        let said = reading.present.get(KEY).expect("raised on the first capture");
        assert!(said.contains("1260 minute(s) ago"), "{said}");
        assert!(said.contains("2026-09-14"), "it names when, on this PC's clock: {said}");
    }

    /// **§6.9: a boot with no clock record in it yet is not stalled** — a Pi
    /// writes nothing until its first hour (`clock.rs`'s own test) — and the
    /// window is measured from the boot, not from the capture.
    #[test]
    fn a_boot_this_run_watched_begin_is_measured_from_the_boot() {
        let mut watch = Watch::default();
        // A first capture of a board up ten minutes, which has written nothing.
        let fresh = card_attempt(&mut watch, ON, 10_000, &card_records("b", None, 600.0), true);
        assert!(fresh.present.is_empty(), "a fresh boot is not a stalled card: {fresh:?}");
        assert!(!fresh.evaluated.contains(KEY), "and nothing has been seen to clear it either");

        // An hour and ten minutes after that boot, still nothing written.
        let late = card_attempt(&mut watch, ON, 14_300, &card_records("b", None, 4_900.0), false);
        assert!(late.present.contains_key(KEY), "{late:?}");

        // A reboot this run observes starts the measurement again.
        let mut rebooted = Watch::default();
        card_attempt(&mut rebooted, ON, 0, &card_records("a", Some(60.0), 120.0), true);
        let after = card_attempt(&mut rebooted, ON, 100_000, &card_records("b", None, 300.0), false);
        assert!(after.present.is_empty(), "five minutes into a new boot is not stalled: {after:?}");
    }

    /// **§6.9 before the window is covered**: Rivet Monitor restarted with a
    /// cursor part-way through a boot, so the backfill can hold no clock record
    /// and the earlier ones are behind the cursor. It judges NOTHING until it
    /// has watched for the whole window — and above all clears nothing.
    #[test]
    fn a_restart_part_way_through_a_boot_judges_nothing_until_the_window_is_covered() {
        let mut watch = Watch::default();
        for at in (0..4_200).step_by(60) {
            #[allow(clippy::cast_precision_loss)]
            let records = card_records("b", None, 50_000.0 + at as f64);
            let reading = card_attempt(&mut watch, ON, at, &records, false);
            assert!(
                !reading.evaluated.contains(KEY) && !reading.present.contains_key(KEY),
                "at {at}: a restart knows nothing yet, so it must not clear a standing condition: {reading:?}"
            );
        }
        let now = card_attempt(&mut watch, ON, 4_200, &card_records("b", None, 54_200.0), false);
        assert!(now.present.contains_key(KEY), "once it HAS watched for the window, it may raise");
    }

    /// **§6.9: `card-stalled-minutes = 0` switches it off**, and only a `log`
    /// capture judges it at all.
    #[test]
    fn card_stalled_is_off_at_zero_and_judged_only_on_a_log_capture() {
        let off = Conditions {
            card_stalled_minutes: 0,
            ..ON
        };
        let mut watch = Watch::default();
        for at in (0..10_000).step_by(60) {
            let reading = card_attempt(&mut watch, off, at, &card_records("b", None, 50_000.0), false);
            assert!(!reading.evaluated.contains(KEY) && !reading.present.contains_key(KEY), "at {at}");
        }

        let mut other = Watch::default();
        for signal in [Signal::Status, Signal::Thermal, Signal::Screenshot] {
            let reading = attempt(&mut other, signal, 99_999, &answered(STATUS));
            assert!(!reading.evaluated.contains(KEY), "{signal} judges nothing about the card");
        }
    }

    /// **§6.9 is judged BEFORE §2.4's filter**, like §6.3's `returned`: a
    /// filter that stores none of `rivet-sysd`'s records must not switch the
    /// condition off.
    #[test]
    fn card_stalled_is_judged_before_the_services_filter() {
        let records = card_records("b", Some(60.0), 120.0);
        let filter = ["aura-renderer".to_string()];
        let written = crate::store::log_lines(0, 1, &crate::capture::LogAnswer {
            records: records.clone(),
            cursor: Some("b@120".to_string()),
            boot_gone: false,
            since: None,
            held: None,
            unreadable: None,
        }, None, Some(&filter));
        assert_eq!(written.stored, 0, "the filter keeps none of these records");

        // And the condition still sees the write, because it reads the records
        // the machine sent, not the ones the filter kept.
        let mut watch = Watch::default();
        let summary = LogSummary {
            returned: records.len(),
            stored: written.stored,
            unreadable_records: 0,
            first: true,
            boot_gone: false,
            cursor_moved: true,
            card: card_clock(&records),
        };
        let outcome = answered(LOG_ANSWER);
        let reading = watch.attempt(
            ON,
            &Attempted {
                machine: M,
                signal: Signal::Log,
                captured_at: 0,
                outcome: &outcome,
                stored: Ok(()),
                log: Some(&summary),
                filters: &[(Signal::Log, filter.to_vec())],
            },
        );
        assert!(reading.evaluated.contains(KEY) && !reading.present.contains_key(KEY));
    }

    // -- frozen/ -----------------------------------------------------------------

    fn frame(holder: Option<&str>, stamp: Option<&str>, at: u64) -> Frame {
        Frame {
            holder: holder.map(str::to_string),
            stamp: stamp.map(str::to_string),
            captured_at: at,
        }
    }

    /// Feed a recorded sequence; say, per observation, `R` raised, `C` looked
    /// at and fine, `-` not judged.
    fn frozen_sequence(frames: &[Frame]) -> String {
        let mut watch = Watch::default();
        frames
            .iter()
            .map(|f| {
                let reading = watch.screenshot(M, ON, f);
                if reading.present.contains_key("frozen/rivet-01") {
                    'R'
                } else if reading.evaluated.contains("frozen/rivet-01") {
                    'C'
                } else {
                    '-'
                }
            })
            .collect()
    }

    /// **§6.4: raised only while an application holds the display**, was seen
    /// drawing, and its frame has then not advanced for `frozen-minutes`.
    #[test]
    fn frozen_is_raised_only_while_a_holder_shows_one_frame_for_the_window() {
        let aura = Some("aura-renderer");
        let seq = [
            frame(aura, Some("2026-09-15T03:48:00.000Z"), 0),
            frame(aura, Some("2026-09-15T03:49:00.000Z"), 60),
            frame(aura, Some("2026-09-15T03:49:00.000Z"), 120),
            frame(aura, Some("2026-09-15T03:49:00.000Z"), 359),
            frame(aura, Some("2026-09-15T03:49:00.000Z"), 360),
            frame(aura, Some("2026-09-15T03:49:00.000Z"), 420),
        ];
        assert_eq!(frozen_sequence(&seq), "-C--RR");
    }

    /// **§6.4: a holder never seen drawing is NOT frozen**, however long its
    /// one frame sits there — and judging it so clears a standing condition.
    ///
    /// The shape of rivet-01 from 2026-09-20: Aura stopped writing its own
    /// `screen.qoi` when the compositor took the screen, the file it left
    /// behind never changed again, and this raised `frozen/` against a screen
    /// that was animating the whole time.
    #[test]
    fn a_holder_never_seen_drawing_is_not_frozen() {
        let aura = Some("aura-renderer");
        let stale = Some("1789945010570");
        let seq = [
            frame(aura, stale, 0),
            frame(aura, stale, 299),
            frame(aura, stale, 300),
            frame(aura, stale, 86_400),
        ];
        assert_eq!(frozen_sequence(&seq), "--CC");
    }

    /// **§6.4: no holder never raises it**, however long, and clears it.
    #[test]
    fn no_holder_never_raises_frozen_and_clears_it() {
        let quiet: Vec<Frame> = (0..20).map(|i| frame(None, None, i * 60)).collect();
        assert_eq!(frozen_sequence(&quiet), "C".repeat(20));

        let aura = Some("aura-renderer");
        let seq = [
            frame(aura, Some("f0"), 0),
            frame(aura, Some("f1"), 60),
            frame(aura, Some("f1"), 400),
            frame(None, None, 460),
            frame(aura, Some("f1"), 520),
            frame(aura, Some("f1"), 900),
        ];
        assert_eq!(
            frozen_sequence(&seq),
            "-CRC-C",
            "nothing holding clears; the holder's return starts afresh, not yet seen drawing"
        );
    }

    /// **§6.4: cleared when the frame advances.**
    #[test]
    fn an_advancing_frame_clears_frozen() {
        let aura = Some("aura-renderer");
        let seq = [
            frame(aura, Some("f0"), 0),
            frame(aura, Some("f1"), 60),
            frame(aura, Some("f1"), 360),
            frame(aura, Some("f2"), 420),
            frame(aura, Some("f2"), 480),
        ];
        assert_eq!(frozen_sequence(&seq), "-CRC-");
    }

    /// **§6.4: cleared when the holder changes**, and the new holder must be
    /// seen drawing before its own still frame can raise it.
    #[test]
    fn a_holder_change_clears_frozen() {
        let (aura, launcher) = (Some("aura-renderer"), Some("rivet-launcher"));
        let seq = [
            frame(aura, Some("f0"), 0),
            frame(aura, Some("f1"), 60),
            frame(aura, Some("f1"), 360),
            frame(launcher, Some("f1"), 420),
            frame(launcher, Some("f1"), 720),
            frame(launcher, Some("f2"), 780),
            frame(launcher, Some("f2"), 1_080),
        ];
        assert_eq!(frozen_sequence(&seq), "-CRCCCR");
    }

    /// A holder whose frame carries no timestamp cannot be judged either way.
    #[test]
    fn a_frame_with_no_timestamp_is_not_judged() {
        let aura = Some("aura-renderer");
        let seq = [
            frame(aura, Some("f0"), 0),
            frame(aura, Some("f1"), 60),
            frame(aura, None, 660),
            frame(aura, Some("f1"), 661),
        ];
        assert_eq!(frozen_sequence(&seq), "-C-R");
    }

    // -- power/ ------------------------------------------------------------------

    /// `rivet-01`'s real `get thermal` answer, 2026-09-15 17:30:55Z, with the
    /// two flags this condition reads substituted.
    fn thermal(now: &str, boot: &str) -> String {
        format!(
            r#"{{"subject":"thermal","zones":[{{"name":"cpu-thermal","millidegrees":56965}}],"zones_unreadable":[],"frequency":{{"current_hz":900000000,"maximum_hz":1500000000,"minimum_hz":600000000,"ceiling_hz":1500000000,"ceiling_below_maximum":false,"governor":"ondemand","at_maximum":false,"source":"/sys/devices/system/cpu/cpu0/cpufreq"}},"throttle":{{"under_voltage_now":{now},"frequency_capped_now":false,"throttled_now":false,"soft_temperature_limit_now":false,"under_voltage_since_boot":{boot},"frequency_capped_since_boot":false,"throttled_since_boot":false,"soft_temperature_limit_since_boot":false,"raw":0,"source":"/dev/vcio"}},"read_from":"/sys/class/thermal","means":"…"}}"#
        )
    }

    const UNMEASURED: &str = r#"{"subject":"thermal","zones":[],"zones_unreadable":[],"throttle_unmeasured":"no mailbox read in this build","read_from":"/sys/class/thermal","means":"…"}"#;

    /// **§6.5: the since-boot flag alone raises it** — a dip between two
    /// readings — and so does the now flag.
    #[test]
    fn power_is_raised_by_the_since_boot_flag_alone() {
        let mut watch = Watch::default();
        let dip = attempt(&mut watch, Signal::Thermal, 0, &answered(&thermal("false", "true")));
        assert!(dip.present["power/rivet-01"].contains("since boot"));
        let now = attempt(&mut watch, Signal::Thermal, 60, &answered(&thermal("true", "false")));
        assert!(now.present.contains_key("power/rivet-01"));
    }

    /// **§6.5: cleared by a reading where both are false.**
    #[test]
    fn power_is_cleared_by_both_flags_false() {
        let (record, _) = record("power-clears");
        let mut watch = Watch::default();
        let raised = record.settle(&attempt(&mut watch, Signal::Thermal, 0, &answered(&thermal("false", "true"))), 0);
        assert_eq!(keys(&raised), vec![("power/rivet-01".to_string(), Change::Raised)]);
        let still = record.settle(&attempt(&mut watch, Signal::Thermal, 60, &answered(&thermal("false", "true"))), 60);
        assert!(keys(&still).is_empty());
        let cleared = record.settle(&attempt(&mut watch, Signal::Thermal, 120, &answered(&thermal("false", "false"))), 120);
        assert_eq!(keys(&cleared), vec![("power/rivet-01".to_string(), Change::Cleared)]);
    }

    /// **§6.5: an unmeasured reading neither raises nor clears.**
    #[test]
    fn an_unmeasured_thermal_reading_neither_raises_nor_clears_power() {
        let (record, _) = record("power-unmeasured");
        let mut watch = Watch::default();
        let unmeasured = attempt(&mut watch, Signal::Thermal, 0, &answered(UNMEASURED));
        assert!(!unmeasured.evaluated.contains("power/rivet-01"));

        record.settle(&attempt(&mut watch, Signal::Thermal, 10, &answered(&thermal("true", "true"))), 10);
        let after = record.settle(&attempt(&mut watch, Signal::Thermal, 20, &answered(UNMEASURED)), 20);
        assert!(keys(&after).is_empty());
        assert!(record.standing().contains_key("power/rivet-01"), "absent flags are not clear flags");
    }

    // -- capture/ ------------------------------------------------------------------

    /// **§6.6: raised by a failure on this side, cleared by that SAME capture
    /// next succeeding** — not by a different signal's success, not by a miss.
    #[test]
    fn a_failed_capture_is_cleared_only_by_the_same_capture_succeeding() {
        let (record, _) = record("capture-clears");
        let mut watch = Watch::default();
        let raised = record.settle(&attempt(&mut watch, Signal::Status, 0, &failed()), 0);
        assert_eq!(keys(&raised), vec![("capture/rivet-01/status".to_string(), Change::Raised)]);

        let other = record.settle(&attempt(&mut watch, Signal::Thermal, 1, &answered(&thermal("false", "false"))), 1);
        assert!(keys(&other).is_empty(), "{:?}", keys(&other));
        let miss = record.settle(&attempt(&mut watch, Signal::Status, 2, &missed()), 2);
        assert!(keys(&miss).is_empty(), "a miss says nothing about this side");

        let fixed = record.settle(&attempt(&mut watch, Signal::Status, 3, &answered(STATUS)), 3);
        assert_eq!(keys(&fixed), vec![("capture/rivet-01/status".to_string(), Change::Cleared)]);
    }

    /// **§5.5: a write that failed is this side's too**, even when the machine
    /// answered — and a marker that could not be written is one.
    #[test]
    fn a_write_that_failed_raises_capture() {
        let mut watch = Watch::default();
        let outcome = answered(STATUS);
        let reading = watch.attempt(
            ON,
            &Attempted {
                machine: M,
                signal: Signal::Status,
                captured_at: 0,
                outcome: &outcome,
                stored: Err("cannot open D:/full/status/2026-09-15.jsonl"),
                log: None,
                filters: &[],
            },
        );
        assert!(reading.present["capture/rivet-01/status"].contains("could not be written"));
        assert!(reading.evaluated.contains("reach/rivet-01"), "the machine did answer");
    }

    /// **§2.4 through the layer**: a `services` name the machine does not run
    /// raises `capture/<machine>/<signal>/service`, and fixing it clears it.
    #[test]
    fn a_services_name_the_machine_does_not_run_raises_and_clears() {
        let path = scratch("service").join(FILE);
        let record = Record::open(Some(path));
        let mut machine = resolved(M, ON, &[Signal::Status, Signal::Log]);
        machine.signals[0].services = Some(vec!["aura-renderr".to_string()]);
        record.subjects(Subjects::of(&[machine]), 0);

        let mut watch = Watch::default();
        let outcome = answered(STATUS);
        let filters = [(Signal::Status, vec!["aura-renderr".to_string()])];
        let seen = |at, filters: &[(Signal, Vec<String>)], watch: &mut Watch| {
            watch.attempt(
                ON,
                &Attempted {
                    machine: M,
                    signal: Signal::Status,
                    captured_at: at,
                    outcome: &outcome,
                    stored: Ok(()),
                    log: None,
                    filters,
                },
            )
        };
        let raised = record.settle(&seen(0, &filters, &mut watch), 0);
        assert_eq!(keys(&raised), vec![("capture/rivet-01/status/service".to_string(), Change::Raised)]);
        assert!(raised.announcements[0].summary.contains("aura-renderr"));
        assert!(keys(&record.settle(&seen(60, &filters, &mut watch), 60)).is_empty());

        let fixed = [(Signal::Status, vec!["aura-renderer".to_string()])];
        let cleared = record.settle(&seen(120, &fixed, &mut watch), 120);
        assert_eq!(keys(&cleared), vec![("capture/rivet-01/status/service".to_string(), Change::Cleared)]);
    }

    #[test]
    fn unmatched_services_are_found_per_filter() {
        let status: serde_json::Value = serde_json::from_str(STATUS).unwrap();
        let filters = vec![
            (Signal::Status, vec!["aura-renderer".to_string()]),
            (Signal::Log, vec!["tempmon-rivet".to_string(), "aura-renderr".to_string()]),
        ];
        assert_eq!(
            unmatched_services(&filters, &status),
            vec![(Signal::Status, vec![]), (Signal::Log, vec!["aura-renderr".to_string()])]
        );
    }

    // -- a subject that was deleted — §6.1 -----------------------------------------

    /// Stand every monitor condition for `rivet-01`, plus two for a machine
    /// whose name merely STARTS the same.
    fn stand_everything(record: &Record) {
        let mut reading = Reading::default();
        for key in [
            "reach/rivet-01",
            "log-stalled/rivet-01",
            "card-stalled/rivet-01",
            "frozen/rivet-01",
            "power/rivet-01",
            "capture/rivet-01/register",
            "capture/rivet-01/thermal",
            "capture/rivet-01/status/service",
            "reach/rivet-010",
            "capture/rivet-010/status",
        ] {
            raise(&mut reading, key.to_string(), "standing".to_string());
        }
        record.settle(&reading, 0);
    }

    fn subjects_with(rivet_01: Option<(Conditions, &[Signal], bool)>) -> Subjects {
        let mut machines = vec![resolved("rivet-010", ON, &[Signal::Status])];
        if let Some((conditions, signals, filtered)) = rivet_01 {
            let mut machine = resolved(M, conditions, signals);
            if filtered {
                for signal in &mut machine.signals {
                    if signal.signal == Signal::Status {
                        signal.services = Some(vec!["aura-renderer".to_string()]);
                    }
                }
            }
            machines.push(machine);
        }
        Subjects::of(&machines)
    }

    fn cleared(settled: &Settled) -> BTreeSet<String> {
        settled
            .announcements
            .iter()
            .filter(|a| a.change == Change::Cleared)
            .map(|a| a.key.clone())
            .collect()
    }

    fn set(keys: &[&str]) -> BTreeSet<String> {
        keys.iter().map(|k| (*k).to_string()).collect()
    }

    const ALL: &[Signal] = &EVERY;

    /// **§6.1: removing a machine's entry clears everything under it** — and
    /// nothing of a machine whose name only begins the same.
    #[test]
    fn removing_a_machine_clears_everything_under_it_and_nothing_else() {
        let path = scratch("deleted-machine").join(FILE);
        let record = Record::open(Some(path));
        record.subjects(subjects_with(Some((ON, ALL, true))), 0);
        stand_everything(&record);
        assert_eq!(record.standing().len(), 10);

        let settled = record.subjects(subjects_with(None), 100);
        assert_eq!(
            cleared(&settled),
            set(&[
                "reach/rivet-01",
                "log-stalled/rivet-01",
                "card-stalled/rivet-01",
                "frozen/rivet-01",
                "power/rivet-01",
                "capture/rivet-01/register",
                "capture/rivet-01/thermal",
                "capture/rivet-01/status/service",
            ])
        );
        assert!(settled.announcements.iter().all(|a| a.summary.contains("no longer a [[monitor.machine]]")));
        assert_eq!(
            record.standing().keys().cloned().collect::<BTreeSet<_>>(),
            set(&["reach/rivet-010", "capture/rivet-010/status"])
        );
        assert!(record.subjects(subjects_with(None), 200).announcements.is_empty(), "once");

        // And no `[monitor]` table at all is no subject anywhere.
        let none = record.subjects(Subjects::default(), 300);
        assert_eq!(cleared(&none), set(&["reach/rivet-010", "capture/rivet-010/status"]));
    }

    /// **§6.1: removing a signal clears only the conditions that exist because
    /// of it.**
    #[test]
    fn removing_a_signal_clears_only_what_exists_because_of_it() {
        let path = scratch("deleted-signal").join(FILE);
        let record = Record::open(Some(path));
        record.subjects(subjects_with(Some((ON, ALL, true))), 0);
        stand_everything(&record);

        let no_thermal = record.subjects(
            subjects_with(Some((ON, &[Signal::Status, Signal::Log, Signal::Screenshot][..], true))),
            10,
        );
        assert_eq!(cleared(&no_thermal), set(&["power/rivet-01", "capture/rivet-01/thermal"]));

        // §6.9 depends on `log` exactly as §6.3 does, and goes with it.
        let no_log = record.subjects(subjects_with(Some((ON, &[Signal::Status, Signal::Screenshot][..], true))), 20);
        assert_eq!(cleared(&no_log), set(&["log-stalled/rivet-01", "card-stalled/rivet-01"]));

        let no_screenshot = record.subjects(subjects_with(Some((ON, &[Signal::Status][..], true))), 30);
        assert_eq!(cleared(&no_screenshot), set(&["frozen/rivet-01"]));

        let no_filter = record.subjects(subjects_with(Some((ON, &[Signal::Status][..], false))), 40);
        assert_eq!(cleared(&no_filter), set(&["capture/rivet-01/status/service"]));

        assert!(record.standing().contains_key("reach/rivet-01"));
        assert!(record.standing().contains_key("capture/rivet-01/register"));
    }

    /// **§6.1: switching a condition off with `0` or `false` clears it.**
    #[test]
    fn switching_a_condition_off_clears_it() {
        let path = scratch("switched-off").join(FILE);
        let record = Record::open(Some(path));
        record.subjects(subjects_with(Some((ON, ALL, true))), 0);
        stand_everything(&record);

        let off = Conditions {
            reach_misses: 0,
            log_stalled_minutes: 0,
            frozen_minutes: 0,
            card_stalled_minutes: 0,
            power: false,
        };
        let settled = record.subjects(subjects_with(Some((off, ALL, true))), 10);
        assert_eq!(
            cleared(&settled),
            set(&[
                "reach/rivet-01",
                "log-stalled/rivet-01",
                "card-stalled/rivet-01",
                "frozen/rivet-01",
                "power/rivet-01"
            ])
        );
    }

    /// **A worker still holding the old plan cannot raise what was just
    /// cleared as deleted** — its reading is dropped.
    #[test]
    fn a_reading_about_a_subject_no_longer_configured_is_dropped() {
        let (record, _) = record("stale-plan");
        record.subjects(Subjects::of(&[resolved(M, ON, &[Signal::Status])]), 0);
        let mut watch = Watch::default();
        let late = record.settle(&attempt(&mut watch, Signal::Thermal, 1, &answered(&thermal("true", "true"))), 1);
        assert!(late.announcements.is_empty(), "{:?}", keys(&late));
        assert!(record.standing().is_empty());
    }

    /// Nothing is settled before the supervisor has accepted a configuration.
    #[test]
    fn nothing_settles_before_a_configuration_is_accepted() {
        let path = scratch("unaccepted").join(FILE);
        let record = Record::open(Some(path.clone()));
        let mut watch = Watch::default();
        let settled = record.settle(&attempt(&mut watch, Signal::Status, 0, &failed()), 0);
        assert!(settled.announcements.is_empty());
        assert!(!path.exists());
    }

    // -- restarts and the record --------------------------------------------------

    /// **A restart repeats nothing** — and a fresh `Watch`, which knows no
    /// history, clears nothing it cannot see cleared.
    #[test]
    fn a_restart_repeats_nothing() {
        let (record, path) = record("restart");
        let mut watch = Watch::default();
        for at in (0..=900).step_by(60) {
            record.settle(&log_attempt(&mut watch, at, 0, 0), at);
        }
        let aura = Some("aura-renderer");
        for (at, stamp) in [(0, "f0"), (1, "f1"), (601, "f1")] {
            let reading = watch.screenshot(M, ON, &frame(aura, Some(stamp), at));
            record.settle(&reading, at);
        }
        // The misses come AFTER the answered log captures, which reset them.
        for at in 901..904 {
            record.settle(&attempt(&mut watch, Signal::Status, at, &missed()), at);
        }
        record.settle(&attempt(&mut watch, Signal::Thermal, 904, &failed()), 904);
        let before = record.standing();
        assert_eq!(
            before.keys().cloned().collect::<BTreeSet<_>>(),
            set(&[
                "log-stalled/rivet-01",
                "frozen/rivet-01",
                "reach/rivet-01",
                "capture/rivet-01/thermal"
            ]),
        );
        drop(record);

        let reopened = Record::open(Some(path));
        reopened.subjects(Subjects::of(&[resolved(M, ON, &EVERY)]), 1_000);
        let mut fresh = Watch::default();
        let mut announced = Vec::new();
        announced.extend(keys(&reopened.settle(&attempt(&mut fresh, Signal::Status, 1_000, &missed()), 1_000)));
        announced.extend(keys(&reopened.settle(&attempt(&mut fresh, Signal::Thermal, 1_001, &failed()), 1_001)));
        announced.extend(keys(&reopened.settle(&fresh.screenshot(M, ON, &frame(aura, Some("f1"), 1_003)), 1_003)));
        assert!(announced.is_empty(), "{announced:?}");
        assert_eq!(reopened.standing(), before);

        // An empty log answer after the restart does not clear the stall it
        // cannot see the start of — though, being an answer, it does clear
        // `reach/`, which is a true statement about the machine.
        let empty = log_attempt(&mut fresh, 1_002, 0, 0);
        assert!(!empty.evaluated.contains("log-stalled/rivet-01"));
        assert_eq!(keys(&reopened.settle(&empty, 1_002)), vec![("reach/rivet-01".to_string(), Change::Cleared)]);
    }

    /// **A record that will not parse is not read as empty**, is not
    /// overwritten, and says so once.
    #[test]
    fn an_unreadable_record_is_never_read_as_empty() {
        let dir = scratch("unreadable");
        let path = dir.join(FILE);
        std::fs::write(&path, "standing = [not toml").unwrap();
        let record = Record::open(Some(path.clone()));
        let first = record.subjects(Subjects::of(&[resolved(M, ON, &EVERY)]), 0);
        assert!(first.trouble.as_deref().is_some_and(|t| t.contains("could not be read")));
        let mut watch = Watch::default();
        let second = record.settle(&attempt(&mut watch, Signal::Status, 1, &failed()), 1);
        assert!(second.announcements.is_empty());
        assert!(second.trouble.is_none(), "an unchanged problem is said once");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "standing = [not toml");

        std::fs::write(&path, "[standing]\n\"capture/rivet-01/status\" = 5\n").unwrap();
        let healed = record.settle(&attempt(&mut watch, Signal::Status, 2, &failed()), 2);
        assert!(healed.announcements.is_empty(), "it was already standing: {:?}", keys(&healed));
    }

    // -- the interleaving ------------------------------------------------------------

    /// **The backup pass and the monitor's workers, interleaved, lose and
    /// double nothing.**
    ///
    /// One thread does what the tray's backup worker does to `state.toml`:
    /// load it whole, reconcile its announcements, record a success, rename it
    /// over. Four threads do what four machines' workers do, and one does what
    /// the supervisor does, all through one [`Record`]. Every round flips each
    /// condition, so every round owes exactly one announcement per key; a lost
    /// update shows as a doubled or missing announcement, or as a file that
    /// disagrees with what was announced.
    #[test]
    fn the_backup_pass_and_the_monitor_workers_lose_and_double_nothing() {
        const ROUNDS: u64 = 201;
        let dir = scratch("interleave");
        let config = dir.join("config.toml");
        let machines: Vec<String> = (0..4).map(|i| format!("rivet-0{i}")).collect();
        let resolved_all: Vec<ResolvedMachine> = machines
            .iter()
            .map(|m| resolved(m, ON, &[Signal::Status, Signal::Thermal]))
            .collect();
        let record = Arc::new(Record::open(Some(beside(&config))));
        record.subjects(Subjects::of(&resolved_all), 0);
        let start = Arc::new(Barrier::new(machines.len() + 2));

        let backup = {
            let (config, start) = (config.clone(), Arc::clone(&start));
            std::thread::spawn(move || {
                start.wait();
                let mut announced = Vec::new();
                for round in 0..ROUNDS {
                    let mut ledger = Ledger::load(&config).expect("state.toml reads");
                    let mut reading = Reading::default();
                    if round.is_multiple_of(2) {
                        raise(&mut reading, "fetch/rivet-00".to_string(), "403".to_string());
                    } else {
                        fine(&mut reading, "fetch/rivet-00".to_string());
                    }
                    announced.extend(alarm::reconcile(&mut ledger.standing, &reading, round));
                    ledger
                        .record("rivet-00", &format!("app-{round}"), round)
                        .expect("state.toml writes");
                    std::thread::yield_now();
                }
                announced
            })
        };
        let supervisor = {
            let (record, start, subjects) = (Arc::clone(&record), Arc::clone(&start), Subjects::of(&resolved_all));
            std::thread::spawn(move || {
                start.wait();
                let mut announced = Vec::new();
                for round in 0..ROUNDS {
                    announced.extend(record.subjects(subjects.clone(), round).announcements);
                    std::thread::yield_now();
                }
                announced
            })
        };
        let workers: Vec<_> = machines
            .iter()
            .map(|machine| {
                let (record, start, machine) = (Arc::clone(&record), Arc::clone(&start), machine.clone());
                std::thread::spawn(move || {
                    start.wait();
                    let mut announced = Vec::new();
                    for round in 0..ROUNDS {
                        let mut reading = Reading::default();
                        for key in [format!("capture/{machine}/status"), format!("reach/{machine}")] {
                            if round.is_multiple_of(2) {
                                raise(&mut reading, key, "down".to_string());
                            } else {
                                fine(&mut reading, key);
                            }
                        }
                        let settled = record.settle(&reading, round);
                        assert!(settled.trouble.is_none(), "{:?}", settled.trouble);
                        announced.extend(settled.announcements);
                        std::thread::yield_now();
                    }
                    (machine, announced)
                })
            })
            .collect();

        let backup_announced = backup.join().expect("the backup thread");
        assert!(supervisor.join().expect("the supervisor").is_empty(), "every subject exists");
        let expected = |round: u64| if round.is_multiple_of(2) { Change::Raised } else { Change::Cleared };
        for worker in workers {
            let (machine, announced) = worker.join().expect("a worker");
            for key in [format!("capture/{machine}/status"), format!("reach/{machine}")] {
                let changes: Vec<Change> = announced.iter().filter(|a| a.key == key).map(|a| a.change).collect();
                assert_eq!(
                    changes,
                    (0..ROUNDS).map(expected).collect::<Vec<_>>(),
                    "{key}: every flip announced exactly once"
                );
            }
        }
        assert_eq!(
            backup_announced.iter().map(|a| a.change).collect::<Vec<_>>(),
            (0..ROUNDS).map(expected).collect::<Vec<_>>()
        );

        // What is on disk is what was announced, in both files, and neither
        // holds the other's.
        let reopened = Record::open(Some(beside(&config)));
        let on_disk: BTreeSet<String> = reopened.standing().keys().cloned().collect();
        let wanted: BTreeSet<String> = machines
            .iter()
            .flat_map(|m| [format!("capture/{m}/status"), format!("reach/{m}")])
            .collect();
        assert_eq!(on_disk, wanted);
        let ledger = Ledger::load(&config).expect("state.toml reads");
        assert_eq!(ledger.standing.keys().cloned().collect::<Vec<_>>(), vec!["fetch/rivet-00".to_string()]);
        assert_eq!(ledger.success.len(), usize::try_from(ROUNDS).unwrap(), "no success lost");

        // And a restart over the final state says nothing.
        reopened.subjects(Subjects::of(&resolved_all), ROUNDS);
        for machine in &machines {
            let mut reading = Reading::default();
            raise(&mut reading, format!("capture/{machine}/status"), "down".to_string());
            raise(&mut reading, format!("reach/{machine}"), "down".to_string());
            assert!(reopened.settle(&reading, ROUNDS).announcements.is_empty());
        }
    }
}
