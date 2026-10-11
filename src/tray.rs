//! **The notification-area icon and its menu** — `SPEC-01` §5.
//!
//! ```text
//! Fleet Portal
//! --------------------------------
//! rivet-01          ♥ 🡇  ▸     one flyout per register member (§5.6)
//! dev-01                ▸        Open dev-01
//! --------------------------------   ------------------
//! Back Up Now                        Health monitoring  (checkmark)
//! View Config                        Backups            (checkmark)
//! View Log
//! Start with Windows (checkmark)
//! --------------------------------
//! Quit
//! ```
//!
//! **The member list is derived, never configured** (§5.2). It comes from the
//! register the background pass already reads, so a machine joining the fleet
//! appears here without anybody editing anything. Every member gets a flyout,
//! because its switches are worth reaching whether or not its proxy answered;
//! only the flyout's *Open* item waits on the proxy, and is greyed without one.
//!
//! **The checkmarks are read from the file, not from the snapshot** (§5.6). The
//! file is the one authority for both switches, and the operator may have
//! edited it by hand since the last pass.
//!
//! **One list, and one entry is not a special case** — it is the list with one
//! thing in it. There is no shape to change when the fleet grows.
//!
//! # Two threads, and only one of them may touch the icon
//!
//! Win32 tray and menu handles belong to the thread that made them. So the
//! **worker** thread does the network and the fetching and publishes a
//! [`Snapshot`]; the **message-loop** thread owns the icon and rebuilds the
//! menu from that snapshot on a timer. Nothing crosses the other way.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tray_icon::menu::{
    CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
};
use tray_icon::{Icon, TrayIconBuilder};

use crate::icon::State;
use crate::ledger::{self, Ledger};
use crate::schedule;
use crate::switches::Switch;

/// How often the message loop looks for a new snapshot.
const UI_TICK_MS: u32 = 1_000;

/// How many ticks between re-reads of the two standing records (§5.8).
const PROBLEMS_EVERY: u32 = 10;

/// How many problems the menu lists before summarising the rest.
const PROBLEMS_SHOWN: usize = 8;

/// What the worker publishes and the menu is built from.
#[derive(Debug, Default)]
struct Snapshot {
    generation: u64,
    state_current: bool,
    state_refused: bool,
    tooltip: String,
    portal_url: Option<String>,
    /// `(register name, proxy home page)` — **every** register member, and the
    /// page only where its proxy answered.
    members: Vec<(String, Option<String>)>,
    /// **When this was published**, in epoch seconds, and **how long the worker
    /// said it would be before the next one**. Added 2026-09-18.
    ///
    /// Every other field here describes the fleet. These two describe *this
    /// program*, and they exist because on that day the worker thread died and
    /// the tray went on displaying the colour it had published then — for
    /// hours, while the machine it was reporting on came back and ran
    /// perfectly. A red that cannot change looks exactly like
    /// a red that is right.
    ///
    /// [`crate::icon::State::Stopped`] is decided from these by the event loop,
    /// which is alive whether or not the worker is.
    published_at: u64,
    /// `0` until the first pass completes, which is what lets the event loop say
    /// *nothing has been decided yet* rather than *the worker is gone*.
    next_due_seconds: u64,
}

/// What a menu item does when it is clicked.
#[derive(Clone)]
enum Action {
    Open(String),
    OpenConfig(PathBuf),
    /// The newest log in this directory, or the directory if it holds none.
    OpenLog(PathBuf),
    ToggleAutostart,
    /// Flip one machine's switch in the configuration file — `SPEC-01` §2.7.
    Switch { which: Switch, host: String },
    /// Wake the worker for a pass that backs up everything — `SPEC-01` §3.7.
    BackUpNow,
    Quit,
}

