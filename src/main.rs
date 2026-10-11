//! **Rivet Monitor** — a Windows 11 notification-area application that operates
//! a Rivet fleet on its owner's behalf.
//!
//! `ADR-001`: it is deliberately outside Rivet. Rivet defines what an
//! application publishes and `cargo rivet backup` reads it; this program makes
//! the operator's decisions — when, how often, how many to keep, and where.
//!
//! # It has a headless surface, and that is not a convenience
//!
//! A tray icon cannot be asserted against. Every decision this program makes is
//! reachable from the command line — `check`, `register`, `run-once`,
//! `status` — so that what it does can be proved by running it rather than by
//! looking at a picture of a menu.
//!
//! ```text
//! rivet-monitor                 the tray icon (default)
//! rivet-monitor check           the configuration, the tool, the register, the overrides
//! rivet-monitor register        what the fleet server says the fleet is
//! rivet-monitor run-once        one scheduled pass, fetching what is due
//! rivet-monitor back-up-now     one pass that fetches everything, whatever its schedule
//! rivet-monitor status          what has been backed up and when
//! rivet-monitor monitor         the monitor alone, in the foreground (SPEC-02 §3.6)
//! rivet-monitor --config PATH   any of the above against another file
//! ```

// **A console subsystem binary, deliberately.** The commands above are how this
// program is asserted against, and a `windows` subsystem binary prints nothing
// to a terminal that ran it. The tray lets go of a console it allocated for
// itself instead — `tray::detach_own_console`.

mod alarm;
mod armada;
mod autostart;
mod capture;
mod clock;
mod conditions;
mod config;
mod icon;
mod ledger;
mod monitor;
mod notify;
mod problems;
mod proxy;
mod register;
mod schedule;
mod shell;
mod store;
mod switches;
mod timetable;
mod tls;
mod tool;
#[cfg(windows)]
mod tray;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::Config;
use crate::ledger::Ledger;

/// **The commands this program owns**, hung on the surface Ignition composes.
///
/// Ignition's CLI surface owns *help*, *version* and *update*, each
/// independently spelled; the consumer composes its own arguments alongside and
/// receives an [`Outcome::Passthrough`] for anything Ignition did not handle.
///
/// **`update` is `Surface::Off`, and that is the one exemption.** The
/// maintainer's ruling is that Rivet Monitor follows Ignition's configuration
/// and logging patterns and does *not* take its self-update: this program is
/// built and placed by hand from a working tree, and a binary that could
/// replace itself from a tag would be a second answer to *which build is
/// running* while a session is mid-investigation.
fn surface() -> ignition::cli::Cli {
    ignition::cli::Cli::new("rivet-monitor")
        .version(env!("CARGO_PKG_VERSION"))
        .about(
            "Operates a Rivet fleet's backups and health from the notification area. With no \
             command, it starts the tray.",
        )
        // Both spellings of each, which is what every sibling tool offers.
        .help_surface(ignition::cli::Surface::Both)
        .version_surface(ignition::cli::Surface::Both)
}

/// The parser: Ignition's surface plus this program's own verbs.
fn parser() -> clap::Command {
    surface()
        .command()
        .subcommand_required(false)
        .arg(
            clap::Arg::new("config")
                .long("config")
                .value_name("PATH")
                .global(true)
                .help("use this configuration file instead of the platform one"),
        )
        .subcommand(clap::Command::new("check").about(
            "read the configuration, prove the cargo-rivet it points at, read the register, \
             and report every override that matches nothing. Fetches NOTHING",
        ))
        .subcommand(
            clap::Command::new("register")
                .about("print the fleet register as the fleet server gives it"),
        )
        .subcommand(clap::Command::new("run-once").about(
            "one scheduled pass: fetch everything that is due, place it, and apply retention",
        ))
        .subcommand(clap::Command::new("back-up-now").about(
            "one pass that fetches every application on every machine whose backups are \
             switched on, whatever its schedule — manual-only ones included, exempt ones never",
        ))
        .subcommand(
            clap::Command::new("status")
                .about("what has been backed up, when, and how stale that is"),
        )
        .subcommand(clap::Command::new("notify-preview").about(
            "show one sample of every Windows notification this program can raise, a few \
             seconds apart, so you can see what they look like. Contacts nothing",
        ))
        .subcommand(clap::Command::new("monitor").about(
            "run the monitor ([monitor], SPEC-02) in the foreground, printing every attempt's \
             outcome, until Ctrl+C. Runs NO backups",
        ))
}

