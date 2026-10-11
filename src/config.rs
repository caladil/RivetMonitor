//! **The configuration file** — `SPEC-01` §2.
//!
//! Ignition reads it; this module is the schema and, more importantly, the
//! REFUSALS. Five of its keys have no default on purpose (§2), among them:
//!
//! * `backups.folder` (§2.1), because a backup tool that invents a destination
//!   is one whose backups are somewhere the operator did not choose;
//! * `tool.path`, because the `cargo-rivet` on a workstation's PATH may be a
//!   build with no `backup` verb, and a program that silently ran it would
//!   report "nothing configured" for ever. `crate::tool` proves the binary can
//!   before anything is scheduled.
//!
//! `deny_unknown_fields` is set on every table here. A mistyped key that was
//! quietly ignored is the same failure class as §2.3's unmatched override: a
//! setting the operator believes is in force and is not.

use std::path::{Path, PathBuf};

use ignition::logging::LogConfig;
use serde::{Deserialize, Serialize};

/// The name Ignition places the configuration file under —
/// `%APPDATA%\RivetMonitor\config.toml` on Windows.
pub const APP_NAME: &str = "RivetMonitor";

pub use crate::timetable::Schedule;

/// **What a retired `cadence` key is refused with** — `SPEC-01` §2.2.
///
/// Kept as its own message rather than left to `deny_unknown_fields`, because
/// the operator's file said `cadence` for a month, and *"unknown field"* would
/// tell him he typed it wrong rather than that it was replaced.
const CADENCE_RETIRED: &str = "`cadence` was replaced by `schedule` on 2026-10-02 and is no \
     longer read, so NO BACKUP RUNS until it is changed. Write the days and times instead, \
     like schedule = [\"Tue 04:00\", \"Fri 04:00\"], or schedule = [] for manual only \
     (SPEC-01 §2.2, §3.1).";

fn default_retention() -> u32 {
    3
}

fn default_poll_seconds() -> u64 {
    300
}

/// `local`, the first suffix `rivet-provision ca` records and the one mDNS
/// resolves on this network.
fn default_host_suffix() -> String {
    "local".to_string()
}

/// One `[[backups.override]]` — `SPEC-01` §2.2.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Override {
    /// The machine, **as the register names it** (`SPEC-01` §5.3).
    pub host: String,
    /// The application, as its `.rvt` names it — `tempmon-rivet`.
    pub application: String,
    /// This application's own times; `[]` makes it manual only.
    #[serde(default)]
    pub schedule: Option<Schedule>,
    /// **Retired** — read only so it can be refused by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cadence: Option<toml::Value>,
    #[serde(default)]
    pub retention: Option<u32>,
    #[serde(default)]
    pub folder: Option<PathBuf>,
    /// **Never fetched.** What a very large or very uninteresting application
    /// gets.
    #[serde(default)]
    pub exempt: bool,
}

impl Override {
    /// `host/application`, which is how this program keys everything.
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}/{}", self.host, self.application)
    }
}

/// One `[[backups.machine]]` — a machine's backup switch, `SPEC-01` §2.6.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct BackupMachine {
    /// The machine, **as the register names it** (`SPEC-01` §5.3).
    pub name: String,
    /// Off unless it says so: a machine is backed up only by an operator's
    /// decision, never by being in the register.
    #[serde(default)]
    pub enabled: bool,
}

/// `[backups]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Backups {
    /// **REQUIRED, and there is no default** — §2.1.
    #[serde(default)]
    pub folder: Option<PathBuf>,
    /// **REQUIRED, and there is no default** — §3.1. `[]` is manual only.
    #[serde(default)]
    pub schedule: Option<Schedule>,
    /// **Retired** — read only so it can be refused by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cadence: Option<toml::Value>,
    #[serde(default = "default_retention")]
    pub retention: u32,
    #[serde(default, rename = "override")]
    pub overrides: Vec<Override>,
    /// The per-machine switches — `SPEC-01` §2.6. A machine with no entry, or
    /// one that is not `enabled`, has nothing fetched.
    #[serde(default, rename = "machine")]
    pub machines: Vec<BackupMachine>,
}

impl Default for Backups {
    fn default() -> Self {
        Self {
            folder: None,
            schedule: None,
            cadence: None,
            retention: default_retention(),
            overrides: Vec::new(),
            machines: Vec::new(),
        }
    }
}

/// `[fleet]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Fleet {
    /// `host:port` of `rivet-fleetd` — the register's home. REQUIRED.
    #[serde(default)]
    pub server: Option<String>,
    /// The name in the fleet server's certificate, when `server` names an
    /// address. `SPEC-01` §4.1: a certificate carries a DNS name and an address
    /// matches none of them.
    #[serde(default)]
    pub name: Option<String>,
    /// How often the register is re-read and the schedule re-examined.
    #[serde(default = "default_poll_seconds")]
    pub poll_seconds: u64,
    /// **The DNS suffix a machine's certificate carries**, appended to a
    /// register name that has no dot in it.
    ///
    /// The register holds *labels* — `rivet-01` — and TLS verifies a *DNS
    /// name*; `rivet-provision ca` issues `server-<name>.<suffix>.crt` and
    /// writes the suffixes it used into a `suffix` file beside the authority.
    /// A label with a dot already in it is taken as a full name and left alone.
    #[serde(default = "default_host_suffix")]
    pub host_suffix: String,
    /// **The fleet authority** — `browser.crt`, `browser.key`, `root.crt`.
    /// Defaults to `~/.rivet/ca`, which is `cargo rivet backup`'s own default
    /// and where `rivet-provision ca` puts one.
    #[serde(default)]
    pub authority: Option<PathBuf>,
    /// **The control-plane identity** — a directory holding `device.seed` and
    /// `device.crt`. REQUIRED and NOT defaulted: a workstation that has
    /// provisioned machines holds several, and guessing which one a fleet
    /// accepts would be a silent failure the first time it was wrong.
    #[serde(default)]
    pub identity: Option<PathBuf>,
}

impl Default for Fleet {
    fn default() -> Self {
        Self {
            server: None,
            name: None,
            poll_seconds: default_poll_seconds(),
            host_suffix: default_host_suffix(),
            authority: None,
            identity: None,
        }
    }
}

/// `[tool]` — where `cargo-rivet` is.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Tool {
    /// REQUIRED. See the module note: the PATH build is the trap.
    #[serde(default)]
    pub path: Option<PathBuf>,
}

/// The whole file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub backups: Backups,
    #[serde(default)]
    pub fleet: Fleet,
    #[serde(default)]
    pub tool: Tool,
    /// **Optional.** No `[monitor]` table means no monitor — `SPEC-02` §1.
    #[serde(default)]
    pub monitor: Option<Monitor>,
    /// **Ignition's logging schema, with this program's own default rotation.**
    ///
    /// Ignition rotates its log one of two ways: `"run"`, one file per process
    /// launch, which suits command-line tools and short-lived desktop apps; and
    /// `"size"`, which suits a long-running process, where a per-launch policy
    /// never fires at all.
    ///
    /// **This is a long-running process.** The tray starts at login and runs
    /// for days, so a per-launch policy means one file growing without bound
    /// and a `keep` that never prunes. So **`"size"` is the default whether or
    /// not a `[logging]` table is written** — Ignition's own default inside a
    /// table is `"run"`, and a table that sets only `enabled` or `level` must
    /// not quietly bring it back. An explicit `rotation` is honoured.
    #[serde(default = "daemon_logging", deserialize_with = "daemon_logging_from")]
    pub logging: LogConfig,
}

/// A `[logging]` table, with `rotation = "size"` supplied when it names none —
/// see [`Config::logging`].
fn daemon_logging_from<'de, D: serde::Deserializer<'de>>(d: D) -> Result<LogConfig, D::Error> {
    let mut table = toml::Table::deserialize(d)?;
    table
        .entry("rotation")
        .or_insert_with(|| toml::Value::String(String::from("size")));
    toml::Value::Table(table)
        .try_into()
        .map_err(serde::de::Error::custom)
}

