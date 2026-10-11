//! **The monitor: its own timer, one worker per machine** — `SPEC-02` §2.2,
//! §2.4, §3, §4.1, §4.2 and §5.
//!
//! # Two kinds of thread, and why neither waits for a backup
//!
//! The **supervisor** re-reads the configuration every [`TICK`], reads the
//! register, proves the tool, and hands each monitored machine a [`Plan`]. It
//! also runs retention, once at the first tick that can and hourly after.
//!
//! Each **worker** owns one machine. It captures one signal at a time, the most
//! overdue first, and writes one line per attempt. A machine that is off costs
//! its own worker a timeout and nothing else (§3.2).
//!
//! **Neither is `SPEC-01` §3's pass.** They share no lock and no thread with
//! `crate::schedule`, so a backup fetch in progress never delays a capture and
//! a capture never delays a backup (§3.1).
//!
//! # The log is a capture with a cursor
//!
//! `log` (§4.2) runs in the same worker, on the same cadence rules, and writes
//! its records, its attempt line and — only after those are on disk — its
//! cursor, `<folder>/<machine>/log/cursor`. One worker per machine is the
//! cursor's only writer, which is why it is a file of its own and not a key in
//! `state.toml` (the backup pass rewrites that file whole).
//!
//! # Conditions: every attempt settles what it checked
//!
//! §6's conditions go through `SPEC-01` §5.5's layer, and this module does not
//! decide them — `crate::conditions` does. Each worker keeps a
//! [`Watch`](crate::conditions::Watch) for its machine and, after every attempt
//! it actually made, settles the conditions that attempt checked into the one
//! shared [`Record`](crate::conditions::Record). The supervisor settles
//! `capture/<machine>/register` against the register it captures with, and on
//! every accepted tick tells the record which subjects still exist, so a
//! condition whose machine, signal or setting was removed clears. The record is
//! `monitor-state.toml`, not `state.toml`, and `crate::conditions` says why.
//!
//! # The screenshot is a capture that decides whether to keep what it got
//!
//! `screenshot` (§4.3) asks `cargo rivet screenshot --holder --json` to write
//! the display holder's frame into a staging file inside `monitor.folder`,
//! then keeps it (a rename) or deletes it: nothing holds the display, or the
//! frame's own timestamp is the one last STORED for that holder. That last
//! stored timestamp is read back off the screenshot index when a worker
//! starts ([`Memory`]), so a restart does not store an unchanged frame again.
//! Every answered attempt also feeds
//! [`Watch::screenshot`](crate::conditions::Watch::screenshot), which is
//! §6.4's `frozen/`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::alarm::{Announcement, Change, Reading};
use crate::armada;
use crate::capture::{self, Attempt, Outcome};
use crate::clock;
use crate::conditions::{self, Attempted, Frame, Record, Settled, Subjects, Watch};
use crate::config::{self, Conditions, Refusal, ResolvedSignal, Signal};
use crate::register::{Device, Register};
use crate::store;
use crate::tool::Target;

/// How often the supervisor re-reads the configuration — §3.5's "next tick".
pub const TICK: Duration = Duration::from_secs(5);

/// How often an idle worker looks for something due.
const WORKER_TICK: Duration = Duration::from_millis(500);

/// §5.4: hourly.
const RETENTION_EVERY: Duration = Duration::from_secs(3_600);

/// How long a register read that failed waits before it is tried again.
const REGISTER_RETRY: Duration = Duration::from_secs(30);

/// How long a tool that failed its proof waits before it is proved again.
const PROOF_RETRY: Duration = Duration::from_secs(60);

/// Something the monitor did or found. The caller decides where it goes: the
/// `monitor` command prints it, and the tray logs it.
#[derive(Debug, Clone)]
pub enum Event {
    /// One attempt, and where its line went.
    Attempt {
        machine: String,
        signal: Signal,
        captured_at: u64,
        elapsed_ms: u64,
        outcome: &'static str,
        reason: Option<String>,
        /// The file the line was appended to, or **why it was not** — §5.5.
        stored: Result<PathBuf, String>,
        /// For an answered `log` capture: what came back and what was kept.
        log: Option<LogSummary>,
        /// For an answered `screenshot` capture: what the index recorded.
        screenshot: Option<ShotSummary>,
    },
    /// §5.3's `not-captured` marker.
    NotCaptured {
        machine: String,
        signal: Signal,
        from: u64,
        to: u64,
        stored: Result<PathBuf, String>,
    },
    /// The monitor's configuration refusals, whole, each time they change. An
    /// empty list means the ones reported before are gone.
    Refused(Vec<Refusal>),
    /// Something loud under a stable key that is NOT one of §6's conditions —
    /// the register read failing, which the backup pass already announces as
    /// `register`. `Some` when it appears, `None` when its check ran and found
    /// it gone.
    Finding { key: String, message: Option<String> },
    /// **A §6 condition appeared or cleared** — once per transition, settled
    /// into the monitor's record (`SPEC-01` §5.5).
    Announced(Announcement),
    /// The record of what has been announced could not be read or written.
    Unrecorded(String),
    /// Worth saying once.
    Note(String),
    /// A retention sweep.
    Retired {
        removed: Vec<PathBuf>,
        trouble: Vec<String>,
    },
}

/// **One answered log capture, counted** — and what `SPEC-02` §6.3 judges.
///
/// `log-stalled/` is judged on `returned`, **before §2.4's filter**, so a
/// narrow filter on a quiet service cannot fake a stall; `stored` is the same
/// count after it. Both are on disk too, on each answered attempt line in
/// `log/<date>.jsonl`.
///
/// Every capture's own connection writes `rivet-mcpd` records into the board's
/// journal, so a journal that is accepting records never answers empty. That
/// does not stop §6.3 firing: those records take the same path as every other,
/// and a writer that has stopped appending stops appending them too. What it
/// cannot see is a journal whose records are still READABLE and never reach
/// the card — `SPEC-02` §6.3 says so.
#[derive(Debug, Clone, PartialEq)]
pub struct LogSummary {
    /// Records the machine returned — BEFORE the filter.
    pub returned: usize,
    /// Record lines written — AFTER the filter, unreadable ones included.
    pub stored: usize,
    /// Record strings that were not a JSON object, kept as text.
    pub unreadable_records: usize,
    /// No cursor was held: this was `get log latest all`.
    pub first: bool,
    /// The board's retention dropped the cursor's boot; a `retention-gap`
    /// marker was written when records came back.
    pub boot_gone: bool,
    /// The cursor file now names the last record returned.
    pub cursor_moved: bool,
    /// **What these records say about the card** — `SPEC-02` §6.9, read
    /// BEFORE the filter like `returned`. `None` when the answer carried no
    /// record this build could place in a boot.
    pub card: Option<conditions::CardClock>,
}

/// **One answered screenshot, as its index line records it** — §4.3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShotSummary {
    /// `stored`, `unchanged`, `no application holds the display`, or `not
    /// stored` when a frame to keep could not be placed.
    pub result: &'static str,
    pub holder: Option<String>,
    /// The frame's timestamp, by the BOARD's clock.
    pub frame_modified_ms: Option<i64>,
    /// Where it was stored, relative to the screenshot directory.
    pub file: Option<String>,
}

/// Where events go.
pub type Reporter = Arc<dyn Fn(Event) + Send + Sync>;