fn main() {
    let matches = parser().get_matches();

    // **Ignition first.** `--help` and `--version` are handled by clap during
    // parsing and never reach here; the `help` and `version` VERBS do, and
    // `dispatch` answers them. Anything else comes back as `Passthrough` and is
    // this program's to run.
    //
    // The update config is Ignition-owned schema this program does not embed,
    // because its update surface is `Off` — `dispatch` ignores the value unless
    // an update action is present.
    match surface().dispatch(&matches, &ignition::update::UpdateConfig::default()) {
        Ok(ignition::cli::Outcome::Handled) => return,
        Ok(ignition::cli::Outcome::Passthrough) => {}
        Err(e) => {
            eprintln!("rivet-monitor: {e}");
            std::process::exit(2);
        }
    }

    let path_override = matches.get_one::<String>("config").map(PathBuf::from);
    let code = match matches.subcommand_name() {
        None => run_tray(path_override.as_deref()),
        Some("check") => report(path_override.as_deref(), false, false),
        Some("run-once") => report(path_override.as_deref(), true, false),
        Some("back-up-now") => report(path_override.as_deref(), true, true),
        Some("register") => show_register(path_override.as_deref()),
        Some("status") => show_status(path_override.as_deref()),
        Some("monitor") => run_monitor(path_override),
        Some("notify-preview") => {
            #[cfg(windows)]
            {
                let logging = config::load(path_override.as_deref())
                    .map(|loaded| loaded.config.logging)
                    .unwrap_or_default();
                notify::preview(&tray::log_dir(&logging));
            }
            #[cfg(not(windows))]
            notify::preview(std::path::Path::new("logs"));
            0
        }
        // Unreachable: clap refuses an unknown subcommand during parsing, with
        // its own error and a suggestion. Kept as a value rather than a panic
        // because this crate denies `panic` and a surface must not crash.
        Some(_) => 2,
    };
    std::process::exit(code);
}

/// Read the configuration and the ledger, or say why not.
fn open(path_override: Option<&std::path::Path>) -> Result<(Config, PathBuf, Ledger), String> {
    let loaded = config::load(path_override)?;
    let ledger = Ledger::load(&loaded.path)?;
    if loaded.absent {
        eprintln!(
            "note: there is no configuration file at {} yet, so every value is a default — and \
             the required ones have none.",
            loaded.path.display()
        );
    }
    Ok((loaded.config, loaded.path, ledger))
}