/// [`Config::logging`]'s default: Ignition's, rolled by **size** rather than by
/// run, because this program is a daemon.
///
/// Everything else stays Ignition's — `enabled`, `dir`, `keep`, `level`,
/// `max_bytes` — so a value this program has no opinion about keeps whatever
/// the library decided it should be. That is the point of embedding its schema
/// instead of copying it.
fn daemon_logging() -> LogConfig {
    LogConfig {
        rotation: ignition::logging::Rotation::Size,
        ..LogConfig::default()
    }
}

// ---------------------------------------------------------------------------
// [monitor] — SPEC-02 §2
// ---------------------------------------------------------------------------

/// One thing the monitor can ask a machine — `SPEC-02` §2.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Signal {
    Status,
    Log,
    Thermal,
    Screenshot,
    Memory,
    Boot,
    Storage,
    Slot,
    /// **The kernel's own process table** — added 2026-09-16, and the only
    /// signal here that exists to catch a failure this project has already
    /// seen three times. See `Signal::Process` in `default_every_seconds`.
    Process,
}

/// Every signal, in §2.3's table order.
pub const SIGNALS: [Signal; 9] = [
    Signal::Status,
    Signal::Log,
    Signal::Thermal,
    Signal::Screenshot,
    Signal::Memory,
    Signal::Boot,
    Signal::Storage,
    Signal::Slot,
    Signal::Process,
];

/// **The cadence floor**, and why it is a floor: every capture opens a
/// control-plane connection, and every connection writes records into the
/// board's own journal (`ADR-002` §5).
pub const EVERY_SECONDS_FLOOR: u64 = 30;

impl Signal {
    /// The name the file, the table and the directory all use.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Log => "log",
            Self::Thermal => "thermal",
            Self::Screenshot => "screenshot",
            Self::Memory => "memory",
            Self::Boot => "boot",
            Self::Storage => "storage",
            Self::Slot => "slot",
            Self::Process => "process",
        }
    }

    /// §2.3's default `every-seconds`.
    ///
    /// **The screenshot's is five minutes, and that is a measurement rather
    /// than a taste.** A stored frame is the tool's uncompressed PNG —
    /// 6,222,418 bytes for `rivet-01`'s 1920×1080 display — so a minute's
    /// cadence stores about 9 GB a day per machine. `ADR-002` §2's History has
    /// the ruling.
    #[must_use]
    pub const fn default_every_seconds(self) -> u64 {
        match self {
            Self::Status | Self::Log | Self::Thermal | Self::Process => 60,
            Self::Memory | Self::Boot | Self::Screenshot => 300,
            Self::Storage | Self::Slot => 3_600,
        }
    }

    /// §2.3's default `keep-days` — **every signal's dated files, the
    /// screenshot's index included.**
    ///
    /// The screenshot's stored FRAMES are not dated files and are not kept by
    /// this: they are kept by count, [`Signal::default_keep_frames`], because a
    /// frame is 6.2 MB and a day of them is not comparable to a day of status
    /// lines (`ADR-002` §2's History).
    #[must_use]
    pub const fn default_keep_days(self) -> u64 {
        let _ = self;
        14
    }

    /// §2.3's default `keep-frames`: **the newest three stored frames**.
    ///
    /// Ruled by the maintainer on 2026-09-15 (`R-001`), with the five-minute
    /// cadence, and it is what makes the screenshot affordable: three frames is
    /// about 19 MB a machine, against the 36 GB `keep-days = 3` would have held.
    #[must_use]
    pub const fn default_keep_frames(self) -> u64 {
        let _ = self;
        3
    }

    /// §2.4: `services` narrows these two and nothing else.
    #[must_use]
    pub const fn takes_services(self) -> bool {
        matches!(self, Self::Status | Self::Log)
    }

    /// §2.3: **`keep-frames` is the screenshot's alone**, and `keep-days` is
    /// every other signal's. Neither is accepted on the other, by name.
    #[must_use]
    pub const fn takes_keep_frames(self) -> bool {
        matches!(self, Self::Screenshot)
    }
}

impl std::fmt::Display for Signal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// One signal's table under a `[[monitor.machine]]` — present means captured.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct SignalTable {
    #[serde(default)]
    pub every_seconds: Option<u64>,
    /// §2.3, **every signal but `screenshot`**. Parsed on all of them so that
    /// one on `screenshot` is refused by name, pointing at `keep-frames`.
    #[serde(default)]
    pub keep_days: Option<u64>,
    /// §2.3, **`screenshot` alone**: how many stored frames are kept. Parsed on
    /// every signal for the same reason `keep_days` is.
    #[serde(default)]
    pub keep_frames: Option<u64>,
    /// §2.4. Parsed on every signal so that one on the WRONG signal can be
    /// refused by name, with the reason, rather than as a bare unknown key.
    #[serde(default)]
    pub services: Option<Vec<String>>,
}

/// One `[[monitor.machine]]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct MonitorMachine {
    /// **A REGISTER name** — `SPEC-02` §2.2. Nothing else about the machine may
    /// be written here; address, port, pin and DNS name come from the register.
    pub name: String,
    /// **The machine's health-monitoring switch**, off unless it says so —
    /// `SPEC-01` §2.6. A block that is off keeps its settings and captures
    /// nothing; [`Monitor::resolve`] leaves it out, which is exactly how a
    /// machine removed from the file is treated.
    #[serde(default)]
    pub enabled: bool,
    // §2.5: any condition default, overridden for this machine.
    #[serde(default)]
    pub reach_misses: Option<u32>,
    #[serde(default)]
    pub log_stalled_minutes: Option<u64>,
    #[serde(default)]
    pub frozen_minutes: Option<u64>,
    #[serde(default)]
    pub card_stalled_minutes: Option<u64>,
    #[serde(default)]
    pub power: Option<bool>,
    #[serde(default)]
    pub status: Option<SignalTable>,
    #[serde(default)]
    pub log: Option<SignalTable>,
    #[serde(default)]
    pub thermal: Option<SignalTable>,
    #[serde(default)]
    pub screenshot: Option<SignalTable>,
    #[serde(default)]
    pub memory: Option<SignalTable>,
    #[serde(default)]
    pub boot: Option<SignalTable>,
    #[serde(default)]
    pub storage: Option<SignalTable>,
    #[serde(default)]
    pub slot: Option<SignalTable>,
    #[serde(default)]
    pub process: Option<SignalTable>,
}

impl MonitorMachine {
    /// The table for one signal, when it is present.
    #[must_use]
    pub const fn table(&self, signal: Signal) -> Option<&SignalTable> {
        match signal {
            Signal::Status => self.status.as_ref(),
            Signal::Log => self.log.as_ref(),
            Signal::Thermal => self.thermal.as_ref(),
            Signal::Screenshot => self.screenshot.as_ref(),
            Signal::Memory => self.memory.as_ref(),
            Signal::Boot => self.boot.as_ref(),
            Signal::Storage => self.storage.as_ref(),
            Signal::Slot => self.slot.as_ref(),
            Signal::Process => self.process.as_ref(),
        }
    }
}

const fn default_timeout_seconds() -> u64 {
    20
}
const fn default_reach_misses() -> u32 {
    3
}
const fn default_log_stalled_minutes() -> u64 {
    10
}
/// **It must exceed twice the screenshot cadence, and a configuration where it
/// does not is refused.** At the cadence the maintainer ruled — 300 s — the old
/// default of 5 minutes was one cadence, so `frozen/` would have raised on a
/// single repeated frame. 15 minutes is 900 s against a floor of 600
/// (`SPEC-02` §2.3, §6.4).
const fn default_frozen_minutes() -> u64 {
    15
}
/// `rivet-sysd` writes the clock to the card once an hour and logs it only
/// after the write completes, so 70 minutes is one interval plus room for a
/// late write — `SPEC-02` §6.9.
const fn default_card_stalled_minutes() -> u64 {
    70
}
const fn default_power() -> bool {
    true
}