/// **One event, as the lines a person reads.**
#[must_use]
pub fn describe(event: &Event) -> Vec<String> {
    let one_line = |text: &str| {
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if flat.chars().count() > 300 {
            flat.chars().take(297).collect::<String>() + "..."
        } else {
            flat
        }
    };
    match event {
        Event::Attempt {
            machine,
            signal,
            captured_at,
            elapsed_ms,
            outcome,
            reason,
            stored,
            log,
            screenshot,
        } => {
            let mut line = format!(
                "{} {machine:<12} {:<8} {outcome:<9} {elapsed_ms:>6} ms",
                clock::utc_stamp(*captured_at),
                signal.name()
            );
            if let Some(log) = log {
                line.push_str(&format!(
                    "  {} returned, {} stored{}{}{}{}",
                    log.returned,
                    log.stored,
                    if log.first { ", first capture (latest all)" } else { "" },
                    if log.unreadable_records > 0 {
                        format!(", {} unreadable kept as text", log.unreadable_records)
                    } else {
                        String::new()
                    },
                    if log.boot_gone { ", RETENTION GAP: the cursor's boot is gone" } else { "" },
                    if log.cursor_moved { "" } else { ", cursor kept" },
                ));
            }
            if let Some(shot) = screenshot {
                line.push_str(&format!("  {}", shot.result));
                if let Some(holder) = &shot.holder {
                    line.push_str(&format!(
                        " {holder}, frame {} (board clock)",
                        shot.frame_modified_ms
                            .map_or_else(|| "with no timestamp".to_string(), |ms| format!("{ms} ms"))
                    ));
                }
                if let Some(file) = &shot.file {
                    line.push_str(&format!(" -> {file}"));
                }
            }
            if let Some(reason) = reason {
                line.push_str("  ");
                line.push_str(&one_line(reason));
            }
            if let Err(why) = stored {
                line.push_str(&format!("  NOT STORED: {}", one_line(why)));
            }
            vec![line]
        }
        Event::NotCaptured {
            machine,
            signal,
            from,
            to,
            stored,
        } => {
            let mut line = format!(
                "{} {machine:<12} {:<8} not-captured from {} to {}",
                clock::utc_stamp(*to),
                signal.name(),
                clock::utc_stamp(*from),
                clock::utc_stamp(*to)
            );
            if let Err(why) = stored {
                line.push_str(&format!("  NOT STORED: {}", one_line(why)));
            }
            vec![line]
        }
        Event::Refused(refusals) if refusals.is_empty() => {
            vec!["the monitor's configuration is accepted; capturing can start".to_string()]
        }
        Event::Refused(refusals) => std::iter::once("REFUSED — nothing is captured".to_string())
            .chain(refusals.iter().map(|r| format!("  {}", one_line(&r.to_string()))))
            .collect(),
        Event::Finding {
            key,
            message: Some(message),
        } => vec![format!("[started] {key} — {}", one_line(message))],
        Event::Finding { key, message: None } => vec![format!("[fixed] {key}")],
        Event::Announced(one) => {
            let verb = match one.change {
                Change::Raised => "started",
                Change::Cleared => "fixed",
            };
            vec![format!("[{verb}] {} — {}", one.key, one_line(&one.summary))]
        }
        Event::Unrecorded(why) => vec![format!("ANNOUNCEMENTS NOT RECORDED: {}", one_line(why))],
        Event::Note(note) => vec![format!("note: {}", one_line(note))],
        Event::Retired { removed, trouble } => {
            let mut lines = vec![format!(
                "retention: {} file(s) older than their keep-days removed",
                removed.len()
            )];
            lines.extend(removed.iter().map(|p| format!("  removed {}", p.display())));
            lines.extend(trouble.iter().map(|t| format!("  RETENTION FAILED: {t}")));
            lines
        }
    }
}

/// **The signal to capture next** — §3.2 and §3.4.
///
/// Due when `every-seconds` have passed since the last ATTEMPT, whatever its
/// outcome. Of those due, the most overdue; never attempted counts as the most
/// overdue of all; ties go to §2.3's order. A last attempt in the future — the
/// PC's clock was set back — is due now, rather than silent until the clock
/// catches up.
#[must_use]
pub fn most_overdue(
    signals: &[ResolvedSignal],
    last: &BTreeMap<Signal, Option<u64>>,
    now: u64,
) -> Option<Signal> {
    let mut best: Option<(u64, Signal)> = None;
    for signal in signals {
        let lateness = match last.get(&signal.signal).copied().flatten() {
            None => u64::MAX,
            Some(then) if then > now => 0,
            Some(then) => {
                let due_at = then.saturating_add(signal.every_seconds);
                if now < due_at {
                    continue;
                }
                now - due_at
            }
        };
        if best.is_none_or(|(most, _)| lateness > most) {
            best = Some((lateness, signal.signal));
        }
    }
    best.map(|(_, signal)| signal)
}

/// Everything one machine's worker needs, resolved by the supervisor.
#[derive(Debug, Clone)]
struct Plan {
    folder: PathBuf,
    tool: PathBuf,
    identity: PathBuf,
    timeout: Duration,
    /// From the register — or the register's reason it cannot place this one.
    target: Result<Target, String>,
    /// The signals to capture — the spot readings (§4.1), the log (§4.2) and
    /// the screenshot (§4.3) — in §2.3's order.
    signals: Vec<ResolvedSignal>,
    /// Signals the tool could not be proved for, and why.
    unproven: BTreeMap<Signal, String>,
    /// Every `services` filter configured on this machine (§2.4), judged
    /// against each `status` answer.
    filters: Vec<(Signal, Vec<String>)>,
    /// §6's thresholds for this machine, §2.5's overrides applied.
    conditions: Conditions,
    /// How many retention sweeps have run. A change means a sweep may have
    /// retired the frame this worker remembers storing (§4.3, §5.4).
    sweeps: u64,
}

/// A worker's mailbox. `plan: None` pauses it; `alive: false` ends it.
struct Slot {
    alive: bool,
    plan: Option<Plan>,
}

/// **Run the monitor until `stop` is set.** Blocks; the tray runs it on a
/// thread, and `rivet-monitor monitor` in the foreground.
///
/// **It settles the monitor's announcements** into `monitor-state.toml`, beside
/// the configuration file — the tray and `rivet-monitor monitor` alike, because
/// both run workers that act (`crate::conditions`).
pub fn run(path_override: Option<PathBuf>, report: &Reporter, stop: &Arc<AtomicBool>) {
    let record = config::config_path(path_override.as_deref()).map(|path| conditions::beside(&path));
    let mut supervisor = Supervisor {
        record: Arc::new(Record::open(record)),
        ..Supervisor::default()
    };
    let mut next_retention: Option<Instant> = None;
    while !stop.load(Ordering::SeqCst) {
        supervisor.tick(path_override.as_deref(), report, stop);
        if next_retention.is_none_or(|at| Instant::now() >= at) && supervisor.retention(report) {
            next_retention = Some(Instant::now() + RETENTION_EVERY);
        }
        sleep_unless(TICK, stop);
    }
    supervisor.finish();
}

/// Start [`run`] on its own thread.
pub fn spawn(path_override: Option<PathBuf>, report: Reporter, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    std::thread::spawn(move || run(path_override, &report, &stop))
}

fn sleep_unless(duration: Duration, stop: &AtomicBool) {
    let until = Instant::now() + duration;
    while !stop.load(Ordering::SeqCst) {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        std::thread::sleep(left.min(Duration::from_millis(200)));
    }
}

/// Where a register was read from: a server address, the name TLS verifies
/// it against, the authority. `fleet.server`/`fleet.name` for the
/// single-fleet fallback; a Fleet's `host`/`name` for one read through the
/// Armada Registry (`crate::armada`).
type RegisterSource = (String, Option<String>, PathBuf);