/// `check`, `run-once` and `back-up-now`: one pass, printed.
fn report(path_override: Option<&std::path::Path>, confirm: bool, force: bool) -> i32 {
    let (config, config_path, mut ledger) = match open(path_override) {
        Ok(what) => what,
        Err(why) => {
            eprintln!("{why}");
            return 1;
        }
    };
    let now = ledger::now();
    let mut context = schedule::Context {
        config: &config,
        ledger: &mut ledger,
        now,
        zone: &clock::Local,
        confirm,
        force,
    };
    let pass = schedule::run(&mut context);

    println!("configuration  {}", config_path.display());
    if let Some(version) = &pass.tool_version {
        println!("cargo-rivet    {version}");
    }
    if let Some(url) = &pass.portal_url {
        println!("fleet portal   {url}");
    }
    if !pass.members.is_empty() {
        println!("\nfleet            health  backups  home page");
        for member in &pass.members {
            // The two switches, SPEC-01 §2.6 — what the tray's flyout checkmarks.
            let health = if config.monitoring_on(&member.label) { "on" } else { "off" };
            let backups = if config.backups_on(&member.label) { "on" } else { "off" };
            match &member.index_url {
                Some(url) => println!("  {:<16} {health:<7} {backups:<8} {url}", member.label),
                None => println!(
                    "  {:<16} {health:<7} {backups:<8} {} — no proxy answered on 443, so its \
                     flyout's Open item is greyed",
                    member.label, member.dns
                ),
            }
        }
    }
    if !pass.health.is_empty() {
        println!("\nhealth");
        for (host, health) in &pass.health {
            println!("  {host:<16} {}", health.machine);
            for service in &health.services {
                if service.health == "healthy" && !service.critical {
                    continue;
                }
                let mark = if service.critical { "*" } else { " " };
                match &service.caption {
                    Some(caption) => println!(
                        "    {mark} {:<20} {:<9} {caption}",
                        service.name, service.health
                    ),
                    None => println!("    {mark} {:<20} {}", service.name, service.health),
                }
            }
        }
        println!(
            "  (* is a service the machine's own document calls critical, which is what\n   \
             decides whether it reaches the machine's word — Rivet ADR-045 §4.3.)"
        );
    }
    if !pass.stored.is_empty() {
        println!("\nstored");
        for one in &pass.stored {
            println!("  {}", one.display());
        }
    }
    if !pass.notes.is_empty() {
        println!("\nnotes");
        for note in &pass.notes {
            println!("  {note}");
        }
    }
    if !pass.alerts.is_empty() {
        println!("\nALERTS");
        for alert in &pass.alerts {
            println!("  {}", alert.message);
        }
    }
    if !pass.refusals.is_empty() {
        println!("\nREFUSED — nothing was fetched");
        for refusal in &pass.refusals {
            println!("  {refusal}");
        }
    }
    // The monitor's refusals stop no backup (SPEC-02 §3.1), but they are a
    // problem: each one not already a backup refusal is among the alerts above
    // (`monitor-refusal/`, SPEC-01 §5.8), so it counts against the pass's result
    // like any other, and is said once.

    // **Only a pass that acted settles the standing record.** `check` runs
    // fewer checks and fetches nothing; letting it settle would let a
    // diagnostic the operator typed consume the announcement that the pass
    // doing the work was about to make.
    if confirm {
        let (announced, save_failed) = alarm::settle(&pass, &mut ledger, now);
        if !announced.is_empty() {
            println!("\nCHANGED SINCE THE LAST PASS");
            for one in &announced {
                let verb = match one.change {
                    alarm::Change::Raised => "started",
                    alarm::Change::Cleared => "fixed",
                };
                println!("  [{verb}] {}", one.summary);
            }
            println!(
                "\n(The tray raises the critical few of these as Windows notifications, once \
                 each and after ten minutes — SPEC-01 §5.5.4. This command raises none.)"
            );
        }
        if let Some(why) = save_failed {
            eprintln!(
                "the record of what has already been announced could not be written, so the \
                 next pass will repeat itself: {why}"
            );
        }
    }

    println!(
        "\nlast successful backup: {}",
        pass.newest_success
            .map_or_else(|| "never".to_string(), |t| ledger::ago(t, now))
    );
    i32::from(!pass.healthy())
}

/// `monitor` — `SPEC-02` §3.6: the monitor alone, in the foreground.
///
/// **It runs no backup.** It exists so the monitor can be run and asserted
/// against without the tray, the way `run-once` exists for the backup pass —
/// and like `run-once`, and unlike `check`, it ACTS, so it settles the
/// monitor's own announcements (`SPEC-02` §6, `crate::conditions`). It never
/// touches `state.toml`.
fn run_monitor(path_override: Option<PathBuf>) -> i32 {
    let stop = Arc::new(AtomicBool::new(false));
    on_ctrl_c(&stop);
    match config::config_path(path_override.as_deref()) {
        Some(path) => println!("monitor  {}  (Ctrl+C stops it)", path.display()),
        None => println!("monitor  (no configuration directory)  (Ctrl+C stops it)"),
    }
    let report: monitor::Reporter = Arc::new(|event| {
        for line in monitor::describe(&event) {
            println!("{line}");
        }
    });
    monitor::run(path_override, &report, &stop);
    println!("stopped: every worker finished the capture it was in");
    0
}

/// The flag Ctrl+C sets.
static STOP: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();