/// `[monitor]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Monitor {
    /// **REQUIRED, and there is no default** — §2.1.
    #[serde(default)]
    pub folder: Option<PathBuf>,
    /// One capture's limit, enforced by killing the child — §3.3.
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    // §6's condition defaults, which every machine inherits (§2.5). 0 / false
    // switches one off. Parsed and resolved here; the conditions themselves
    // are `SPEC-02` §6, in `conditions.rs`.
    #[serde(default = "default_reach_misses")]
    pub reach_misses: u32,
    #[serde(default = "default_log_stalled_minutes")]
    pub log_stalled_minutes: u64,
    #[serde(default = "default_frozen_minutes")]
    pub frozen_minutes: u64,
    #[serde(default = "default_card_stalled_minutes")]
    pub card_stalled_minutes: u64,
    #[serde(default = "default_power")]
    pub power: bool,
    #[serde(default, rename = "machine")]
    pub machines: Vec<MonitorMachine>,
}

/// §6's thresholds for one machine, defaults and overrides already applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Conditions {
    pub reach_misses: u32,
    pub log_stalled_minutes: u64,
    pub frozen_minutes: u64,
    pub card_stalled_minutes: u64,
    pub power: bool,
}

/// One captured signal, defaults applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSignal {
    pub signal: Signal,
    pub every_seconds: u64,
    /// How long this signal's dated files are kept (§5.4).
    pub keep_days: u64,
    /// **`screenshot` only**: how many stored frames are kept (§5.4). `None`
    /// on every other signal, which keeps nothing by count.
    pub keep_frames: Option<u64>,
    pub services: Option<Vec<String>>,
}

/// One monitored machine, defaults and overrides applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedMachine {
    pub name: String,
    pub conditions: Conditions,
    /// Only the signals whose table is present, in §2.3's order.
    pub signals: Vec<ResolvedSignal>,
}

impl Monitor {
    /// Every machine that is **switched on**, with §2.3's defaults and §2.5's
    /// overrides applied.
    ///
    /// A switched-off machine is absent here, and absence is what stops its
    /// worker and clears its conditions (`SPEC-02` §3.5, §6.1) — the same path
    /// a block deleted from the file takes, so switching off needs no second
    /// mechanism.
    ///
    /// This does not refuse anything; [`Config::monitor_refusals`] does, and
    /// nothing is captured while it has anything to say.
    #[must_use]
    pub fn resolve(&self) -> Vec<ResolvedMachine> {
        self.machines
            .iter()
            .filter(|machine| machine.enabled)
            .map(|machine| ResolvedMachine {
                name: machine.name.clone(),
                conditions: Conditions {
                    reach_misses: machine.reach_misses.unwrap_or(self.reach_misses),
                    log_stalled_minutes: machine
                        .log_stalled_minutes
                        .unwrap_or(self.log_stalled_minutes),
                    frozen_minutes: machine.frozen_minutes.unwrap_or(self.frozen_minutes),
                    card_stalled_minutes: machine
                        .card_stalled_minutes
                        .unwrap_or(self.card_stalled_minutes),
                    power: machine.power.unwrap_or(self.power),
                },
                signals: SIGNALS
                    .iter()
                    .filter_map(|&signal| {
                        let table = machine.table(signal)?;
                        Some(ResolvedSignal {
                            signal,
                            every_seconds: table
                                .every_seconds
                                .unwrap_or_else(|| signal.default_every_seconds()),
                            keep_days: table
                                .keep_days
                                .unwrap_or_else(|| signal.default_keep_days()),
                            keep_frames: signal.takes_keep_frames().then(|| {
                                table
                                    .keep_frames
                                    .unwrap_or_else(|| signal.default_keep_frames())
                            }),
                            services: table.services.clone(),
                        })
                    })
                    .collect(),
            })
            .collect()
    }

    /// The folder, `~` expanded, when one is configured.
    #[must_use]
    pub fn folder(&self) -> Option<PathBuf> {
        self.folder.as_deref().map(expand_tilde)
    }
}

/// A path made comparable: `~` expanded, made absolute, `.` and `..` resolved,
/// and — on Windows — compared without regard to case or `\\?\`.
///
/// **Resolved through the filesystem when the path exists**, so a folder named
/// two ways is still one folder; lexically when it does not, because a folder
/// that does not exist yet is exactly the case a first configuration has.
fn comparable(path: &Path) -> Vec<String> {
    let expanded = expand_tilde(path);
    let absolute = std::fs::canonicalize(&expanded).unwrap_or_else(|_| {
        if expanded.is_absolute() {
            expanded.clone()
        } else {
            std::env::current_dir()
                .map(|d| d.join(&expanded))
                .unwrap_or_else(|_| expanded.clone())
        }
    });
    let text = absolute.to_string_lossy().replace('/', "\\");
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text).to_string();
    let mut parts: Vec<String> = Vec::new();
    for part in text.split('\\') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(if cfg!(windows) {
                other.to_lowercase()
            } else {
                other.to_string()
            }),
        }
    }
    parts
}

/// Is `inner` the same folder as `outer`, or somewhere inside it?
fn same_or_inside(inner: &Path, outer: &Path) -> bool {
    let (inner, outer) = (comparable(inner), comparable(outer));
    !outer.is_empty() && inner.len() >= outer.len() && inner[..outer.len()] == outer[..]
}

/// `~` at the front of a path, replaced with this user's home.
///
/// Ignition expands it for the configuration file's own location; these are
/// paths the file's CONTENT names, so it is done here.
#[must_use]
pub fn expand_tilde(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let Some(rest) = text.strip_prefix('~') else {
        return path.to_path_buf();
    };
    let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) else {
        return path.to_path_buf();
    };
    let rest = rest.trim_start_matches(['/', '\\']);
    if rest.is_empty() {
        PathBuf::from(home)
    } else {
        PathBuf::from(home).join(rest)
    }
}

/// `~/.rivet/ca`, the default authority directory.
#[must_use]
pub fn default_authority() -> PathBuf {
    expand_tilde(Path::new("~/.rivet/ca"))
}

/// Where the file is and what it said.
pub struct Loaded {
    pub config: Config,
    pub path: PathBuf,
    /// True when nothing is on disk yet, so the values are all defaults.
    pub absent: bool,
}

/// Read the configuration, honouring `--config PATH` when one was given.
///
/// # Errors
///
/// A file that is not TOML, a key this build does not define, or a platform
/// with no configuration directory. **A file that does not exist is not an
/// error** — it loads as defaults, and [`Config::refusals`] is then what says
/// the program cannot act.
pub fn load(path_override: Option<&Path>) -> Result<Loaded, String> {
    let mut loader = ignition::config::Loader::new(APP_NAME);
    if let Some(p) = path_override {
        loader = loader.path_override(p);
    }
    let path = loader
        .config_path()
        .ok_or("there is no configuration directory on this system, so there is nowhere for a configuration file to be")?;
    let absent = !path.exists();
    let config: Config = loader
        .try_load()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Loaded {
        config,
        path,
        absent,
    })
}

/// **Where the configuration file would be**, whether or not it parses.
///
/// The path is known before the file is read, and that matters on the failure
/// path: `SPEC-01` §5.5 has to settle a *"this file will not load"* condition,
/// and the record it settles into lives beside the file. A caller that only had
/// [`load`]'s error would know something is wrong and have nowhere to write it
/// down.
#[must_use]
pub fn config_path(path_override: Option<&Path>) -> Option<PathBuf> {
    let mut loader = ignition::config::Loader::new(APP_NAME);
    if let Some(p) = path_override {
        loader = loader.path_override(p);
    }
    loader.config_path()
}

/// A reason this program cannot take a backup, in the operator's words.
///
/// **These are not warnings.** Each one stops every backup, and the icon says
/// so rather than sitting quietly (`SPEC-01` §2.1, §5.4).
#[derive(Debug, Clone)]
pub struct Refusal {
    pub key: &'static str,
    pub why: String,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.key, self.why)
    }
}

impl Config {
    /// **Is this machine's backup switch on?** — `SPEC-01` §2.6. A machine with
    /// no `[[backups.machine]]`, or one that is not `enabled`, is not.
    #[must_use]
    pub fn backups_on(&self, host: &str) -> bool {
        self.backups
            .machines
            .iter()
            .any(|machine| machine.name == host && machine.enabled)
    }