/// **The key `Supervisor::registers` caches the single-fleet fallback
/// under.** Not a Fleet name — an Armada Fleet is never named this, because
/// `armada`'s own naming rules govern Fleet names and this key is chosen to
/// be disjoint from them (a slash cannot appear in a Fleet's file-derived
/// name).
const SINGLE_FLEET_KEY: &str = "fleet/server";

/// **One cached register and its retry state** — the same caching the
/// single-fleet path always had, generalised so every Armada Fleet gets its
/// own independently: a Fleet whose portal is unreachable must not affect
/// another Fleet's polling interval or retry clock.
#[derive(Default)]
struct RegisterCache {
    register: Option<(Register, u64, RegisterSource)>,
    tried: Option<Instant>,
}

impl RegisterCache {
    /// Read this register if it is due, else the last good answer.
    ///
    /// **A read that fails after one has succeeded keeps capturing against
    /// the last answer, and says so**: a fleet server is very often ON a
    /// monitored machine, and a monitor that went blind whenever that machine
    /// stopped answering would stop writing `missed` lines at the moment they
    /// are the evidence (`SPEC-02` §2.2 and its History). This is exactly the
    /// behaviour the single-fleet path always had; it is unchanged here, only
    /// given a home that more than one source can use.
    fn poll(
        &mut self,
        from: RegisterSource,
        poll: Duration,
        findings: &mut BTreeMap<String, String>,
        report: &Reporter,
        key: &str,
    ) -> Option<Register> {
        if self.register.as_ref().is_some_and(|(_, _, was)| *was != from) {
            // A different source: the old answer is not about it.
            self.register = None;
            self.tried = None;
        }
        let due = match (&self.register, self.tried) {
            (_, None) => true,
            (Some(_), Some(tried)) => tried.elapsed() >= poll,
            (None, Some(tried)) => tried.elapsed() >= REGISTER_RETRY,
        };
        if due {
            self.tried = Some(Instant::now());
            match crate::register::fetch(&from.0, from.1.as_deref(), &from.2, 4443) {
                Ok(register) => {
                    self.register = Some((register, clock::now(), from));
                    clear(findings, report, key);
                }
                Err(why) => {
                    let message = match &self.register {
                        Some((_, at, _)) => format!(
                            "the register could not be re-read, so capture continues against \
                             the one read at {}: {why}",
                            clock::utc_stamp(*at)
                        ),
                        None => format!(
                            "the register could not be read and has not been read since this \
                             started, so no monitored machine can be found and NOTHING IS \
                             CAPTURED — unverified, not refused (SPEC-02 §2.2): {why}"
                        ),
                    };
                    raise(findings, report, key, message);
                }
            }
        }
        self.register.as_ref().map(|(register, _, _)| register.clone())
    }
}

#[derive(Default)]
struct Supervisor {
    workers: BTreeMap<String, (Arc<Mutex<Slot>>, JoinHandle<()>)>,
    /// Workers told to end, joined at [`Supervisor::finish`].
    retired: Vec<JoinHandle<()>>,
    /// **Every register this supervisor polls**, keyed by [`SINGLE_FLEET_KEY`]
    /// for the single-fleet fallback or by an Armada Fleet's own name — each
    /// with its own cached answer and its own retry clock (§2.2's History).
    registers: BTreeMap<String, RegisterCache>,
    /// The refusals last reported, as text, so an unchanged set is said once.
    refused: String,
    notes: BTreeSet<String>,
    /// The supervisor's own standing findings — only `monitor/register`,
    /// which is not a §6 condition.
    findings: BTreeMap<String, String>,
    /// §6's announcement record, shared with every worker.
    record: Arc<Record>,
    /// Tool proofs: success is kept; a failure is kept for [`PROOF_RETRY`].
    proofs: BTreeMap<(PathBuf, Signal), Result<(), (String, Instant)>>,
    /// §5.4: the `keep-days` each machine and signal last had.
    keep: BTreeMap<(String, Signal), u64>,
    /// §5.4: the `keep-frames` each machine last had.
    keep_frames: BTreeMap<String, u64>,
    /// **How many retention sweeps have run.** A worker re-reads its memory of
    /// the last frame stored per holder when this changes, so it can never say
    /// `unchanged` about a PNG a sweep has just retired (§4.3, §5.4).
    sweeps: u64,
    /// The folder retention may sweep — only while the monitor is accepted.
    folder: Option<PathBuf>,
}