/// **The first Ctrl+C asks the monitor to stop; a second one ends the process.**
///
/// Stopping rather than dying lets each worker finish the line it is writing,
/// so a file never ends in half a line. A second press is the operator saying
/// he will not wait, and Windows' own handler is then allowed to run.
#[cfg(windows)]
fn on_ctrl_c(stop: &Arc<AtomicBool>) {
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
    use windows_sys::core::BOOL;

    unsafe extern "system" fn handler(_kind: u32) -> BOOL {
        match STOP.get() {
            Some(flag) if !flag.swap(true, Ordering::SeqCst) => 1,
            _ => 0,
        }
    }
    let _ = STOP.set(Arc::clone(stop));
    // SAFETY: `handler` is a valid `extern "system"` routine for the whole
    // life of the process, and it touches only a static atomic.
    unsafe { SetConsoleCtrlHandler(Some(handler), 1) };
}

#[cfg(not(windows))]
fn on_ctrl_c(stop: &Arc<AtomicBool>) {
    let _ = STOP.set(Arc::clone(stop));
    let _ = Ordering::SeqCst;
}

/// `register`.
fn show_register(path_override: Option<&std::path::Path>) -> i32 {
    let (config, _, _) = match open(path_override) {
        Ok(what) => what,
        Err(why) => {
            eprintln!("{why}");
            return 1;
        }
    };
    let Some(server) = config.fleet.server.clone() else {
        eprintln!("fleet.server is not configured, so there is no register to read.");
        return 1;
    };
    match register::fetch(
        &server,
        config.fleet.name.as_deref(),
        &config.authority(),
        4443,
    ) {
        Ok(reg) => {
            println!(
                "fleet {}  ({} device(s))",
                reg.fleet.name.as_deref().unwrap_or("(unnamed)"),
                reg.devices.len()
            );
            for device in &reg.devices {
                println!("\n  {}", device.name);
                println!("    id        {}", device.id);
                println!("    dns       {}", config.dns_name(&device.name));
                println!("    role      {}", device.role.as_deref().unwrap_or("-"));
                println!(
                    "    location  {}",
                    device.location.as_deref().unwrap_or("-")
                );
                match device.target() {
                    Ok(target) => {
                        println!("    control   {}:{}", target.address, target.control_port);
                        println!("    pin       {}", target.pin);
                    }
                    Err(why) => println!("    UNUSABLE  {why}"),
                }
            }
            0
        }
        Err(why) => {
            eprintln!("{why}");
            1
        }
    }
}

/// `status`.
fn show_status(path_override: Option<&std::path::Path>) -> i32 {
    let (config, _, ledger) = match open(path_override) {
        Ok(what) => what,
        Err(why) => {
            eprintln!("{why}");
            return 1;
        }
    };
    let now = ledger::now();
    println!("ledger  {}", ledger.path().display());
    if ledger.success.is_empty() {
        println!("  nothing has been successfully backed up yet.");
    }
    for (key, at) in &ledger.success {
        let schedule = key
            .split_once('/')
            .and_then(|(host, app)| config.resolve(host, app).ok())
            .map_or_else(|| "no schedule".to_string(), |r| r.schedule.to_string());
        println!("  {key:<40} {} ({schedule})", ledger::ago(*at, now));
    }
    println!(
        "\nlast successful backup: {}",
        ledger
            .newest()
            .map_or_else(|| "never".to_string(), |t| ledger::ago(t, now))
    );
    0
}

#[cfg(windows)]
fn run_tray(path_override: Option<&std::path::Path>) -> i32 {
    match tray::run(path_override.map(std::path::Path::to_path_buf)) {
        Ok(()) => 0,
        Err(why) => {
            eprintln!("{why}");
            1
        }
    }
}

#[cfg(not(windows))]
fn run_tray(_path_override: Option<&std::path::Path>) -> i32 {
    eprintln!(
        "Rivet Monitor's tray is Windows-only by decision (ADR-001 §2): a tray icon is a \
         platform's own idiom, and the honest way to serve another platform is a second program. \
         The commands still run here — try `check`."
    );
    2
}