/// **Run the tray.** Returns when the operator chooses Quit.
///
/// # Errors
///
/// A tray the shell will not create, or an icon this build cannot make.
pub fn run(path_override: Option<PathBuf>) -> Result<(), String> {
    detach_own_console();

    // **A configuration that will not load must NOT stop the tray appearing.**
    // This used to be a `?`, and the failure it produced was the worst one this
    // program has: the operator mistypes a key, logs in, and there is no icon
    // at all — nothing to click, nothing to read, and no way to reach `View
    // Config` and fix it. A monitor that vanishes when something is wrong is
    // the opposite of a monitor.
    //
    // So the tray comes up red with the reason in its tooltip, and the worker
    // clears it within half a minute of the file being fixed.
    let loaded = crate::config::load(path_override.as_deref());
    let config_path = match &loaded {
        Ok(loaded) => loaded.path.clone(),
        Err(_) => crate::config::config_path(path_override.as_deref()).ok_or(
            "there is no configuration directory on this system, so there is nowhere for a \
             configuration file to be",
        )?,
    };

    // Logging is Ignition's, and it is started AFTER the configuration is read
    // because the configuration is what says where the log goes. A file that
    // did not parse gets the defaults, which is better than no log at all on
    // the run where something is wrong.
    let logging = match &loaded {
        Ok(loaded) => loaded.config.logging.clone(),
        Err(_) => ignition::logging::LogConfig::default(),
    };
    let _log = ignition::logging::init(crate::config::APP_NAME, &logging);
    let log_dir = log_dir(&logging);
    // Name this program to Windows' notifications before any can be raised.
    if let Err(why) = crate::notify::register() {
        tracing::error!("{why}; notifications will be attributed to PowerShell");
    }

    let opening = match &loaded {
        Ok(_) => String::from("Rivet Monitor — starting"),
        Err(why) => format!("Rivet Monitor — {why}"),
    };
    let snapshot = Arc::new(Mutex::new(Snapshot {
        state_refused: loaded.is_err(),
        tooltip: opening.clone(),
        ..Snapshot::default()
    }));

    // **Back Up Now**, from the menu to the worker. A flag rather than a
    // channel: two clicks before the worker wakes are one request, not two.
    let back_up_now = Arc::new(std::sync::atomic::AtomicBool::new(false));
    spawn_worker(
        path_override.clone(),
        Arc::clone(&snapshot),
        Arc::clone(&back_up_now),
    );

    // **The monitor, on its own timer** — `SPEC-02` §3.1. It shares nothing
    // with the backup worker above, so a fetch in progress never delays a
    // capture. With no `[monitor]` table it reads the file on its tick and
    // captures nothing. Its events go where the tray's announcements go.
    let _monitor = crate::monitor::spawn(
        path_override.clone(),
        Arc::new(|event| {
            for line in crate::monitor::describe(&event) {
                tracing::info!("{line}");
            }
        }),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    );

    let mut actions: Vec<(MenuId, Action)> = Vec::new();
    let menu = {
        let held = snapshot.lock().map_err(poisoned)?;
        build_menu(&held, &[], ledger::now(), &config_path, &log_dir, &mut actions)?
    };
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(&opening)
        .with_icon(make_icon(if loaded.is_err() {
            State::Refused
        } else {
            State::Unknown
        })?)
        .build()
        .map_err(|e| format!("the notification area would not take an icon: {e}"))?;

    let mut shown = 0_u64;
    // **What the event loop needs to judge the WORKER**, as opposed to the
    // fleet — `State::Stopped`, added 2026-09-18. `started` dates the case
    // where the very first pass panics and no snapshot is ever published;
    // `stopped_shown` stops the icon being rewritten every tick once it has;
    // `stopped_notified` is the notification's own once, which can come later.
    let started = ledger::now();
    let mut stopped_shown = false;
    let mut stopped_notified = false;
    // **Everything standing, from both records** (§5.8), re-read every
    // `PROBLEMS_EVERY` ticks. The monitor settles on its own cadence, so waiting
    // for the backup pass would leave a health problem off the menu for up to
    // five minutes.
    let mut problems: Vec<crate::problems::Problem> = Vec::new();
    let mut ticks = 0_u32;
    pump(|| {
        if ticks.is_multiple_of(PROBLEMS_EVERY) {
            let now_standing = crate::problems::standing(&config_path, ledger::now());
            let keys = |list: &[crate::problems::Problem]| {
                list.iter().map(|p| p.key.clone()).collect::<Vec<_>>()
            };
            if keys(&now_standing) != keys(&problems) {
                // Rebuilt only when the SET changes: a menu replaced while it
                // is open can close under the operator's pointer.
                shown = u64::MAX;
            }
            problems = now_standing;
            // The critical few, once each, after they have lasted (§5.5.4).
            crate::notify::check(&config_path, &problems, ledger::now(), &log_dir);
        }
        ticks = ticks.wrapping_add(1);
        // The snapshot the worker last published.
        let Ok(current) = snapshot.lock() else {
            return true;
        };

        // **Is the worker still there at all?** This runs on the tray's own
        // event loop, which is alive whether or not the worker thread is —
        // and that asymmetry is the whole point. Every other colour below is
        // read out of a snapshot, so if nothing publishes one, every other
        // colour is whatever was true when the worker died.
        //
        // **Three intervals, not one.** A pass that is slow — a machine timing
        // out at twenty seconds, several of them — must not be called dead, and
        // a `poll-seconds` of 300 makes the window fifteen minutes.
        let silence = silence_of(
            current.generation,
            current.published_at,
            started,
            ledger::now(),
        );
        if silence > allowed_silence(current.generation, current.next_due_seconds) {
            if !stopped_shown {
                stopped_shown = true;
                tracing::error!(
                    "no backup pass has published for {silence} s, which is more than three \
                     intervals, so the worker is treated as STOPPED. The icon says so rather \
                     than going on showing what it last published: a colour that cannot change \
                     reads as one that is still true"
                );
                if let Ok(icon) = make_icon(State::Stopped) {
                    let _ = tray.set_icon(Some(icon));
                }
                let _ = tray.set_tooltip(Some(
                    "Rivet Monitor — THIS PROGRAM has stopped checking. Nothing below is \
                     current; restart it.",
                ));
            }
            // **The icon goes black at three intervals; the notification waits
            // until the silence has also lasted ten minutes**, as every
            // critical notification does (§5.5.4). Three intervals of a
            // 30-second cadence is ninety seconds.
            if !stopped_notified && stopped_has_lasted(silence) {
                stopped_notified = true;
                crate::notify::stopped(&log_dir);
            }
            drop(current);
            return true;
        }
        if stopped_shown {
            // **Coming back.** The worker published again, so the black has to
            // go — and it will not go on its own if the generation happens to
            // match what was last drawn, because the branch below only repaints
            // on a change. Forcing a mismatch repaints from the live snapshot.
            stopped_shown = false;
            stopped_notified = false;
            shown = u64::MAX;
        }

        if current.generation != shown {
            shown = current.generation;
            // **Red for anything standing in EITHER record** — a health problem
            // colours the icon exactly as a backup problem does (§5.4).
            let state = if current.state_refused || !problems.is_empty() {
                State::Refused
            } else if current.state_current {
                State::Current
            } else {
                State::Stale
            };
            if let Ok(icon) = make_icon(state) {
                let _ = tray.set_icon(Some(icon));
            }
            let _ = tray.set_tooltip(Some(&with_problem_count(&current.tooltip, problems.len())));
            let mut rebuilt = Vec::new();
            let now = ledger::now();
            if let Ok(menu) =
                build_menu(&current, &problems, now, &config_path, &log_dir, &mut rebuilt)
            {
                tray.set_menu(Some(Box::new(menu)));
                actions = rebuilt;
            }
        }
        drop(current);

        while let Ok(event) = MenuEvent::receiver().try_recv() {
            let Some((_, action)) = actions.iter().find(|(id, _)| *id == event.id) else {
                continue;
            };
            match action.clone() {
                Action::Open(url) => crate::shell::open_url(&url),
                Action::OpenConfig(path) => crate::shell::open_path(&path),
                Action::OpenLog(dir) => crate::shell::open_path(&newest_log(&dir).unwrap_or(dir)),
                Action::ToggleAutostart => {
                    if let Err(why) = crate::autostart::set(!crate::autostart::enabled()) {
                        tracing::error!("{why}");
                    }
                }
                Action::Switch { which, host } => {
                    flip(&config_path, which, &host);
                    // **The checkmark and the pill follow at once** (§5.6), not
                    // at the next pass: the menu is rebuilt from the file on the
                    // next tick. Forcing a mismatch is how the branch above is
                    // told to redraw when the worker has published nothing new.
                    shown = u64::MAX;
                }
                Action::BackUpNow => {
                    back_up_now.store(true, std::sync::atomic::Ordering::SeqCst);
                    tracing::info!(
                        "Back Up Now chosen from the menu: the next pass, starting within a \
                         second unless one is already running, backs up every application on \
                         every machine whose backups are switched on, whatever its schedule"
                    );
                }
                Action::Quit => return false,
            }
        }
        true
    });
    Ok(())
}