impl Supervisor {
    fn tick(&mut self, path_override: Option<&Path>, report: &Reporter, stop: &Arc<AtomicBool>) {
        let loaded = match config::load(path_override) {
            Ok(loaded) => loaded,
            Err(why) => {
                // The same rule `SPEC-01` §5.5.3a applies to backups: nothing
                // runs on a file that will not load. The gap this leaves is
                // written as `not-captured` when capture resumes.
                self.refuse(report, vec![Refusal { key: "config", why }]);
                self.pause_all();
                return;
            }
        };
        let config = loaded.config;
        let Some(monitor) = config.monitor.clone() else {
            self.refuse(report, Vec::new());
            // No `[monitor]` table is no monitor: every subject is gone, so
            // every condition the monitor had standing clears (§6.1).
            announce(report, self.record.subjects(Subjects::default(), clock::now()));
            self.note(
                report,
                format!(
                    "{} has no [monitor] table, so nothing is captured (SPEC-02 §1)",
                    loaded.path.display()
                ),
            );
            self.retire_all();
            return;
        };
        let refusals = config.monitor_refusals();
        if !refusals.is_empty() {
            self.refuse(report, refusals);
            self.pause_all();
            return;
        }
        self.refuse(report, Vec::new());
        let Some(folder) = monitor.folder() else {
            return;
        };
        self.folder = Some(folder.clone());
        self.note(report, format!("capturing into {}", folder.display()));

        let machines = monitor.resolve();
        // §6.1: judged on the accepted file alone, so a machine, signal or
        // setting removed clears its conditions whether or not the register
        // can be read.
        announce(report, self.record.subjects(Subjects::of(&machines), clock::now()));
        for machine in &machines {
            for signal in &machine.signals {
                self.keep
                    .insert((machine.name.clone(), signal.signal), signal.keep_days);
                if let Some(frames) = signal.keep_frames {
                    self.keep_frames.insert(machine.name.clone(), frames);
                }
            }
        }

        // **The Armada Registry, read before every tick's register fetch** —
        // `crate::armada`, Rivet `ADR-059` §6.3. Its own `read` treats an
        // absent registry or an empty `fleets/` as no Fleets at all, which is
        // this branch's cue to fall back to the single `[fleet]
        // server`/`name` path exactly as it always worked (behaviour
        // preserved for a single-fleet setup). A `fleets/*.toml` that would
        // not parse is reported once as a note and skipped; it never blocks
        // the Fleets that did read.
        let (armada_fleets, armada_problems) = armada::read(None);
        for problem in armada_problems {
            self.note(report, format!("armada: {problem}"));
        }

        let devices: BTreeMap<String, Device> = if armada_fleets.is_empty() {
            let Some(register) = self.register(&config, report) else {
                self.pause_all();
                return;
            };
            register
                .devices
                .into_iter()
                .map(|d| (d.name.clone(), d))
                .collect()
        } else {
            let registers = self.armada_registers(&config, &armada_fleets, report);
            if registers.is_empty() {
                self.pause_all();
                return;
            }
            // **The union of every Fleet's devices, keyed by device name so
            // two Fleets cannot collide.** Fleets are walked in
            // `armada::read`'s own sorted-by-file-name order, so when two
            // Fleets do share a device name the first one alphabetically
            // wins, deterministically, and the loser is named in a note
            // rather than silently shadowed.
            let mut merged: BTreeMap<String, Device> = BTreeMap::new();
            let mut duplicates: BTreeSet<String> = BTreeSet::new();
            for fleet in &armada_fleets {
                let Some(register) = registers.get(&fleet.name) else {
                    continue;
                };
                for device in &register.devices {
                    if merged.contains_key(&device.name) {
                        duplicates.insert(device.name.clone());
                        continue;
                    }
                    merged.insert(device.name.clone(), device.clone());
                }
            }
            for name in duplicates {
                self.note(
                    report,
                    format!(
                        "{name:?} is a device name in more than one Fleet in the Armada \
                         Registry; the Fleet that sorts first by name is the one captured \
                         (SPEC-02 §2.2)."
                    ),
                );
            }
            merged
        };

        let (Some(tool), Some(identity)) = (
            config.tool.path.as_deref().map(config::expand_tilde),
            config.fleet.identity.as_deref().map(config::expand_tilde),
        ) else {
            return;
        };
        let timeout = Duration::from_secs(monitor.timeout_seconds);

        let mut wanted: BTreeSet<String> = BTreeSet::new();
        for machine in machines {
            wanted.insert(machine.name.clone());
            // §2.2 through §6.6: judged against the register being captured
            // with, on every tick that has one — which is this tick acting.
            let key = format!("capture/{}/register", machine.name);
            let mut reading = Reading::default();
            reading.evaluated.insert(key.clone());
            let Some(device) = devices.get(&machine.name) else {
                let held = if devices.is_empty() {
                    "(the register is empty)".to_string()
                } else {
                    devices.keys().map(String::as_str).collect::<Vec<_>>().join(", ")
                };
                reading.present.insert(
                    key,
                    format!(
                        "[[monitor.machine]] {:?} is in NO register, so it is not captured. A \
                         machine gets renamed, and a monitor entry that quietly matched nothing \
                         would be a flight recorder that stopped recording (SPEC-02 §2.2). The \
                         register's names are: {held}",
                        machine.name
                    ),
                );
                announce(report, self.record.settle(&reading, clock::now()));
                self.pause(&machine.name);
                continue;
            };
            announce(report, self.record.settle(&reading, clock::now()));

            let signals: Vec<ResolvedSignal> = machine.signals.clone();
            let mut unproven = BTreeMap::new();
            for signal in &signals {
                if let Err(why) = self.prove(&tool, signal.signal) {
                    unproven.insert(signal.signal, why);
                }
            }
            let filters = machine
                .signals
                .iter()
                .filter_map(|s| s.services.clone().map(|names| (s.signal, names)))
                .collect();
            let target = device.target().map(|mut target| {
                target.name = config.dns_name(&device.name);
                target
            });
            let plan = Plan {
                folder: folder.clone(),
                tool: tool.clone(),
                identity: identity.clone(),
                timeout,
                target,
                signals,
                unproven,
                filters,
                conditions: machine.conditions,
                sweeps: self.sweeps,
            };
            self.assign(&machine.name, plan, report, stop);
        }

        // §3.5: a machine removed from the file stops being captured. Its data
        // stays, and ages out under the keep-days it last had (§5.4).
        let gone: Vec<String> = self
            .workers
            .keys()
            .filter(|name| !wanted.contains(*name))
            .cloned()
            .collect();
        for name in gone {
            self.retire(&name);
        }
        // Its conditions were cleared above, by `Record::subjects`.
    }

    /// **The single-fleet fallback register**, or `None` when `fleet.server`
    /// is not configured. Used only when [`armada::read`] found no Fleets —
    /// `Supervisor::tick`'s fallback, unchanged from before this module knew
    /// about the Armada Registry.
    fn register(&mut self, config: &config::Config, report: &Reporter) -> Option<Register> {
        let server = config.fleet.server.clone()?;
        let from = (server, config.fleet.name.clone(), config.authority());
        let poll = Duration::from_secs(config.fleet.poll_seconds.max(30));
        let cache = self.registers.entry(SINGLE_FLEET_KEY.to_string()).or_default();
        cache.poll(from, poll, &mut self.findings, report, "monitor/register")
    }

    /// **Every Armada Fleet's register, read the same way the single-fleet
    /// path always read its one** — `register::fetch` against the Fleet's
    /// PORTAL ([`armada::Fleet::portal`], the server's host on the portal's
    /// default port), verifying the certificate for the Fleet's name as a DNS
    /// name ([`config::Config::dns_name`], `SPEC-01` §2.5) — the name the
    /// single-fleet path reaches the same server by. `pin` is never read here;
    /// it is the control plane's key and this is a portal read
    /// (`crate::armada`'s module doc).
    ///
    /// **The authority is shared** — `config.authority()`, the same
    /// `[fleet] authority` (or its default) the single-fleet path uses.
    /// Nothing in a Fleet file names a different one, so every Fleet is read
    /// as this operator's own certificate against this operator's own root.
    ///
    /// Returned keyed by Fleet name; a Fleet whose read failed and has never
    /// succeeded is simply absent from the map, same as the single-fleet
    /// fallback returning `None`.
    fn armada_registers(
        &mut self,
        config: &config::Config,
        fleets: &[armada::Fleet],
        report: &Reporter,
    ) -> BTreeMap<String, Register> {
        let authority = config.authority();
        let poll = Duration::from_secs(config.fleet.poll_seconds.max(30));
        let mut out = BTreeMap::new();
        for fleet in fleets {
            let from = (fleet.portal(), Some(config.dns_name(&fleet.name)), authority.clone());
            let key = format!("monitor/register/{}", fleet.name);
            let cache = self.registers.entry(fleet.name.clone()).or_default();
            if let Some(register) = cache.poll(from, poll, &mut self.findings, report, &key) {
                out.insert(fleet.name.clone(), register);
            }
        }
        out
    }

    /// Prove the tool for one subject, from the cache when it can.
    fn prove(&mut self, tool: &Path, signal: Signal) -> Result<(), String> {
        let key = (tool.to_path_buf(), signal);
        match self.proofs.get(&key) {
            Some(Ok(())) => return Ok(()),
            Some(Err((why, at))) if at.elapsed() < PROOF_RETRY => return Err(why.clone()),
            _ => {}
        }
        let proved = match signal {
            Signal::Log => crate::tool::prove_log(tool),
            Signal::Screenshot => crate::tool::prove_screenshot(tool),
            _ => crate::tool::prove_get(tool, signal.name()),
        };
        self.proofs.insert(
            key,
            proved.clone().map_err(|why| (why, Instant::now())),
        );
        proved
    }

    fn assign(&mut self, name: &str, plan: Plan, report: &Reporter, stop: &Arc<AtomicBool>) {
        if let Some((slot, _)) = self.workers.get(name) {
            if let Ok(mut held) = slot.lock() {
                held.plan = Some(plan);
            }
            return;
        }
        let slot = Arc::new(Mutex::new(Slot {
            alive: true,
            plan: Some(plan),
        }));
        let handle = {
            let (machine, slot, report, stop, record) = (
                name.to_string(),
                Arc::clone(&slot),
                Arc::clone(report),
                Arc::clone(stop),
                Arc::clone(&self.record),
            );
            std::thread::spawn(move || worker(&machine, &slot, &report, &stop, &record))
        };
        self.workers.insert(name.to_string(), (slot, handle));
    }