    /// **Is this machine's health-monitoring switch on?** — `SPEC-01` §2.6.
    ///
    /// The one switch over both mechanisms: the flight recorder reads it through
    /// [`Monitor::resolve`], and the backup pass reads it here before asking the
    /// machine how it is.
    #[must_use]
    pub fn monitoring_on(&self, host: &str) -> bool {
        self.monitor.as_ref().is_some_and(|monitor| {
            monitor
                .machines
                .iter()
                .any(|machine| machine.name == host && machine.enabled)
        })
    }

    /// Everything missing that must be present before anything can be fetched.
    #[must_use]
    pub fn refusals(&self) -> Vec<Refusal> {
        let mut out = Vec::new();
        let mut switched: Vec<&str> = Vec::new();
        for machine in &self.backups.machines {
            if switched.contains(&machine.name.as_str()) {
                out.push(Refusal {
                    key: "backups.machine",
                    why: format!(
                        "two [[backups.machine]] entries name {:?}. Whichever applied, the other \
                         would be a switch you believe is in force and is not (SPEC-01 §2.6).",
                        machine.name
                    ),
                });
                continue;
            }
            switched.push(&machine.name);
        }
        if self.backups.folder.is_none() {
            out.push(Refusal {
                key: "backups.folder",
                why: "no folder is configured, so NO BACKUP RUNS. There is deliberately no \
                      default: a backup tool that invents a destination is one whose backups \
                      are somewhere you did not choose, and you find out when you need them \
                      (SPEC-01 §2.1)."
                    .to_string(),
            });
        }
        if self.backups.cadence.is_some() {
            out.push(Refusal {
                key: "backups.cadence",
                why: CADENCE_RETIRED.to_string(),
            });
        } else if self.backups.schedule.is_none() {
            out.push(Refusal {
                key: "backups.schedule",
                why: "no schedule is configured, so NO BACKUP RUNS. There is deliberately no \
                      default: write the days and times backups should run, like schedule = \
                      [\"Tue 04:00\", \"Fri 04:00\"], or schedule = [] for manual only \
                      (SPEC-01 §3.1)."
                    .to_string(),
            });
        }
        for over in &self.backups.overrides {
            if over.cadence.is_some() {
                out.push(Refusal {
                    key: "backups.override.cadence",
                    why: format!("the override for {}: {CADENCE_RETIRED}", over.key()),
                });
            }
        }
        if self.tool.path.is_none() {
            out.push(Refusal {
                key: "tool.path",
                why: format!(
                    "no `cargo-rivet` is configured. This is not defaulted to the one on your \
                     PATH on purpose — that build may have no `backup` verb at all, and a \
                     program that invoked it would report `nothing to back up` for ever. \
                     `{}` installs one into Cargo's bin directory.",
                    crate::tool::INSTALL
                ),
            });
        }
        if self.fleet.server.is_none() {
            out.push(Refusal {
                key: "fleet.server",
                why: "no fleet server is configured, so the register cannot be read and this \
                      program does not know which machines exist (SPEC-01 §4)."
                    .to_string(),
            });
        }
        if self.fleet.identity.is_none() {
            out.push(Refusal {
                key: "fleet.identity",
                why: "no control-plane identity is configured. It is a DIFFERENT credential \
                      from the fleet authority and neither can do the other's job (SPEC-01 \
                      §4): the identity is what every machine is asked through — what it \
                      offers, its health, every backup — and the authority only reads the \
                      register. `cargo rivet setup` writes yours at ~/.rivet/identity/operator."
                    .to_string(),
            });
        }
        if self.backups.retention == 0 {
            out.push(Refusal {
                key: "backups.retention",
                why: "retention is 0, which would mean keeping no backups at all. If that is \
                      what you want, `exempt = true` says it without pretending to run."
                    .to_string(),
            });
        }
        // **An override's 0 is the same mistake, one level down** — and worse:
        // retention prunes right after a backup is placed and recorded, so a 0
        // would delete every backup of that application the moment it landed
        // while reporting each one a success.
        for over in &self.backups.overrides {
            if over.retention == Some(0) {
                out.push(Refusal {
                    key: "backups.override.retention",
                    why: format!(
                        "the override for {}: retention is 0, which would delete each backup \
                         the moment it is placed, after recording it as a success. If that is \
                         what you want, `exempt = true` says it without pretending to run.",
                        over.key()
                    ),
                });
            }
        }
        out
    }