fn poisoned<T>(_: T) -> String {
    "the snapshot lock was poisoned, which means the worker panicked".to_string()
}

/// Build the menu from a snapshot, filling `actions` with what each item does.
fn build_menu(
    snapshot: &Snapshot,
    problems: &[crate::problems::Problem],
    now: u64,
    config_path: &std::path::Path,
    log_dir: &std::path::Path,
    actions: &mut Vec<(MenuId, Action)>,
) -> Result<Menu, String> {
    let menu = Menu::new();
    let add = |menu: &Menu, item: &dyn tray_icon::menu::IsMenuItem| -> Result<(), String> {
        menu.append(item)
            .map_err(|e| format!("the menu would not take an item: {e}"))
    };

    // **What is wrong, first** (§5.8): one plain line per standing problem,
    // newest first. Each opens the log, which is where the detail is. No lines
    // at all is the menu saying nothing is wrong.
    for problem in problems.iter().take(PROBLEMS_SHOWN) {
        let item = MenuItem::new(crate::problems::line(problem, now), true, None);
        actions.push((item.id().clone(), Action::OpenLog(log_dir.to_path_buf())));
        add(&menu, &item)?;
    }
    if problems.len() > PROBLEMS_SHOWN {
        let more = MenuItem::new(
            format!("\u{2026} and {} more (View Log)", problems.len() - PROBLEMS_SHOWN),
            true,
            None,
        );
        actions.push((more.id().clone(), Action::OpenLog(log_dir.to_path_buf())));
        add(&menu, &more)?;
    }
    if !problems.is_empty() {
        add(&menu, &PredefinedMenuItem::separator())?;
    }

    let portal = MenuItem::new("Fleet Portal", snapshot.portal_url.is_some(), None);
    if let Some(url) = &snapshot.portal_url {
        actions.push((portal.id().clone(), Action::Open(url.clone())));
    }
    add(&menu, &portal)?;
    add(&menu, &PredefinedMenuItem::separator())?;

    // **Derived from the register** (§5.2), labelled with the machine's register
    // name (§5.3), one flyout each (§5.6). The switches are read from the file
    // here, at build; a file that will not load leaves them greyed rather than
    // showing a state nobody can vouch for.
    let config = crate::config::load(Some(config_path)).ok().map(|l| l.config);
    for (host, url) in &snapshot.members {
        let on = |which: Switch| config.as_ref().is_some_and(|c| which.is_on(c, host));
        let flyout = Submenu::new(
            with_pill(host, on(Switch::Monitoring), on(Switch::Backups)),
            true,
        );
        let append = |item: &dyn tray_icon::menu::IsMenuItem| -> Result<(), String> {
            flyout
                .append(item)
                .map_err(|e| format!("the flyout would not take an item: {e}"))
        };

        let open = MenuItem::new(format!("Open {host}"), url.is_some(), None);
        if let Some(url) = url {
            actions.push((open.id().clone(), Action::Open(url.clone())));
        }
        append(&open)?;
        append(&PredefinedMenuItem::separator())?;
        // **A name that could not be a folder cannot be switched on** (`SPEC-01`
        // §4.4): a block written for it would only be refused, and a refused
        // `[[monitor.machine]]` stops the whole monitor. Greyed, not hidden —
        // the machine is still in the fleet, and the menu says so.
        let switchable = config.is_some() && crate::register::usable_name(host).is_ok();
        for which in Switch::ALL {
            let item = CheckMenuItem::new(which.label(), switchable, on(which), None);
            actions.push((
                item.id().clone(),
                Action::Switch {
                    which,
                    host: host.clone(),
                },
            ));
            append(&item)?;
        }
        add(&menu, &flyout)?;
    }
    if !snapshot.members.is_empty() {
        add(&menu, &PredefinedMenuItem::separator())?;
    }

    // **The only way a manual-only application is ever backed up** (§3.7).
    let now_item = MenuItem::new("Back Up Now", true, None);
    actions.push((now_item.id().clone(), Action::BackUpNow));
    add(&menu, &now_item)?;

    let view = MenuItem::new("View Config", true, None);
    actions.push((
        view.id().clone(),
        Action::OpenConfig(config_path.to_path_buf()),
    ));
    add(&menu, &view)?;

    // **Where the tooltip's "see the log" points** (§5). Without this the log
    // is somewhere only Ignition's own documentation says.
    let log = MenuItem::new("View Log", true, None);
    actions.push((log.id().clone(), Action::OpenLog(log_dir.to_path_buf())));
    add(&menu, &log)?;

    let start = CheckMenuItem::new(
        "Start with Windows",
        true,
        crate::autostart::enabled(),
        None,
    );
    actions.push((start.id().clone(), Action::ToggleAutostart));
    add(&menu, &start)?;
    add(&menu, &PredefinedMenuItem::separator())?;

    let quit = MenuItem::new("Quit", true, None);
    actions.push((quit.id().clone(), Action::Quit));
    add(&menu, &quit)?;
    Ok(menu)
}