    fn pause(&self, name: &str) {
        if let Some((slot, _)) = self.workers.get(name)
            && let Ok(mut held) = slot.lock()
        {
            held.plan = None;
        }
    }

    fn pause_all(&mut self) {
        self.folder = None;
        for (slot, _) in self.workers.values() {
            if let Ok(mut held) = slot.lock() {
                held.plan = None;
            }
        }
    }

    fn retire(&mut self, name: &str) {
        if let Some((slot, handle)) = self.workers.remove(name) {
            if let Ok(mut held) = slot.lock() {
                held.alive = false;
                held.plan = None;
            }
            self.retired.push(handle);
        }
    }

    fn retire_all(&mut self) {
        self.folder = None;
        let names: Vec<String> = self.workers.keys().cloned().collect();
        for name in names {
            self.retire(&name);
        }
    }

    /// End every worker and wait for each to finish the capture it is in.
    fn finish(mut self) {
        self.retire_all();
        for handle in self.retired {
            let _ = handle.join();
        }
    }

    /// §5.4, over whatever folder is currently accepted. False when there is
    /// none, so the caller tries again next tick rather than in an hour.
    fn retention(&mut self, report: &Reporter) -> bool {
        let Some(folder) = self.folder.clone() else {
            return false;
        };
        let keep = |machine: &str, signal: Signal| {
            self.keep
                .get(&(machine.to_string(), signal))
                .copied()
                .unwrap_or_else(|| signal.default_keep_days())
        };
        let frames = |machine: &str| {
            self.keep_frames
                .get(machine)
                .copied()
                .unwrap_or_else(|| Signal::Screenshot.default_keep_frames())
        };
        let (removed, trouble) = store::retain(&folder, &keep, &frames, clock::local_today());
        // Every worker re-reads which frame it last stored: this sweep may have
        // retired the one it remembers (§4.3).
        self.sweeps = self.sweeps.saturating_add(1);
        report(Event::Retired { removed, trouble });
        true
    }

    fn refuse(&mut self, report: &Reporter, refusals: Vec<Refusal>) {
        let text = refusals
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        if text != self.refused {
            self.refused = text;
            report(Event::Refused(refusals));
        }
    }

    fn note(&mut self, report: &Reporter, note: String) {
        if self.notes.insert(note.clone()) {
            report(Event::Note(note));
        }
    }
}

/// Report a finding once, when it first appears.
fn raise(findings: &mut BTreeMap<String, String>, report: &Reporter, key: &str, message: String) {
    if !findings.contains_key(key) {
        findings.insert(key.to_string(), message.clone());
        report(Event::Finding {
            key: key.to_string(),
            message: Some(message),
        });
    }
}

/// Report a finding gone, once, when its check ran and did not find it.
fn clear(findings: &mut BTreeMap<String, String>, report: &Reporter, key: &str) {
    if findings.remove(key).is_some() {
        report(Event::Finding {
            key: key.to_string(),
            message: None,
        });
    }
}

/// Say what settling produced.
fn announce(report: &Reporter, settled: Settled) {
    for one in settled.announcements {
        report(Event::Announced(one));
    }
    if let Some(why) = settled.trouble {
        report(Event::Unrecorded(why));
    }
}

/// **One machine's worker.**
fn worker(
    machine: &str,
    slot: &Arc<Mutex<Slot>>,
    report: &Reporter,
    stop: &AtomicBool,
    record: &Record,
) {
    let mut last: BTreeMap<Signal, Option<u64>> = BTreeMap::new();
    let mut seeded_from: Option<PathBuf> = None;
    let mut seeded_after: Option<u64> = None;
    let mut memory = Memory::default();
    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let plan = match slot.lock() {
            Ok(held) if held.alive => held.plan.clone(),
            _ => return,
        };
        let Some(plan) = plan else {
            std::thread::sleep(WORKER_TICK);
            continue;
        };
        // §3.4 and §5.3 both need the last attempt, and after a restart only
        // the disk knows it.
        if seeded_from.as_ref() != Some(&plan.folder) {
            last.clear();
            // The frames stored in another folder say nothing about this one.
            memory.stored_frames = None;
            seeded_from = Some(plan.folder.clone());
        }
        // §5.4: a retention sweep may have retired the frame this worker
        // remembers storing, and `unchanged` against a PNG that is gone would
        // keep nothing at all. Read it off the index again.
        if seeded_after != Some(plan.sweeps) {
            memory.stored_frames = None;
            seeded_after = Some(plan.sweeps);
        }
        for signal in &plan.signals {
            last.entry(signal.signal)
                .or_insert_with(|| store::last_attempt(&plan.folder, machine, signal.signal));
        }
        let now = clock::now();
        let Some(due) = most_overdue(&plan.signals, &last, now) else {
            std::thread::sleep(WORKER_TICK);
            continue;
        };
        let Some(signal) = plan.signals.iter().find(|s| s.signal == due) else {
            continue;
        };
        let previous = last.get(&due).copied().flatten();
        let context = Context {
            machine,
            plan: &plan,
            report,
            record,
        };
        capture_one(&context, signal, previous, now, &mut memory);
        last.insert(due, Some(now));
    }
}

/// **What one machine's worker remembers between attempts.**
#[derive(Debug, Default)]
struct Memory {
    /// §6's memory of this machine — misses, a quiet journal, the frame last
    /// seen. In memory only (`crate::conditions`).
    watch: Watch,
    /// §4.3: the frame timestamp last STORED for each holder. `None` until the
    /// first screenshot attempt reads it off the index
    /// ([`store::last_stored_frames`]), which is how it survives a restart;
    /// after that it is updated only when a `stored` line has been written, so
    /// it never says more than the index does.
    stored_frames: Option<BTreeMap<String, i64>>,
}

/// What one capture needs besides the signal and the time.
struct Context<'a> {
    machine: &'a str,
    plan: &'a Plan,
    report: &'a Reporter,
    record: &'a Record,
}

impl Context<'_> {
    /// **This attempt acted: settle what it checked** — `crate::conditions`.
    ///
    /// A `not-captured` marker that could not be written ahead of this
    /// attempt is a write that failed as much as the attempt's own line, so it
    /// counts as this attempt's (§5.5) — one settle, rather than a raise and a
    /// clearance a moment apart.
    ///
    /// `frame` is an answered screenshot's observation (§6.4), settled in the
    /// same breath as the attempt that saw it.
    #[allow(clippy::too_many_arguments)]
    fn settle(
        &self,
        watch: &mut Watch,
        signal: Signal,
        now: u64,
        outcome: &Outcome,
        stored: &Result<PathBuf, String>,
        marker: Option<&str>,
        log: Option<&LogSummary>,
        frame: Option<&Frame>,
    ) {
        let stored = match (stored, marker) {
            (Err(why), _) => Err(why.as_str()),
            (Ok(_), Some(why)) => Err(why),
            (Ok(_), None) => Ok(()),
        };
        let mut reading = watch.attempt(
            self.plan.conditions,
            &Attempted {
                machine: self.machine,
                signal,
                captured_at: now,
                outcome,
                stored,
                log,
                filters: &self.plan.filters,
            },
        );
        if let Some(frame) = frame {
            let frozen = watch.screenshot(self.machine, self.plan.conditions, frame);
            reading.present.extend(frozen.present);
            reading.evaluated.extend(frozen.evaluated);
        }
        announce(self.report, self.record.settle(&reading, now));
    }
}