    /// **Everything that stops the monitor capturing anything** — `SPEC-02` §2.
    ///
    /// Empty when there is no `[monitor]` table, because no table is no monitor
    /// rather than a broken one.
    ///
    /// **Kept apart from [`Config::refusals`] on purpose.** A monitor that is
    /// misconfigured must not stop a single backup, any more than a backup in
    /// progress may delay a capture (§3.1). So `backups.folder` missing is not a
    /// reason here, and a `[monitor]` mistake is not a reason there.
    ///
    /// What IS shared is what a capture needs to reach a machine at all: the
    /// tool, the register and the identity (§4).
    #[must_use]
    pub fn monitor_refusals(&self) -> Vec<Refusal> {
        let Some(monitor) = &self.monitor else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut refuse = |key: &'static str, why: String| out.push(Refusal { key, why });

        match monitor.folder() {
            None => refuse(
                "monitor.folder",
                "a [monitor] table is present and names no folder, so NOTHING IS CAPTURED. \
                 There is deliberately no default: monitoring is high-volume, and every \
                 platform default on Windows is on the system drive (SPEC-02 §2.1, SPEC-01 \
                 §2.1)."
                    .to_string(),
            ),
            Some(folder) => {
                if let Some(backups) = self.backups.folder.as_deref() {
                    let backups = expand_tilde(backups);
                    if same_or_inside(&folder, &backups) {
                        refuse(
                            "monitor.folder",
                            format!(
                                "{} is backups.folder or inside it. Monitoring is \
                                 high-volume and disposable and backups are precious and \
                                 small; they do not share a folder (SPEC-02 §2.1, ADR-002 §2).",
                                folder.display()
                            ),
                        );
                    } else if same_or_inside(&backups, &folder) {
                        refuse(
                            "monitor.folder",
                            format!(
                                "backups.folder ({}) is inside monitor.folder ({}). The reverse \
                                 is refused too: retention in one must never be able to reach \
                                 the other (SPEC-02 §2.1).",
                                backups.display(),
                                folder.display()
                            ),
                        );
                    }
                }
            }
        }

        if monitor.timeout_seconds == 0 {
            refuse(
                "monitor.timeout-seconds",
                "0 seconds would kill every capture before it could answer, and record a \
                 machine that is working as one that is not (SPEC-02 §3.3)."
                    .to_string(),
            );
        }

        // What a capture needs to reach a machine at all (§4). Named here as
        // well as in `refusals`, because a monitor-only configuration has no
        // other place that would say so.
        if self.tool.path.is_none() {
            refuse(
                "tool.path",
                format!(
                    "no `cargo-rivet` is configured, and every capture is a call to it (SPEC-02 \
                     §4). `{}` installs one into Cargo's bin directory.",
                    crate::tool::INSTALL
                ),
            );
        }
        if self.fleet.server.is_none() {
            refuse(
                "fleet.server",
                "no fleet server is configured, so the register cannot be read and no \
                 monitored machine can be found (SPEC-02 §2.2)."
                    .to_string(),
            );
        }
        if self.fleet.identity.is_none() {
            refuse(
                "fleet.identity",
                "no control-plane identity is configured, and every capture speaks the \
                 control plane (SPEC-02 §4)."
                    .to_string(),
            );
        }

        let mut seen: Vec<&str> = Vec::new();
        for machine in &monitor.machines {
            if let Err(why) = crate::register::usable_name(&machine.name) {
                refuse(
                    "monitor.machine.name",
                    format!(
                        "a [[monitor.machine]] is named by its register name and nothing else \
                         (SPEC-02 §2.2), and that name becomes a directory under monitor.folder: \
                         {why}"
                    ),
                );
                continue;
            }
            if seen.contains(&machine.name.as_str()) {
                refuse(
                    "monitor.machine.name",
                    format!(
                        "two [[monitor.machine]] entries name {:?}. Only the first would \
                         apply, and the second is a setting you believe is in force and is not \
                         (SPEC-02 §2.2).",
                        machine.name
                    ),
                );
                continue;
            }
            seen.push(&machine.name);

            for signal in SIGNALS {
                let Some(table) = machine.table(signal) else {
                    continue;
                };
                if let Some(every) = table.every_seconds
                    && every < EVERY_SECONDS_FLOOR
                {
                    refuse(
                        "monitor.machine.every-seconds",
                        format!(
                            "{}/{signal}: every-seconds = {every} is below the floor of \
                             {EVERY_SECONDS_FLOOR}. Every capture opens a control-plane \
                             connection and every connection writes records into the board's \
                             own journal, so polling faster shortens how far back that journal \
                             reaches (SPEC-02 §2.3, ADR-002 §5).",
                            machine.name
                        ),
                    );
                }
                // §2.3: `keep-days` keeps dated files and `keep-frames` keeps
                // stored frames, and neither is accepted on the other's signal.
                // A key that was quietly ignored is `SPEC-01` §2's failure: a
                // retention the operator believes is in force and is not.
                if table.keep_days.is_some() && signal.takes_keep_frames() {
                    refuse(
                        "monitor.machine.keep-days",
                        format!(
                            "{}/{signal}: `keep-days` is not how stored frames are kept. A frame \
                             is 6.2 MB and a day of them is gigabytes, so the screenshot keeps a \
                             COUNT: use `keep-frames` (default {}), which keeps that many newest \
                             frames across day directories (SPEC-02 §2.3, §5.4).",
                            machine.name,
                            Signal::Screenshot.default_keep_frames()
                        ),
                    );
                }
                if table.keep_frames.is_some() && !signal.takes_keep_frames() {
                    refuse(
                        "monitor.machine.keep-frames",
                        format!(
                            "{}/{signal}: `keep-frames` keeps stored screenshot frames and nothing \
                             else. On `{signal}` it would be a retention you believe is in force \
                             and is not; that signal's files are kept by `keep-days` (SPEC-02 \
                             §2.3, §5.4).",
                            machine.name
                        ),
                    );
                }
                if let Some(keep) = table.keep_days
                    && keep < 1
                    && !signal.takes_keep_frames()
                {
                    refuse(
                        "monitor.machine.keep-days",
                        format!(
                            "{}/{signal}: keep-days = {keep} would delete today's file while \
                             it is still being written. The least is 1 (SPEC-02 §2.3).",
                            machine.name
                        ),
                    );
                }
                if let Some(keep) = table.keep_frames
                    && keep < 1
                    && signal.takes_keep_frames()
                {
                    refuse(
                        "monitor.machine.keep-frames",
                        format!(
                            "{}/{signal}: keep-frames = {keep} would delete every frame as it was \
                             stored, and §6.4 would then compare each new frame against nothing. \
                             The least is 1; to stop capturing frames at all, remove the \
                             [monitor.machine.screenshot] table (SPEC-02 §2.3).",
                            machine.name
                        ),
                    );
                }
                if table.services.is_some() && !signal.takes_services() {
                    refuse(
                        "monitor.machine.services",
                        format!(
                            "{}/{signal}: `services` narrows `status` and `log` and nothing \
                             else. On `{signal}` it would be a filter you believe is in force \
                             and is not (SPEC-02 §2.4).",
                            machine.name
                        ),
                    );
                }
            }

            // §6.4 cannot be judged unless the same frame is FETCHED twice
            // inside its window: the condition is measured across separate
            // observations of one holder and one timestamp.
            if let Some(shot) = machine.table(Signal::Screenshot) {
                let every = shot
                    .every_seconds
                    .unwrap_or_else(|| Signal::Screenshot.default_every_seconds());
                let frozen = machine.frozen_minutes.unwrap_or(monitor.frozen_minutes);
                if frozen > 0 && frozen.saturating_mul(60) <= every.saturating_mul(2) {
                    refuse(
                        "monitor.machine.frozen-minutes",
                        format!(
                            "{}: frozen-minutes = {frozen} is {} s, and it must EXCEED twice the \
                             screenshot cadence, which is {} s at every-seconds = {every}. \
                             `frozen/` is measured across separate fetches of one frame, so a \
                             window that is not longer than two cadences would call a screen \
                             frozen on the strength of a single repeated frame — or, narrower \
                             still, never be judged at all. Raise frozen-minutes above {} s, or \
                             shorten the screenshot's every-seconds (SPEC-02 §2.3, §6.4).",
                            machine.name,
                            frozen.saturating_mul(60),
                            every.saturating_mul(2),
                            every.saturating_mul(2)
                        ),
                    );
                }
            }
        }
        out
    }

    /// The authority directory, defaulted.
    #[must_use]
    pub fn authority(&self) -> PathBuf {
        self.fleet
            .authority
            .as_deref()
            .map_or_else(default_authority, expand_tilde)
    }

    /// The DNS name TLS verifies a machine against, from its register label.
    ///
    /// A label already carrying a dot is a full name and is left alone.
    #[must_use]
    pub fn dns_name(&self, register_name: &str) -> String {
        if register_name.contains('.') || self.fleet.host_suffix.is_empty() {
            register_name.to_string()
        } else {
            format!("{register_name}.{}", self.fleet.host_suffix)
        }
    }

    /// The override that applies to one machine and application, if any.
    #[must_use]
    pub fn override_for(&self, host: &str, application: &str) -> Option<&Override> {
        self.backups
            .overrides
            .iter()
            .find(|o| o.host == host && o.application == application)
    }
}

/// What one application's settings resolve to once its override is applied.
#[derive(Debug, Clone)]
pub struct Resolved {
    /// Empty is manual only.
    pub schedule: Schedule,
    pub retention: u32,
    pub folder: PathBuf,
    pub exempt: bool,
}