/// **The tooltip, counting every standing problem** — both records, not only
/// the backup pass's — and pointing at the menu, which is where they are named.
fn with_problem_count(tooltip: &str, count: usize) -> String {
    let mut out: Vec<&str> = tooltip
        .lines()
        .filter(|line| !line.contains("problem(s)"))
        .collect();
    let counted;
    if count > 0 {
        counted = format!(
            "{count} problem{} — right-click to see {}",
            if count == 1 { "" } else { "s" },
            if count == 1 { "it" } else { "them" }
        );
        out.push(&counted);
    }
    let mut out = out.join("\n");
    // A tray tooltip is 127 characters on Windows and silently truncated, so
    // the cut is made here where it can be seen.
    if out.chars().count() > 120 {
        out = out.chars().take(117).collect::<String>() + "...";
    }
    out
}

/// **Where Ignition writes this program's log** — its own rule, restated
/// because Ignition keeps the resolver private: an explicit `logging.dir`, else
/// `logs` beside the DEFAULT configuration file. The default, not a `--config`
/// override, because that is the path Ignition itself asks for.
pub(crate) fn log_dir(logging: &ignition::logging::LogConfig) -> PathBuf {
    if let Some(dir) = &logging.dir {
        return crate::config::expand_tilde(dir);
    }
    crate::config::config_path(None)
        .and_then(|path| path.parent().map(|p| p.join("logs")))
        .unwrap_or_else(|| PathBuf::from("logs"))
}