/// **One capture, its marker if one is owed, and its line.**
fn capture_one(
    context: &Context<'_>,
    signal: &ResolvedSignal,
    previous: Option<u64>,
    now: u64,
    memory: &mut Memory,
) {
    let Context {
        machine,
        plan,
        report,
        ..
    } = *context;
    // §5.1: the PC's local date at capture, taken with the capture's own time
    // — and, for a stored screenshot, its local time of day from the same
    // reading.
    let local = clock::local_now();
    let file = store::day_file(&plan.folder, machine, signal.signal, local.0);

    let mut marker = None;
    if let Some((from, to)) = store::gap(previous, now, signal.every_seconds) {
        let stored = store::append(&plan.folder, machine, &file, &store::marker_line(from, to))
            .map(|()| file.clone());
        marker = stored
            .as_ref()
            .err()
            .map(|why| format!("the not-captured marker could not be written: {why}"));
        report(Event::NotCaptured {
            machine: machine.to_string(),
            signal: signal.signal,
            from,
            to,
            stored,
        });
    }

    match signal.signal {
        Signal::Log => {
            capture_log(context, signal, now, &file, marker.as_deref(), &mut memory.watch);
            return;
        }
        Signal::Screenshot => {
            capture_screenshot(context, now, &local, &file, marker.as_deref(), memory);
            return;
        }
        _ => {}
    }

    let attempt = if let Some(why) = plan.unproven.get(&signal.signal) {
        Attempt {
            outcome: Outcome::Failed(why.clone()),
            elapsed_ms: 0,
        }
    } else {
        match &plan.target {
            Err(why) => Attempt {
                outcome: Outcome::Failed(format!(
                    "the register cannot place {machine}, so there is nowhere to ask: {why}"
                )),
                elapsed_ms: 0,
            },
            Ok(target) => capture::spot(
                &plan.tool,
                target,
                &plan.identity,
                signal.signal.name(),
                plan.timeout,
            ),
        }
    };

    let services = (signal.signal == Signal::Status)
        .then_some(signal.services.as_deref())
        .flatten();
    let line = store::attempt_line(now, attempt.elapsed_ms, &attempt.outcome, services);
    let stored = store::append(&plan.folder, machine, &file, &line).map(|()| file.clone());
    let reason = match &attempt.outcome {
        Outcome::Answered { .. } => None,
        Outcome::Missed(why) | Outcome::Failed(why) => Some(why.clone()),
    };
    report(Event::Attempt {
        machine: machine.to_string(),
        signal: signal.signal,
        captured_at: now,
        elapsed_ms: attempt.elapsed_ms,
        outcome: attempt.outcome.word(),
        reason,
        stored: stored.clone(),
        log: None,
        screenshot: None,
    });
    context.settle(
        &mut memory.watch,
        signal.signal,
        now,
        &attempt.outcome,
        &stored,
        marker.as_deref(),
        None,
        None,
    );
}

/// **One screenshot capture** — §4.3: ready the staging file, ask the tool for
/// the display holder's frame, then [`finish_screenshot`].
fn capture_screenshot(
    context: &Context<'_>,
    now: u64,
    local: &(i64, String),
    file: &Path,
    marker: Option<&str>,
    memory: &mut Memory,
) {
    let Context {
        machine,
        plan,
        report,
        ..
    } = *context;
    let failed = |why: String| Attempt {
        outcome: Outcome::Failed(why),
        elapsed_ms: 0,
    };
    let asked = if let Some(why) = plan.unproven.get(&Signal::Screenshot) {
        failed(why.clone())
    } else {
        match &plan.target {
            Err(why) => failed(format!(
                "the register cannot place {machine}, so there is nowhere to ask: {why}"
            )),
            Ok(target) => match store::prepare_staging(&plan.folder, machine) {
                Err(why) => failed(why),
                Ok(leftover) => {
                    if leftover {
                        report(Event::Note(format!(
                            "{machine}/screenshot: a staged frame left by an interrupted attempt \
                             was removed before this fetch (SPEC-02 §4.3)"
                        )));
                    }
                    capture::screenshot(
                        &plan.tool,
                        target,
                        &plan.identity,
                        &store::screenshot_staging(&plan.folder, machine),
                        plan.timeout,
                    )
                }
            },
        }
    };
    finish_screenshot(context, now, local, file, marker, memory, asked);
}

/// **What the tool's answer comes to** — split from [`capture_screenshot`] so a
/// test can hand it an answer and a staged file, and see every file effect,
/// line and announcement that follows.
///
/// * A `missed` or `failed` attempt keeps nothing: a staged file a killed tool
///   left half-written is removed, and its line is §5.2's usual one.
/// * An answer that is not a readable screenshot object is `failed`.
/// * Otherwise [`store::judge_frame`] against the last stored timestamp per
///   holder, [`store::place_screenshot`], then the index line — and only when
///   that line is written is a `stored` frame remembered.
/// * **Every answered attempt feeds §6.4's `frozen/`**, whatever the verdict
///   and whether or not its line could be written: the machine's answer is a
///   true observation either way.
fn finish_screenshot(
    context: &Context<'_>,
    now: u64,
    local: &(i64, String),
    file: &Path,
    marker: Option<&str>,
    memory: &mut Memory,
    asked: Attempt,
) {
    let Context {
        machine,
        plan,
        report,
        ..
    } = *context;
    let read = match &asked.outcome {
        Outcome::Answered { value, .. } => Some(capture::read_screenshot(value)),
        Outcome::Missed(_) | Outcome::Failed(_) => None,
    };
    let (attempt, shot) = match read {
        Some(Ok(shot)) => (asked, Some(shot)),
        Some(Err(why)) => (
            Attempt {
                outcome: Outcome::Failed(why),
                elapsed_ms: asked.elapsed_ms,
            },
            None,
        ),
        None => (asked, None),
    };

    let stored_frames = memory
        .stored_frames
        .get_or_insert_with(|| store::last_stored_frames(&plan.folder, machine));
    let (line, trouble, summary) = match &shot {
        None => {
            // Nothing is kept from an attempt that did not answer. A removal
            // that fails here is tried again, loudly, before the next fetch.
            let _ = store::discard_staging(&plan.folder, machine);
            (
                store::attempt_line(now, attempt.elapsed_ms, &attempt.outcome, None),
                None,
                None,
            )
        }
        Some(shot) => {
            let verdict = store::judge_frame(shot, stored_frames);
            let (name, trouble) =
                match store::place_screenshot(&plan.folder, machine, verdict, shot, local.0, &local.1) {
                    Ok(name) => (name, None),
                    Err(why) => (None, Some(why)),
                };
            let line = store::screenshot_line(
                now,
                attempt.elapsed_ms,
                shot,
                verdict,
                name.as_deref(),
                trouble.as_deref(),
            );
            let summary = ShotSummary {
                result: if verdict == store::Verdict::Store && trouble.is_some() {
                    "not stored"
                } else {
                    verdict.word()
                },
                holder: shot.holder.clone(),
                frame_modified_ms: shot.frame_modified_ms,
                file: name,
            };
            (line, trouble, Some(summary))
        }
    };
    let written = store::append(&plan.folder, machine, file, &line).map(|()| file.to_path_buf());
    if written.is_ok()
        && let Some(ShotSummary {
            result: "stored",
            holder: Some(holder),
            frame_modified_ms: Some(ms),
            ..
        }) = &summary
    {
        stored_frames.insert(holder.clone(), *ms);
    }
    let stored = match (written, trouble) {
        (Err(why), _) | (Ok(_), Some(why)) => Err(why),
        (Ok(path), None) => Ok(path),
    };
    let reason = match &attempt.outcome {
        Outcome::Answered { .. } => None,
        Outcome::Missed(why) | Outcome::Failed(why) => Some(why.clone()),
    };
    report(Event::Attempt {
        machine: machine.to_string(),
        signal: Signal::Screenshot,
        captured_at: now,
        elapsed_ms: attempt.elapsed_ms,
        outcome: attempt.outcome.word(),
        reason,
        stored: stored.clone(),
        log: None,
        screenshot: summary,
    });
    let frame = shot.map(|shot| Frame {
        holder: shot.holder,
        stamp: shot.frame_modified_ms.map(|ms| ms.to_string()),
        captured_at: now,
    });
    context.settle(
        &mut memory.watch,
        Signal::Screenshot,
        now,
        &attempt.outcome,
        &stored,
        marker,
        None,
        frame.as_ref(),
    );
}