impl Config {
    /// Resolve one application's settings.
    ///
    /// # Errors
    ///
    /// Only when `backups.folder` is absent and no override supplies one, which
    /// [`Config::refusals`] has already reported.
    pub fn resolve(&self, host: &str, application: &str) -> Result<Resolved, String> {
        let over = self.override_for(host, application);
        let folder = over
            .and_then(|o| o.folder.clone())
            .or_else(|| self.backups.folder.clone())
            .ok_or_else(|| format!("{host}/{application}: no folder is configured"))?;
        Ok(Resolved {
            // An absent `[backups] schedule` is a refusal, so nothing reaches
            // here with it missing; manual only is the answer that cannot
            // fetch by accident if something ever does.
            schedule: over
                .and_then(|o| o.schedule.clone())
                .or_else(|| self.backups.schedule.clone())
                .unwrap_or_default(),
            retention: over
                .and_then(|o| o.retention)
                .unwrap_or(self.backups.retention),
            folder: expand_tilde(&folder),
            exempt: over.is_some_and(|o| o.exempt),
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn parse(text: &str) -> Result<Config, toml::de::Error> {
        toml::from_str(text)
    }

    #[test]
    fn the_defaults_are_no_schedule_and_three() {
        let cfg = parse("").expect("an empty file is a valid file");
        assert_eq!(cfg.backups.schedule, None);
        assert_eq!(cfg.backups.retention, 3);
        assert_eq!(cfg.fleet.poll_seconds, 300);
    }

    /// **The log rolls by size whether or not `[logging]` is written** — a
    /// table that sets only `enabled` or `level` must not bring back the
    /// per-launch rotation a long-running tray never rolls; an explicit choice
    /// is kept.
    #[test]
    fn the_log_rolls_by_size_unless_the_file_says_otherwise() {
        use ignition::logging::Rotation;
        let none = parse("").expect("valid");
        assert_eq!(none.logging.rotation, Rotation::Size);
        let table = parse("[logging]\nenabled = true\nlevel = \"info\"\n").expect("valid");
        assert_eq!(table.logging.rotation, Rotation::Size);
        assert!(table.logging.enabled);
        let chosen = parse("[logging]\nrotation = \"run\"\n").expect("valid");
        assert_eq!(chosen.logging.rotation, Rotation::Run);
    }

    /// **§3.1.** No schedule, no backups — and `[]` is a schedule, manual only.
    #[test]
    fn a_missing_schedule_refuses_and_an_empty_one_does_not() {
        let keys = |text: &str| -> Vec<&'static str> {
            parse(text).expect("valid").refusals().iter().map(|r| r.key).collect()
        };
        assert!(keys("").contains(&"backups.schedule"));
        assert!(!keys("[backups]\nschedule = []\n").contains(&"backups.schedule"));
    }

    /// **§2.2.** The retired key is refused by NAME, with what replaced it —
    /// not as a typo, and not silently ignored.
    #[test]
    fn a_retired_cadence_is_refused_saying_what_replaced_it() {
        let cfg = parse(
            "[backups]\ncadence = \"weekly\"\n\n[[backups.override]]\nhost = \"h\"\n\
             application = \"a\"\ncadence = \"daily\"\n",
        )
        .expect("it still loads, so the tray can say why");
        let refusals = cfg.refusals();
        let cadence = refusals
            .iter()
            .find(|r| r.key == "backups.cadence")
            .expect("refused");
        assert!(cadence.why.contains("replaced by `schedule`"), "{cadence}");
        assert!(refusals.iter().any(|r| r.key == "backups.override.cadence"));
    }

    /// **An override's `retention = 0` is refused** — retention prunes right
    /// after a backup is placed and recorded, so a 0 would delete every backup
    /// of that application while reporting each one a success.
    #[test]
    fn an_override_with_retention_zero_is_refused() {
        let cfg = parse(
            "[backups]\nschedule = []\n\n[[backups.override]]\nhost = \"h\"\n\
             application = \"a\"\nretention = 0\n",
        )
        .expect("it loads, so the tray can say why");
        let refused = cfg
            .refusals()
            .into_iter()
            .find(|r| r.key == "backups.override.retention")
            .expect("refused");
        assert!(refused.why.contains("retention is 0"), "{refused}");
    }

    /// **§2.1.** No folder, no backups, and the icon says so.
    #[test]
    fn a_file_with_no_folder_refuses_by_name() {
        let cfg = parse("").expect("valid");
        let keys: Vec<&str> = cfg.refusals().iter().map(|r| r.key).collect();
        assert!(keys.contains(&"backups.folder"), "{keys:?}");
        assert!(keys.contains(&"tool.path"), "{keys:?}");
        assert!(keys.contains(&"fleet.server"), "{keys:?}");
        assert!(keys.contains(&"fleet.identity"), "{keys:?}");
    }

    /// A mistyped key is a setting the operator believes is in force and is
    /// not, so it is refused rather than ignored.
    #[test]
    fn a_key_this_build_does_not_define_is_refused() {
        let refused = parse("[backups]\nfoldr = \"D:/x\"\n").expect_err("a typo");
        assert!(refused.to_string().contains("foldr"), "{refused}");
    }

    #[test]
    fn an_override_inherits_everything_it_does_not_set() {
        let cfg = parse(
            "[backups]\nfolder = \"D:/b\"\nschedule = [\"Tue 04:00\", \"Fri 04:00\"]\n\
             retention = 5\n\n\
             [[backups.override]]\nhost = \"rivet-01\"\napplication = \"photos-rivet\"\n\
             schedule = [\"Mon 21:00\"]\n\n\
             [[backups.override]]\nhost = \"rivet-01\"\napplication = \"aura-rivet\"\n\
             schedule = []\n",
        )
        .expect("valid");
        let r = cfg.resolve("rivet-01", "photos-rivet").expect("resolves");
        assert_eq!(r.schedule.to_string(), "Mon 21:00");
        assert_eq!(
            r.retention, 5,
            "retention was not overridden, so it inherits"
        );
        assert!(!r.exempt);
        let other = cfg.resolve("rivet-01", "tempmon-rivet").expect("resolves");
        assert_eq!(other.schedule.to_string(), "Tue 04:00, Fri 04:00");
        let manual = cfg.resolve("rivet-01", "aura-rivet").expect("resolves");
        assert!(manual.schedule.manual(), "an override's [] is manual only, not inherit");
    }

    // -- [monitor], SPEC-02 §2 ---------------------------------------------

    /// The fleet and tool keys a capture needs, so a test's refusals are only
    /// the ones it is about.
    const REACHABLE: &str = "[backups]\nfolder = \"D:/Backups\"\nschedule = []\n\
         [fleet]\nserver = \"rivet-01.local:4443\"\nidentity = \"~/.rivet/identity/operator\"\n\
         [tool]\npath = \"D:/cargo-rivet.exe\"\n";

    fn monitor(text: &str) -> Config {
        parse(&format!("{REACHABLE}{text}")).expect("a valid monitor configuration")
    }

    fn monitor_keys(cfg: &Config) -> Vec<&'static str> {
        cfg.monitor_refusals().iter().map(|r| r.key).collect()
    }

    /// **No `[monitor]` table is no monitor**, not a broken one.
    #[test]
    fn no_monitor_table_is_no_monitor_and_no_refusal() {
        let cfg = parse(REACHABLE).expect("valid");
        assert!(cfg.monitor.is_none());
        assert!(cfg.monitor_refusals().is_empty());
    }

    /// **§3.1's independence, in the configuration**: a broken `[monitor]`
    /// stops no backup, and a backup configuration missing its folder stops no
    /// capture.
    #[test]
    fn monitor_and_backup_refusals_do_not_stop_each_other() {
        let cfg = monitor("[monitor]\n");
        assert!(cfg.refusals().is_empty(), "{:?}", cfg.refusals());
        assert!(!cfg.monitor_refusals().is_empty());

        let no_backups = parse(
            "[fleet]\nserver = \"s:4443\"\nidentity = \"i\"\n[tool]\npath = \"t\"\n\
             [monitor]\nfolder = \"D:/Monitor\"\n",
        )
        .expect("valid");
        assert!(no_backups.monitor_refusals().is_empty(), "{:?}", no_backups.monitor_refusals());
        assert!(no_backups.refusals().iter().any(|r| r.key == "backups.folder"));
    }

    /// **§2.1.** A `[monitor]` without a folder captures nothing and says so.
    #[test]
    fn a_monitor_with_no_folder_is_refused_by_name() {
        let cfg = monitor("[monitor]\n[[monitor.machine]]\nname = \"rivet-01\"\n[monitor.machine.status]\n");
        assert_eq!(monitor_keys(&cfg), vec!["monitor.folder"]);
    }

    /// **§2.1.** Not the backups folder, not inside it.
    #[test]
    fn a_monitor_folder_equal_to_or_inside_the_backups_folder_is_refused() {
        for folder in ["D:/Backups", "D:/Backups/monitor", "d:\\backups\\x\\..\\monitor"] {
            let cfg = monitor(&format!("[monitor]\nfolder = {folder:?}\n"));
            let refusals = cfg.monitor_refusals();
            assert_eq!(refusals.len(), 1, "{folder}: {refusals:?}");
            assert_eq!(refusals[0].key, "monitor.folder");
            assert!(refusals[0].why.contains("inside it"), "{}", refusals[0].why);
        }
        assert!(monitor("[monitor]\nfolder = \"D:/BackupsMonitor\"\n")
            .monitor_refusals()
            .is_empty(), "a sibling whose name merely STARTS the same is not inside");
    }

    /// **§2.1, the reverse.** The backups folder inside the monitor folder.
    #[test]
    fn a_backups_folder_inside_the_monitor_folder_is_refused() {
        let cfg = monitor("[monitor]\nfolder = \"D:/\"\n");
        let refusals = cfg.monitor_refusals();
        assert_eq!(refusals.len(), 1, "{refusals:?}");
        assert!(refusals[0].why.contains("is inside monitor.folder"), "{}", refusals[0].why);
    }

    /// **§2.3.** Below 30 is refused, naming the floor and why; 30 is not.
    #[test]
    fn every_seconds_below_the_floor_is_refused_naming_it() {
        let cfg = monitor(
            "[monitor]\nfolder = \"D:/Monitor\"\n[[monitor.machine]]\nname = \"rivet-01\"\n\
             [monitor.machine.thermal]\nevery-seconds = 29\n[monitor.machine.status]\nevery-seconds = 30\n",
        );
        let refusals = cfg.monitor_refusals();
        assert_eq!(refusals.len(), 1, "{refusals:?}");
        assert_eq!(refusals[0].key, "monitor.machine.every-seconds");
        assert!(refusals[0].why.contains("rivet-01/thermal"), "{}", refusals[0].why);
        assert!(refusals[0].why.contains("floor of 30"), "{}", refusals[0].why);
        assert!(refusals[0].why.contains("journal"), "{}", refusals[0].why);
    }

    /// **§2.3.** `keep-days` below 1 is refused.
    #[test]
    fn keep_days_below_one_is_refused() {
        let cfg = monitor(
            "[monitor]\nfolder = \"D:/Monitor\"\n[[monitor.machine]]\nname = \"rivet-01\"\n\
             [monitor.machine.storage]\nkeep-days = 0\n",
        );
        assert_eq!(monitor_keys(&cfg), vec!["monitor.machine.keep-days"]);
    }

    /// **§2.3: `keep-days` on a `screenshot` table is refused BY NAME**,
    /// pointing at `keep-frames` — a key that was quietly ignored is a
    /// retention the operator believes is in force and is not.
    #[test]
    fn keep_days_on_a_screenshot_table_is_refused_naming_keep_frames() {
        let cfg = monitor(
            "[monitor]\nfolder = \"D:/Monitor\"\n[[monitor.machine]]\nname = \"rivet-01\"\n\
             [monitor.machine.screenshot]\nkeep-days = 3\n",
        );
        let refusals = cfg.monitor_refusals();
        assert_eq!(refusals.len(), 1, "{refusals:?}");
        assert_eq!(refusals[0].key, "monitor.machine.keep-days");
        assert!(refusals[0].why.contains("keep-frames"), "{}", refusals[0].why);
        assert!(refusals[0].why.contains("rivet-01/screenshot"), "{}", refusals[0].why);
    }

    /// **§2.3, the reverse**: `keep-frames` belongs to `screenshot` alone, and
    /// below 1 it is refused on the signal that does take it.
    #[test]
    fn keep_frames_is_refused_on_another_signal_and_below_one() {
        let elsewhere = monitor(
            "[monitor]\nfolder = \"D:/Monitor\"\n[[monitor.machine]]\nname = \"rivet-01\"\n\
             [monitor.machine.status]\nkeep-frames = 3\n",
        );
        let refusals = elsewhere.monitor_refusals();
        assert_eq!(refusals.len(), 1, "{refusals:?}");
        assert_eq!(refusals[0].key, "monitor.machine.keep-frames");
        assert!(refusals[0].why.contains("keep-days"), "{}", refusals[0].why);

        let none = monitor(
            "[monitor]\nfolder = \"D:/Monitor\"\n[[monitor.machine]]\nname = \"rivet-01\"\n\
             [monitor.machine.screenshot]\nkeep-frames = 0\n",
        );
        assert_eq!(monitor_keys(&none), vec!["monitor.machine.keep-frames"]);
    }

    /// **§2.3 and §6.4: `frozen-minutes` must EXCEED twice the screenshot
    /// cadence**, and a configuration that breaks it is refused naming both
    /// numbers. The defaults — 15 minutes against 300 s — satisfy it.
    #[test]
    fn frozen_minutes_must_exceed_twice_the_screenshot_cadence() {
        let default = monitor(
            "[monitor]\nfolder = \"D:/Monitor\"\n[[monitor.machine]]\nname = \"rivet-01\"\n\
             [monitor.machine.screenshot]\n",
        );
        assert!(default.monitor_refusals().is_empty(), "{:?}", default.monitor_refusals());

        // Exactly twice is not MORE than twice.
        let equal = monitor(
            "[monitor]\nfolder = \"D:/Monitor\"\nfrozen-minutes = 10\n\
             [[monitor.machine]]\nname = \"rivet-01\"\n[monitor.machine.screenshot]\n",
        );
        let refusals = equal.monitor_refusals();
        assert_eq!(refusals.len(), 1, "{refusals:?}");
        assert_eq!(refusals[0].key, "monitor.machine.frozen-minutes");
        assert!(refusals[0].why.contains("600 s"), "both numbers: {}", refusals[0].why);
        assert!(refusals[0].why.contains("every-seconds = 300"), "{}", refusals[0].why);

        // A machine may override its way back into the rule, either side of it.
        let overridden = monitor(
            "[monitor]\nfolder = \"D:/Monitor\"\nfrozen-minutes = 10\n\
             [[monitor.machine]]\nname = \"rivet-01\"\n[monitor.machine.screenshot]\nevery-seconds = 30\n",
        );
        assert!(overridden.monitor_refusals().is_empty(), "{:?}", overridden.monitor_refusals());

        // And 0 switches the condition off, which cannot break a rule about it.
        let off = monitor(
            "[monitor]\nfolder = \"D:/Monitor\"\nfrozen-minutes = 0\n\
             [[monitor.machine]]\nname = \"rivet-01\"\n[monitor.machine.screenshot]\n",
        );
        assert!(off.monitor_refusals().is_empty(), "{:?}", off.monitor_refusals());
    }

    /// **§2.3's defaults, as the maintainer ruled them on 2026-09-15.**
    #[test]
    fn the_rulings_of_2026_09_15_are_the_defaults() {
        assert_eq!(Signal::Screenshot.default_every_seconds(), 300, "five minutes");
        assert_eq!(Signal::Screenshot.default_keep_frames(), 3, "the last three frames");
        assert_eq!(default_frozen_minutes(), 15);
        assert_eq!(default_card_stalled_minutes(), 70, "an hour's write plus room for a late one");
        for signal in SIGNALS {
            assert_eq!(signal.default_keep_days(), 14, "{signal}: every dated file keeps 14 days");
            assert_eq!(signal.takes_keep_frames(), signal == Signal::Screenshot, "{signal}");
        }
        // The default pair satisfies §6.4's rule with room to spare.
        assert!(
            default_frozen_minutes() * 60 > Signal::Screenshot.default_every_seconds() * 2,
            "the defaults must not refuse themselves"
        );
    }

    /// **The process table is the ninth signal, and it is the one that catches
    /// a wedged supervisor without anybody awake** — added 2026-09-16.
    ///
    /// Rivet's `get process` is answered by a spawned `rivet` reading `/proc`,
    /// with no supervisor involved — which is why it kept answering for eleven
    /// minutes after `rivet-01`'s supervisor stopped on 2026-09-16 while
    /// `get status`, which asks the supervisor, timed out every minute. The row
    /// for `rivet-sysd` carries `user_millis`, `system_millis` and, since that
    /// day, `wchan`: two captures a minute apart are frozen counters and a
    /// kernel function, which is the whole diagnosis, recorded off the machine.
    ///
    /// **A minute, like `status`.** The window it has to catch is the eleven
    /// minutes between the supervisor stopping and `rivet-mcpd` following it
    /// down; an hourly cadence would miss that entirely.
    #[test]
    fn the_process_table_is_captured_every_minute_and_takes_no_narrowing() {
        assert!(SIGNALS.contains(&Signal::Process));
        assert_eq!(Signal::Process.name(), "process", "the get subject, the directory and the table are one word");
        assert_eq!(
            Signal::Process.default_every_seconds(),
            60,
            "the window between a supervisor stopping and the control plane following it \
             was eleven minutes; an hourly signal would have been inside it twice at best"
        );
        assert!(
            !Signal::Process.takes_services(),
            "`services` narrows what a machine reports about ITS OWN services; the process \
             table is the kernel's answer and has no such list to narrow"
        );
        assert!(!Signal::Process.takes_keep_frames());
    }

    /// The table parses, and an unknown key inside it is still refused.
    #[test]
    fn a_process_table_is_configured_like_every_other_signal() {
        let config = parse(&format!(
            "{REACHABLE}[monitor]\nfolder = \"D:/M\"\n[[monitor.machine]]\nname = \"a\"\n\
             [monitor.machine.process]\nevery-seconds = 120\n"
        ))
        .expect("a process table is ordinary configuration");
        let machine = &config.monitor.expect("a monitor").machines[0];
        let table = machine.table(Signal::Process).expect("the table is present");
        assert_eq!(table.every_seconds, Some(120));
        assert!(
            machine.table(Signal::Status).is_none(),
            "a table nobody wrote is absent, and absent means not captured"
        );
    }

    /// **`SPEC-01` §2 still applies**: a key this build does not define is
    /// refused, in `[monitor]`, in a machine, and in a signal's table.
    #[test]
    fn an_unknown_monitor_key_is_refused() {
        for text in [
            "[monitor]\nfolder = \"D:/M\"\nfoldr = \"x\"\n",
            "[monitor]\nfolder = \"D:/M\"\n[[monitor.machine]]\nname = \"a\"\naddress = \"192.168.50.10\"\n",
            "[monitor]\nfolder = \"D:/M\"\n[[monitor.machine]]\nname = \"a\"\n[monitor.machine.status]\nevery-second = 60\n",
            "[monitor]\nfolder = \"D:/M\"\n[[monitor.machine]]\nname = \"a\"\n[monitor.machine.cpu]\n",
        ] {
            let refused = parse(&format!("{REACHABLE}{text}")).expect_err(text);
            assert!(refused.to_string().contains("unknown field"), "{text}: {refused}");
        }
    }

    /// **§2.4.** `services` on anything but `status` and `log` is refused.
    #[test]
    fn services_on_a_signal_other_than_status_or_log_is_refused() {
        let cfg = monitor(
            "[monitor]\nfolder = \"D:/Monitor\"\n[[monitor.machine]]\nname = \"rivet-01\"\n\
             [monitor.machine.status]\nservices = [\"aura-renderer\"]\n\
             [monitor.machine.log]\nservices = [\"aura-renderer\"]\n\
             [monitor.machine.thermal]\nservices = [\"aura-renderer\"]\n",
        );
        let refusals = cfg.monitor_refusals();
        assert_eq!(refusals.len(), 1, "{refusals:?}");
        assert_eq!(refusals[0].key, "monitor.machine.services");
        assert!(refusals[0].why.contains("rivet-01/thermal"), "{}", refusals[0].why);
    }

    /// **§2.2.** Two entries for one machine; only the first would apply.
    #[test]
    fn two_entries_for_one_machine_are_refused() {
        let cfg = monitor(
            "[monitor]\nfolder = \"D:/Monitor\"\n\
             [[monitor.machine]]\nname = \"rivet-01\"\n[monitor.machine.status]\n\
             [[monitor.machine]]\nname = \"rivet-01\"\n[monitor.machine.thermal]\n",
        );
        let refusals = cfg.monitor_refusals();
        assert_eq!(refusals.len(), 1, "{refusals:?}");
        assert_eq!(refusals[0].key, "monitor.machine.name");
        assert!(refusals[0].why.contains("two [[monitor.machine]]"), "{}", refusals[0].why);
    }

    /// **§2.2 and `SPEC-01` §4.4.** A machine's name becomes a directory under
    /// the monitor folder, so a name that could leave it is refused.
    #[test]
    fn a_monitor_machine_name_that_could_leave_the_folder_is_refused() {
        for bad in ["..", "C:\\x", "a/b", "nul"] {
            let cfg = monitor(&format!(
                "[monitor]\nfolder = \"D:/Monitor\"\n\
                 [[monitor.machine]]\nname = {bad:?}\n[monitor.machine.status]\n"
            ));
            let refusals = cfg.monitor_refusals();
            assert_eq!(refusals.len(), 1, "{bad}: {refusals:?}");
            assert_eq!(refusals[0].key, "monitor.machine.name");
            assert!(refusals[0].why.contains("cannot be used as a folder name"), "{}", refusals[0].why);
        }
    }

    /// **`SPEC-01` §2.6: both switches are off unless the file says so.** A
    /// monitoring block with no `enabled` is parsed, checked and kept — and
    /// resolves to nothing, which is what stops its worker.
    #[test]
    fn both_switches_are_off_by_default_per_machine() {
        let cfg = monitor(
            "[monitor]\nfolder = \"D:/Monitor\"\n\
             [[monitor.machine]]\nname = \"rivet-01\"\n[monitor.machine.status]\n\
             [[monitor.machine]]\nname = \"dev-01\"\nenabled = true\n[monitor.machine.status]\n\
             [[backups.machine]]\nname = \"rivet-01\"\nenabled = true\n\
             [[backups.machine]]\nname = \"dev-01\"\n",
        );
        assert!(cfg.refusals().is_empty(), "{:?}", cfg.refusals());
        assert!(!cfg.monitoring_on("rivet-01"), "no `enabled` is off");
        assert!(cfg.monitoring_on("dev-01"));
        assert!(cfg.backups_on("rivet-01"));
        assert!(!cfg.backups_on("dev-01"), "no `enabled` is off");
        assert!(!cfg.backups_on("absent"), "a machine the file never names is off");
        let resolved = cfg.monitor.as_ref().expect("a monitor").resolve();
        assert_eq!(
            resolved.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
            ["dev-01"],
            "a switched-off machine is not resolved, so it has no worker"
        );
    }

    /// **`SPEC-01` §2.6.** Two backup switches for one machine could disagree,
    /// and only one of them would be obeyed.
    #[test]
    fn two_backup_switches_for_one_machine_are_refused() {
        let cfg = monitor(
            "[[backups.machine]]\nname = \"rivet-01\"\nenabled = true\n\
             [[backups.machine]]\nname = \"rivet-01\"\n",
        );
        let refusals = cfg.refusals();
        assert!(
            refusals.iter().any(|r| r.key == "backups.machine"),
            "{refusals:?}"
        );
    }

    /// **§2.3 and §2.5.** An empty table takes both defaults; a machine's
    /// condition keys override `[monitor]`'s, and the rest are inherited.
    #[test]
    fn defaults_and_per_machine_condition_overrides_resolve() {
        let cfg = monitor(
            "[monitor]\nfolder = \"D:/Monitor\"\nlog-stalled-minutes = 12\n\
             [[monitor.machine]]\nname = \"rivet-01\"\nenabled = true\nlog-stalled-minutes = 20\n\
             power = false\ncard-stalled-minutes = 90\n\
             [monitor.machine.status]\nservices = [\"aura-renderer\"]\n\
             [monitor.machine.storage]\n\
             [monitor.machine.slot]\nevery-seconds = 45\nkeep-days = 2\n\
             [[monitor.machine]]\nname = \"board\"\nenabled = true\n[monitor.machine.screenshot]\n",
        );
        assert!(cfg.monitor_refusals().is_empty(), "{:?}", cfg.monitor_refusals());
        let m = cfg.monitor.as_ref().expect("a monitor");
        assert_eq!(m.timeout_seconds, 20);
        let resolved = m.resolve();

        let first = &resolved[0];
        assert_eq!(
            first.conditions,
            Conditions {
                reach_misses: 3,
                log_stalled_minutes: 20,
                frozen_minutes: 15,
                card_stalled_minutes: 90,
                power: false
            }
        );
        let signals: Vec<(Signal, u64, u64)> = first
            .signals
            .iter()
            .map(|s| (s.signal, s.every_seconds, s.keep_days))
            .collect();
        assert_eq!(
            signals,
            vec![
                (Signal::Status, 60, 14),
                (Signal::Storage, 3_600, 14),
                (Signal::Slot, 45, 2)
            ]
        );
        assert_eq!(first.signals[0].services.as_deref(), Some(&["aura-renderer".to_string()][..]));

        let second = &resolved[1];
        assert_eq!(
            second.conditions,
            Conditions {
                reach_misses: 3,
                log_stalled_minutes: 12,
                frozen_minutes: 15,
                card_stalled_minutes: 70,
                power: true
            }
        );
        assert_eq!(second.signals[0].signal, Signal::Screenshot);
        assert_eq!(second.signals[0].every_seconds, 300, "§2.3: five minutes");
        assert_eq!(second.signals[0].keep_frames, Some(3), "§2.3: the newest three frames");
        assert_eq!(second.signals[0].keep_days, 14, "the INDEX is dated like every other file");
        assert!(
            first.signals.iter().all(|s| s.keep_frames.is_none()),
            "keep-frames is the screenshot's alone"
        );
    }

    #[test]
    fn exempt_is_carried_through() {
        let cfg = parse(
            "[backups]\nfolder = \"D:/b\"\n\n[[backups.override]]\nhost = \"h\"\n\
             application = \"a\"\nexempt = true\n",
        )
        .expect("valid");
        assert!(cfg.resolve("h", "a").expect("resolves").exempt);
    }
}