/// The most recently written `.log` in `dir` — the one being appended to.
pub(crate) fn newest_log(dir: &std::path::Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "log"))
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

/// **A machine's menu label, with the pill** — `SPEC-01` §5.7.
///
/// The marks sit after a tab, which Windows draws as a right-aligned column —
/// the one accelerators use — so they line up down the list whatever the names'
/// lengths. A machine with both switches off carries no pill at all.
fn with_pill(host: &str, monitoring: bool, backups: bool) -> String {
    let marks: Vec<&str> = [(monitoring, "\u{2665}"), (backups, "\u{1F847}")]
        .into_iter()
        .filter_map(|(on, mark)| on.then_some(mark))
        .collect();
    if marks.is_empty() {
        host.to_string()
    } else {
        format!("{host}\t{}", marks.join(" "))
    }
}

/// Flip one machine's switch, reading its current state from the file rather
/// than from the checkmark — the file may have been edited by hand since the
/// menu was drawn, and the flip must be the opposite of what is TRUE.
fn flip(config_path: &std::path::Path, which: Switch, host: &str) {
    let current = match crate::config::load(Some(config_path)) {
        Ok(loaded) => which.is_on(&loaded.config, host),
        Err(why) => {
            tracing::error!(
                "the {} switch for {host} was NOT changed, because the configuration will not \
                 load: {why}",
                which.word()
            );
            return;
        }
    };
    match crate::switches::set(config_path, which, host, !current) {
        Ok(()) => tracing::info!(
            "{} for {host} switched {} from the menu; it takes effect on the next pass",
            which.label(),
            if current { "off" } else { "on" }
        ),
        Err(why) => tracing::error!("{why}"),
    }
}

fn make_icon(state: State) -> Result<Icon, String> {
    Icon::from_rgba(
        crate::icon::rgba(state),
        crate::icon::size(),
        crate::icon::size(),
    )
    .map_err(|e| format!("the icon will not build: {e}"))
}

/// **How long since the worker last said anything**, in seconds.
///
/// Before the first pass publishes there is no snapshot to date, so the
/// silence is measured from when the tray started — which is what catches a
/// worker that panics on its very first pass and never publishes at all.
const fn silence_of(generation: u64, published_at: u64, started: u64, now: u64) -> u64 {
    let since = if generation == 0 { started } else { published_at };
    now.saturating_sub(since)
}

/// **How much silence is ordinary** before the worker is called stopped.
///
/// **Three intervals, not one.** A pass does real network work — several
/// machines, each able to time out at twenty seconds — so a slow pass must not
/// be mistaken for a dead one. At the default `poll-seconds` of 300 this is a
/// fifteen-minute window, which is slow enough to be sure and fast enough that
/// nobody spends an evening believing a frozen icon.
///
/// Before the first pass there is no interval to multiply, so the floor cadence
/// stands in: `30 × 6` is the same three intervals a first pass would get if it
/// were running at the fastest rate this program ever uses.
const fn allowed_silence(generation: u64, next_due_seconds: u64) -> u64 {
    if generation == 0 {
        30 * 6
    } else {
        next_due_seconds.saturating_mul(3)
    }
}

/// **Whether a stopped worker has been silent long enough to notify** — the
/// same ten minutes every critical notification has to last (`SPEC-01`
/// §5.5.4), which three short intervals need not reach.
const fn stopped_has_lasted(silence: u64) -> bool {
    silence >= crate::notify::GRACE_SECONDS
}