/// **One log capture** — §4.2: read the cursor held, ask from it, store the
/// records, and only then move the cursor.
fn capture_log(
    context: &Context<'_>,
    signal: &ResolvedSignal,
    now: u64,
    file: &Path,
    marker: Option<&str>,
    watch: &mut Watch,
) {
    let Context {
        machine,
        plan,
        report,
        ..
    } = *context;
    let failed = |why: String| Attempt {
        outcome: Outcome::Failed(why),
        elapsed_ms: 0,
    };
    let held = store::read_cursor(&plan.folder, machine);
    let previous = held.clone().ok().flatten();
    let asked = if let Some(why) = plan.unproven.get(&Signal::Log) {
        failed(why.clone())
    } else {
        match (&plan.target, &held) {
            (_, Err(why)) => failed(why.clone()),
            (Err(why), _) => failed(format!(
                "the register cannot place {machine}, so there is nowhere to ask: {why}"
            )),
            (Ok(target), Ok(cursor)) => capture::log(
                &plan.tool,
                target,
                &plan.identity,
                cursor.as_deref(),
                plan.timeout,
            ),
        }
    };

    // An answer that is JSON about `log` must still read as a log before
    // anything is stored or the cursor moves.
    let read = match &asked.outcome {
        Outcome::Answered { value, .. } => Some(capture::read_log(value, previous.is_some())),
        Outcome::Missed(_) | Outcome::Failed(_) => None,
    };
    let (attempt, answer) = match read {
        Some(Ok(answer)) => (asked, Some(answer)),
        Some(Err(why)) => (
            Attempt {
                outcome: Outcome::Failed(why),
                elapsed_ms: asked.elapsed_ms,
            },
            None,
        ),
        None => (asked, None),
    };

    let (stored, summary) = match &answer {
        None => {
            let line = store::attempt_line(now, attempt.elapsed_ms, &attempt.outcome, None);
            (
                store::append(&plan.folder, machine, file, &line).map(|()| file.to_path_buf()),
                None,
            )
        }
        Some(answer) => {
            let written = store::log_lines(
                now,
                attempt.elapsed_ms,
                answer,
                previous.as_deref(),
                signal.services.as_deref(),
            );
            let committed = store::commit_log(
                &plan.folder,
                machine,
                file,
                &written.lines,
                answer.cursor.as_deref(),
            );
            let summary = LogSummary {
                returned: answer.records.len(),
                stored: written.stored,
                unreadable_records: written.unreadable_records,
                first: previous.is_none(),
                boot_gone: answer.boot_gone,
                cursor_moved: committed.is_ok() && answer.cursor.is_some(),
                // §6.9, from the records as the machine sent them.
                card: conditions::card_clock(&answer.records),
            };
            (committed.map(|()| file.to_path_buf()), Some(summary))
        }
    };
    let reason = match &attempt.outcome {
        Outcome::Answered { .. } => None,
        Outcome::Missed(why) | Outcome::Failed(why) => Some(why.clone()),
    };
    report(Event::Attempt {
        machine: machine.to_string(),
        signal: Signal::Log,
        captured_at: now,
        elapsed_ms: attempt.elapsed_ms,
        outcome: attempt.outcome.word(),
        reason,
        stored: stored.clone(),
        log: summary.clone(),
        screenshot: None,
    });
    context.settle(watch, Signal::Log, now, &attempt.outcome, &stored, marker, summary.as_ref(), None);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn signal(signal: Signal, every_seconds: u64) -> ResolvedSignal {
        ResolvedSignal {
            signal,
            every_seconds,
            keep_days: 14,
            keep_frames: signal.takes_keep_frames().then_some(3),
            services: None,
        }
    }

    /// **§3.4: due from the last ATTEMPT**, whatever it was — and not before.
    #[test]
    fn a_signal_is_due_every_seconds_after_its_last_attempt() {
        let signals = vec![signal(Signal::Thermal, 60)];
        let at = |then: u64| BTreeMap::from([(Signal::Thermal, Some(then))]);
        assert_eq!(most_overdue(&signals, &at(1_000), 1_059), None);
        assert_eq!(most_overdue(&signals, &at(1_000), 1_060), Some(Signal::Thermal));
        assert_eq!(
            most_overdue(&signals, &BTreeMap::from([(Signal::Thermal, None)]), 5),
            Some(Signal::Thermal),
            "never attempted is due at once"
        );
        assert_eq!(
            most_overdue(&signals, &at(9_000), 1_000),
            Some(Signal::Thermal),
            "an attempt in the future means the clock went back; it must not go silent"
        );
    }

    /// **§3.2: the most overdue first**, and never-attempted before all.
    #[test]
    fn the_most_overdue_signal_goes_first() {
        let signals = vec![
            signal(Signal::Status, 60),
            signal(Signal::Memory, 300),
            signal(Signal::Storage, 3_600),
        ];
        let last = BTreeMap::from([
            (Signal::Status, Some(10_000 - 70)),   // 10 s overdue
            (Signal::Memory, Some(10_000 - 400)),  // 100 s overdue
            (Signal::Storage, Some(10_000 - 100)), // not due
        ]);
        assert_eq!(most_overdue(&signals, &last, 10_000), Some(Signal::Memory));

        let mut fresh = last.clone();
        fresh.insert(Signal::Storage, None);
        assert_eq!(most_overdue(&signals, &fresh, 10_000), Some(Signal::Storage));
    }

    // -- the screenshot, through the worker's own path -------------------------

    use crate::alarm::Change;
    use crate::config::ResolvedMachine;
    use crate::tool::Finished;

    const M: &str = "rivet-01";
    /// 2026-09-15T08:49:03Z.
    const T0: u64 = 1_789_462_143;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("rivet-monitor-tests")
            .join("monitor")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    fn answer(holder: &str, ms: i64) -> String {
        format!(
            r#"{{"outcome":"fetched","holder":"{holder}","board_path":"/state/app/{holder}/data/screen.qoi","frame_modified_ms":{ms},"out":"staging.png"}}"#
        )
    }

    const NOBODY: &str = r#"{"outcome":"no-application-holds-the-display","holder":null,"board_path":null,"frame_modified_ms":null,"out":null}"#;

    /// A machine capturing only `screenshot` every 30 s, its record, and every
    /// event it reports.
    struct Rig {
        plan: Plan,
        record: Record,
        report: Reporter,
        events: Arc<Mutex<Vec<Event>>>,
    }

    impl Rig {
        fn new(name: &str, frozen_minutes: u64) -> Self {
            let folder = scratch(name);
            let conditions = Conditions {
                reach_misses: 3,
                log_stalled_minutes: 10,
                frozen_minutes,
                card_stalled_minutes: 70,
                power: true,
            };
            let signals = vec![signal(Signal::Screenshot, 30)];
            let record = Record::open(Some(folder.join(conditions::FILE)));
            record.subjects(
                Subjects::of(&[ResolvedMachine {
                    name: M.to_string(),
                    conditions,
                    signals: signals.clone(),
                }]),
                0,
            );
            let events = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&events);
            Self {
                plan: Plan {
                    folder: folder.join("monitor"),
                    tool: PathBuf::from("cargo-rivet"),
                    identity: PathBuf::from("./id"),
                    timeout: Duration::from_secs(20),
                    target: Err("a test hands the answer in".to_string()),
                    signals,
                    unproven: BTreeMap::new(),
                    filters: Vec::new(),
                    conditions,
                    sweeps: 0,
                },
                record,
                report: Arc::new(move |event| sink.lock().unwrap().push(event)),
                events,
            }
        }

        /// One attempt answered with `stdout` at `at`, the tool having staged
        /// a frame when `staged`.
        fn shoot(&self, memory: &mut Memory, at: u64, stdout: &str, staged: bool) {
            let folder = &self.plan.folder;
            if staged {
                store::prepare_staging(folder, M).unwrap();
                std::fs::write(store::screenshot_staging(folder, M), b"\x89PNG a frame").unwrap();
            }
            let day = clock::days_from_civil(2026, 9, 15);
            let rest = at % 86_400;
            let local = (day, format!("{:02}{:02}{:02}", rest / 3_600, rest % 3_600 / 60, rest % 60));
            let file = store::day_file(folder, M, Signal::Screenshot, day);
            let finished = Finished {
                success: true,
                stdout: stdout.to_string(),
                stderr: String::new(),
                timed_out: false,
                elapsed: Duration::from_millis(900),
            };
            let asked = Attempt {
                outcome: capture::judge_screenshot(&finished, Duration::from_secs(20)),
                elapsed_ms: 900,
            };
            let context = Context {
                machine: M,
                plan: &self.plan,
                report: &self.report,
                record: &self.record,
            };
            finish_screenshot(&context, at, &local, &file, None, memory, asked);
        }

        fn results(&self) -> Vec<String> {
            let file = store::day_file(&self.plan.folder, M, Signal::Screenshot, clock::days_from_civil(2026, 9, 15));
            std::fs::read_to_string(file)
                .unwrap_or_default()
                .lines()
                .map(|line| {
                    let value: serde_json::Value = serde_json::from_str(line).expect("every index line is JSON");
                    value.get("result").or_else(|| value.get("outcome")).and_then(serde_json::Value::as_str).unwrap().to_string()
                })
                .collect()
        }

        fn pngs(&self) -> usize {
            let dir = store::signal_dir(&self.plan.folder, M, Signal::Screenshot).join("2026-09-15");
            std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
        }

        fn staging_left(&self) -> bool {
            store::screenshot_staging(&self.plan.folder, M).exists()
        }

        /// Every announcement so far, as (key, change).
        fn announced(&self) -> Vec<(String, Change)> {
            self.events
                .lock()
                .unwrap()
                .iter()
                .filter_map(|event| match event {
                    Event::Announced(one) => Some((one.key.clone(), one.change)),
                    _ => None,
                })
                .collect()
        }
    }

    /// **§6.4 fed by §4.3**: every answered screenshot attempt is an
    /// observation. A holder seen drawing whose frame then holds for
    /// `frozen-minutes` raises `frozen/` once; the frame advancing clears it
    /// once — and the index and the files say stored, stored, unchanged…,
    /// stored, with no staged PNG left behind.
    #[test]
    fn every_answered_screenshot_feeds_frozen_which_raises_and_clears_once() {
        let rig = Rig::new("frozen-feed", 1);
        let mut memory = Memory::default();
        rig.shoot(&mut memory, T0, &answer("aura-renderer", 1_789_492_100_748), true);
        for at in [T0 + 30, T0 + 60, T0 + 90] {
            rig.shoot(&mut memory, at, &answer("aura-renderer", 1_789_492_160_748), true);
        }
        assert_eq!(rig.announced(), vec![("frozen/rivet-01".to_string(), Change::Raised)]);
        rig.shoot(&mut memory, T0 + 120, &answer("aura-renderer", 1_789_492_220_748), true);
        rig.shoot(&mut memory, T0 + 150, NOBODY, false);
        assert_eq!(
            rig.announced(),
            vec![
                ("frozen/rivet-01".to_string(), Change::Raised),
                ("frozen/rivet-01".to_string(), Change::Cleared),
            ]
        );
        assert_eq!(
            rig.results(),
            ["stored", "stored", "unchanged", "unchanged", "stored", "no application holds the display"]
        );
        assert_eq!(rig.pngs(), 3, "one PNG per stored line");
        assert!(!rig.staging_left());
    }

    /// With the default `frozen-minutes` a frame that advances every minute —
    /// Aura's cadence — never raises `frozen/`.
    #[test]
    fn a_frame_advancing_every_minute_never_raises_frozen() {
        let rig = Rig::new("frozen-quiet", 5);
        let mut memory = Memory::default();
        for step in 0..40_u64 {
            let ms = 1_789_492_160_748 + i64::try_from(step / 2).unwrap() * 60_000;
            rig.shoot(&mut memory, T0 + step * 30, &answer("aura-renderer", ms), true);
        }
        assert!(rig.announced().is_empty(), "{:?}", rig.announced());
        assert_eq!(rig.pngs(), 20);
    }

    /// **The restart**: a worker that starts afresh reads the last stored
    /// timestamp per holder off the index, so its first attempt against the
    /// same frame is `unchanged`, not a second copy.
    #[test]
    fn a_restarted_worker_calls_an_unchanged_frame_unchanged() {
        let rig = Rig::new("restart", 5);
        let mut before = Memory::default();
        rig.shoot(&mut before, T0, &answer("aura-renderer", 1_789_492_160_748), true);
        drop(before);

        let mut after = Memory::default();
        rig.shoot(&mut after, T0 + 30, &answer("aura-renderer", 1_789_492_160_748), true);
        assert_eq!(rig.results(), ["stored", "unchanged"]);
        assert_eq!(rig.pngs(), 1);
        assert!(!rig.staging_left());
    }

    /// **An answer that is not a readable screenshot is `failed`**, keeps
    /// nothing — not even the staged file — and raises `capture/`, not
    /// `frozen/`.
    #[test]
    fn an_unreadable_screenshot_answer_is_failed_and_keeps_nothing() {
        let rig = Rig::new("unreadable", 1);
        let mut memory = Memory::default();
        rig.shoot(&mut memory, T0, "holder   aura-renderer holds the display", true);
        assert!(!rig.staging_left());
        rig.shoot(&mut memory, T0 + 30, r#"{"outcome":"fetched","holder":"aura-renderer"}"#, true);
        assert!(!rig.staging_left());
        assert_eq!(rig.results(), ["failed", "failed"]);
        assert_eq!(rig.pngs(), 0);
        assert_eq!(rig.announced(), vec![("capture/rivet-01/screenshot".to_string(), Change::Raised)]);

        rig.shoot(&mut memory, T0 + 60, &answer("aura-renderer", 5), true);
        assert_eq!(rig.announced().last(), Some(&("capture/rivet-01/screenshot".to_string(), Change::Cleared)));
    }
}