/// The background pass: read the fleet, fetch what is due, publish a snapshot.
fn spawn_worker(
    path_override: Option<PathBuf>,
    snapshot: Arc<Mutex<Snapshot>>,
    back_up_now: Arc<std::sync::atomic::AtomicBool>,
) {
    use std::sync::atomic::Ordering;

    std::thread::spawn(move || {
        let mut generation = 0_u64;
        loop {
            // Taken here, before the pass, so a click during a pass is the
            // NEXT pass's request rather than lost.
            let force = back_up_now.swap(false, Ordering::SeqCst);
            // **A panic in one pass must not end every future pass** — added
            // 2026-09-18, the day one did.
            //
            // This loop had no guard, so a panic anywhere inside `pass_once`
            // unwound out of the thread and killed it silently. On that day the
            // thread died while the machine it watches was unreachable; the
            // machine came back within the hour and the tray went on showing
            // the last colour this worker ever published, for hours. **Nothing
            // anywhere said the worker had
            // gone**, and the icon was indistinguishable from one reporting a
            // real fault.
            //
            // `catch_unwind` needs the payload to be unwind-safe; the snapshot
            // is behind a `Mutex` and the generation is a counter this loop
            // owns, so the only state a panicked pass can leave behind is a
            // poisoned lock — which `publish` and the event loop both already
            // treat as *no snapshot*, not as a reason to stop.
            let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                pass_once(path_override.as_deref(), &snapshot, &mut generation, force)
            }));
            let wait = match attempt {
                Ok(wait) => wait,
                Err(panicked) => {
                    // The payload is whatever was passed to `panic!`, which for
                    // an `expect` or an index is a string. Anything else is
                    // reported as unnameable rather than dropped.
                    let said = panicked
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_string())
                        .or_else(|| panicked.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "a panic carrying no message".to_string());
                    tracing::error!(
                        "the backup pass PANICKED and was caught: {said}. The next pass runs on \
                         the ordinary interval; this is reported rather than fatal because a \
                         worker that dies leaves a tray icon frozen on its last colour, which \
                         reads as a fault that is still true"
                    );
                    // The ordinary floor rather than the configured interval:
                    // the configuration may be what panicked, and re-reading one
                    // small file costs nothing.
                    30
                }
            };
            // **A second at a time**, so Back Up Now does not wait out the
            // interval. Nothing else is checked here: the wait is still the
            // interval unless that one flag is raised.
            for _ in 0..wait {
                if back_up_now.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    });
}

/// One pass, published. Returns how long to wait before the next.
fn pass_once(
    path_override: Option<&std::path::Path>,
    snapshot: &Arc<Mutex<Snapshot>>,
    generation: &mut u64,
    force: bool,
) -> u64 {
    let publish = |snapshot: &Arc<Mutex<Snapshot>>, next: Snapshot| {
        if let Ok(mut held) = snapshot.lock() {
            *held = next;
        }
    };
    *generation += 1;

    let loaded = match crate::config::load(path_override) {
        Ok(loaded) => loaded,
        Err(why) => {
            // **A configuration that will not load is itself an announcement**
            // — `SPEC-01` §5.5. The icon goes red, and nobody is looking at the
            // icon at three in the morning; the operator edited the file, got a
            // key wrong, and every backup on this machine has stopped.
            //
            // It is settled without a pass because there was no pass. The path
            // is known even when the parse failed, which is the whole reason
            // `config::config_path` exists — the standing record lives beside
            // the file, and a condition with nowhere to be written down would
            // be announced afresh every thirty seconds.
            announce_unreadable_config(path_override, &why);
            publish(
                snapshot,
                Snapshot {
                    generation: *generation,
                    state_refused: true,
                    tooltip: format!("Rivet Monitor — {why}"),
                    // This pass reached a conclusion, so it counts as the worker
                    // being alive — a broken config is a reading, not a silence.
                    published_at: ledger::now(),
                    next_due_seconds: 30,
                    ..Snapshot::default()
                },
            );
            // **Half a minute, not the five-minute default.** The operator who
            // broke this file is most likely editing it right now, and the
            // interval he would otherwise wait is five minutes of a red icon
            // after he has already fixed it. Re-reading one small file is not a
            // cost worth spacing out.
            return 30;
        }
    };
    let mut ledger = Ledger::load(&loaded.path).unwrap_or_default().with_path(
        loaded
            .path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("state.toml"),
    );
    let now = ledger::now();
    let mut context = schedule::Context {
        config: &loaded.config,
        ledger: &mut ledger,
        now,
        zone: &crate::clock::Local,
        confirm: true,
        force,
    };
    let pass = schedule::run(&mut context);

    // **What changed since the last pass**, once each — `SPEC-01` §5.5. This is
    // where the Windows notification will be raised (§5.5.4); until then the
    // announcements go to the log, which is the whole of their delivery. They
    // are settled here rather than only in `run-once` because the tray is what
    // actually runs on a cadence, so it is the pass that would repeat itself.
    let (announced, save_failed) = crate::alarm::settle(&pass, &mut ledger, now);
    for one in &announced {
        let verb = match one.change {
            crate::alarm::Change::Raised => "started",
            crate::alarm::Change::Cleared => "fixed",
        };
        tracing::warn!("[{verb}] {} — {}", one.key, one.summary);
    }
    if let Some(why) = save_failed {
        tracing::error!(
            "the record of what has already been announced could not be written, so the next \
             pass will repeat itself: {why}"
        );
    }

    let wait = loaded.config.fleet.poll_seconds.max(30);
    // The ledger AFTER the pass, as the announcement reads it, so an
    // application that has just been backed up is not drawn late.
    let late = pass
        .watched
        .iter()
        .any(|w| w.late(ledger.last_success(&w.host, &w.service)));
    let state = crate::icon::state_for(!pass.refusals.is_empty(), pass.alerts.len(), late);
    publish(
        snapshot,
        Snapshot {
            generation: *generation,
            state_current: state == State::Current,
            state_refused: state == State::Refused,
            tooltip: tooltip(&pass, now),
            portal_url: pass.portal_url.clone(),
            members: pass
                .members
                .iter()
                .map(|m| (m.label.clone(), m.index_url.clone()))
                .collect(),
            published_at: now,
            next_due_seconds: wait,
        },
    );
    wait
}

/// Settle the one condition a pass cannot report, because it stopped the pass
/// from happening: the configuration file will not load.
fn announce_unreadable_config(path_override: Option<&std::path::Path>, why: &str) {
    let Some(path) = crate::config::config_path(path_override) else {
        tracing::error!("{why}");
        return;
    };
    let mut ledger = Ledger::load(&path).unwrap_or_default().with_path(
        path.parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("state.toml"),
    );
    let now = ledger::now();
    let reading = crate::alarm::Reading::of_one(crate::alarm::CONFIG, why.to_string());
    for one in crate::alarm::reconcile(&mut ledger.standing, &reading, now) {
        tracing::warn!("[started] {} — {}", one.key, one.summary);
    }
    let _ = ledger.save();
}

/// **What the icon says when the pointer rests on it** — §5.4.
///
/// The last successful backup comes first, always, because it is the one thing
/// nothing else in the system watches.
fn tooltip(pass: &schedule::Pass, now: u64) -> String {
    let mut out = format!(
        "Rivet Monitor — last successful backup: {}",
        pass.newest_success
            .map_or_else(|| "NEVER".to_string(), |t| ledger::ago(t, now))
    );
    for refusal in &pass.refusals {
        out.push_str(&format!("\n{}: no backups run", refusal.key));
    }
    if !pass.alerts.is_empty() {
        out.push_str(&format!("\n{} problem(s) — see the log", pass.alerts.len()));
    }
    // A tray tooltip is 127 characters on Windows and is silently truncated,
    // so it is truncated HERE where the cut can be seen.
    if out.chars().count() > 120 {
        out = out.chars().take(117).collect::<String>() + "...";
    }
    out
}

/// The Win32 message loop, waking on a timer so the snapshot is picked up.
///
/// `each` returns `false` to stop.
fn pump<F: FnMut() -> bool>(mut each: F) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage, WM_QUIT,
    };

    loop {
        let mut message: MSG = unsafe { std::mem::zeroed() };
        // SAFETY: `message` is a valid, zeroed MSG; a null HWND means "any
        // window on this thread", which is what the tray's hidden window is.
        while unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if message.message == WM_QUIT {
                return;
            }
            // SAFETY: `message` was filled by PeekMessageW immediately above.
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if !each() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(u64::from(UI_TICK_MS)));
    }
}

/// Let go of a console this process allocated for itself.
///
/// **Only when this process is the only one attached**, which means the shell
/// gave it a fresh console because it was double-clicked. Started from a
/// terminal, the console belongs to that terminal and is left alone — the
/// commands in `main` print to it, and detaching would silence them.
fn detach_own_console() {
    use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};

    let mut owners = [0_u32; 2];
    // SAFETY: the buffer and its length agree.
    let count = unsafe { GetConsoleProcessList(owners.as_mut_ptr(), 2) };
    if count == 1 {
        // SAFETY: no console handle is used after this point.
        unsafe { FreeConsole() };
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **The incident of 2026-09-18, as a rule.**
    ///
    /// The backup worker died. The machine it watches came back within the
    /// hour and ran perfectly for hours more, and the tray went on showing the
    /// colour the worker had published at the moment it
    /// died. Nothing said the worker had gone; the icon was indistinguishable
    /// from one reporting a real, current fault.
    #[test]
    fn a_worker_that_has_published_nothing_for_three_intervals_is_stopped() {
        // Published at 1_000, said it would publish again in 300 s.
        let published = 1_000;
        let interval = 300;
        for (now, stopped) in [
            (1_100, false),          // inside the first interval
            (1_300, false),          // one interval late — a slow pass
            (1_900, false),          // exactly three intervals, still allowed
            (1_901, true),           // past three
            (1_000 + 4 * 3_600, true), // hours of silence
        ] {
            let silence = silence_of(1, published, 0, now);
            assert_eq!(
                silence > allowed_silence(1, interval),
                stopped,
                "at now={now} the silence was {silence} s against an allowance of {}",
                allowed_silence(1, interval)
            );
        }
    }

    /// **The icon can go black before anyone is interrupted.** At the floor
    /// cadence three intervals is ninety seconds, and the notification still
    /// waits the ten minutes every critical one has to last (`SPEC-01` §5.5.4).
    #[test]
    fn a_stopped_worker_notifies_only_after_ten_minutes() {
        let stopped_at = allowed_silence(7, 30) + 1;
        assert!(!stopped_has_lasted(stopped_at), "black at {stopped_at} s, but no notification yet");
        assert!(!stopped_has_lasted(crate::notify::GRACE_SECONDS - 1));
        assert!(stopped_has_lasted(crate::notify::GRACE_SECONDS));
        // At the default cadence the icon's own wait is already longer.
        assert!(stopped_has_lasted(allowed_silence(1, 300) + 1));
    }

    /// **A slow pass is not a dead one**, which is why the allowance is three
    /// intervals rather than one. A pass talks to real machines and each can
    /// take the full twenty-second timeout.
    #[test]
    fn the_allowance_is_three_intervals_and_the_floor_applies_before_the_first_pass() {
        assert_eq!(allowed_silence(1, 300), 900, "three of the default interval");
        assert_eq!(allowed_silence(7, 30), 90, "three of the floor");
        assert_eq!(
            allowed_silence(0, 0),
            180,
            "before anything has published there is no interval to multiply, so the floor \
             cadence stands in — otherwise an allowance of zero would call a starting tray dead"
        );
    }

    /// **A worker that panics on its very first pass never publishes at all**,
    /// so there is no snapshot to date the silence from. It is measured from
    /// when the tray started instead — without this the generation-zero case
    /// would compare against `published_at` of `0`, read the silence as the
    /// whole of the Unix epoch, and report every startup as stopped.
    #[test]
    fn silence_before_the_first_pass_is_measured_from_startup_not_from_the_epoch() {
        let started = 10_000;
        assert_eq!(silence_of(0, 0, started, 10_060), 60);
        assert!(
            silence_of(0, 0, started, 10_060) < allowed_silence(0, 0),
            "a tray one minute old is starting, not stopped"
        );
        assert!(
            silence_of(0, 0, started, 10_500) > allowed_silence(0, 0),
            "a tray whose first pass has not published in eight minutes has a dead worker"
        );
    }

    /// **View Log opens the log being written**, not the oldest, and a folder
    /// with no log in it is no file at all.
    #[test]
    fn the_newest_log_is_the_most_recently_written_one() {
        let dir = std::env::temp_dir().join(format!("rivet-monitor-logs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(newest_log(&dir), None, "an empty folder has no newest log");
        std::fs::write(dir.join("RivetMonitor-1.log"), "old").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(dir.join("RivetMonitor-2.log"), "new").unwrap();
        std::fs::write(dir.join("notes.txt"), "not a log").unwrap();
        assert_eq!(newest_log(&dir), Some(dir.join("RivetMonitor-2.log")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// **The tooltip counts every problem and sends you to the menu**, and a
    /// tooltip with none says nothing about problems at all.
    #[test]
    fn the_tooltip_counts_problems_from_both_records() {
        let pass = "Rivet Monitor — last successful backup: 14 hours ago\n1 problem(s) — see the log";
        assert_eq!(
            with_problem_count(pass, 2),
            "Rivet Monitor — last successful backup: 14 hours ago\n2 problems — right-click to see them"
        );
        assert_eq!(
            with_problem_count(pass, 0),
            "Rivet Monitor — last successful backup: 14 hours ago"
        );
        assert!(with_problem_count(pass, 1).ends_with("1 problem — right-click to see it"));
    }

    /// **The pill shows only what is on**, and a machine with nothing on has
    /// none — `SPEC-01` §5.7.
    #[test]
    fn the_pill_carries_one_mark_per_switch_that_is_on() {
        assert_eq!(with_pill("dev-01", false, false), "dev-01");
        assert_eq!(with_pill("rivet-01", true, false), "rivet-01\t\u{2665}");
        assert_eq!(with_pill("rivet-01", false, true), "rivet-01\t\u{1F847}");
        assert_eq!(with_pill("rivet-01", true, true), "rivet-01\t\u{2665} \u{1F847}");
    }

    /// Clocks go backwards — a resync, a virtual machine resuming. A silence
    /// that underflowed would wrap to an enormous number and paint the icon
    /// black for something that is working.
    #[test]
    fn a_clock_that_went_backwards_is_no_silence_at_all() {
        assert_eq!(silence_of(1, 5_000, 0, 4_000), 0);
        assert_eq!(silence_of(0, 0, 5_000, 4_000), 0);
    }

    /// `Stopped` must not be reachable from a pass. It is the event loop's
    /// judgement about the worker, and a pass that reached a conclusion — even
    /// the conclusion that the configuration is broken — is a live worker.
    #[test]
    fn a_pass_that_reached_a_conclusion_is_never_silent() {
        let published = ledger::now();
        assert_eq!(silence_of(1, published, 0, published), 0);
    }
}
