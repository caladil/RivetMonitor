//! **Windows notifications, for the few problems that deserve one** —
//! `SPEC-01` §5.5.4.
//!
//! The menu's problems list (§5.8) shows everything; this interrupts only for
//! what the maintainer named critical on 2026-09-23, and only once:
//!
//! * **once per problem** — a problem is identified by its key AND when it
//!   began, so it notifies when it has lasted [`GRACE_SECONDS`] and never again
//!   while it stands, however many checks see it;
//! * **survives restarts** — what has been notified is kept in
//!   `notified.toml` beside the configuration file, so restarting this program
//!   or the PC repeats nothing;
//! * **has to last** — a problem younger than [`GRACE_SECONDS`] is not
//!   notified, so a board rebooted on purpose does not interrupt anyone;
//! * **no "cleared" notification** — a problem that goes away leaves the menu
//!   quietly, as ruled.
//!
//! The standing records are read, never written, here: their writers are the
//! backup pass and the monitor.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::problems::Problem;

/// How long a problem must have stood before it notifies — ruled 10 minutes.
pub const GRACE_SECONDS: u64 = 600;

/// The AppUserModelID notifications are sent as. Registered per user by
/// [`register`], so Windows names the sender "Rivet Monitor" rather than the
/// PowerShell a toast without one is attributed to.
pub const APP_ID: &str = "Caladil.RivetMonitor";

/// What this program is called on a notification.
const DISPLAY_NAME: &str = "Rivet Monitor";

/// The record of what has been notified, beside the configuration file.
const FILE: &str = "notified.toml";

/// **Whether a problem is one of the kinds that notify**, as ruled: a watched
/// machine not answering, a watched machine's critical application unhealthy,
/// the fleet server unreachable, a backup overdue, the configuration broken.
/// (Rivet Monitor itself stopping is the sixth, raised by the tray's own event
/// loop rather than from a record — see [`stopped`].)
#[must_use]
pub fn is_critical(key: &str) -> bool {
    key == "register"
        || key == "config"
        || ["reach/", "health/", "late/"]
            .iter()
            .any(|prefix| key.starts_with(prefix))
}

/// **Which standing problems should notify now**: critical, standing at least
/// [`GRACE_SECONDS`], and not already notified for this same beginning.
#[must_use]
pub fn due(problems: &[Problem], notified: &BTreeMap<String, u64>, now: u64) -> Vec<Problem> {
    problems
        .iter()
        .filter(|p| is_critical(&p.key))
        .filter(|p| now.saturating_sub(p.since) >= GRACE_SECONDS)
        .filter(|p| notified.get(&p.key) != Some(&p.since))
        .cloned()
        .collect()
}

/// **What has been notified, and is still standing, and still began then.** A
/// problem that cleared, or cleared and came back with a new beginning, is
/// forgotten, so a return is a new problem that notifies again.
#[must_use]
pub fn remembered(notified: &BTreeMap<String, u64>, problems: &[Problem]) -> BTreeMap<String, u64> {
    notified
        .iter()
        .filter(|(key, since)| problems.iter().any(|p| &p.key == *key && p.since == **since))
        .map(|(key, since)| (key.clone(), *since))
        .collect()
}

/// The record's path, beside `config_path`.
fn record_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(FILE)
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct OnDisk {
    #[serde(default)]
    notified: BTreeMap<String, u64>,
}

/// Read the record. **Unreadable reads as everything already notified is
/// unknown** — the worst case is one repeated notification, which is better
/// than a problem that never notifies because a file was damaged.
fn load(path: &Path) -> BTreeMap<String, u64> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| toml::from_str::<OnDisk>(&text).ok())
        .map(|disk| disk.notified)
        .unwrap_or_default()
}

fn save(path: &Path, notified: &BTreeMap<String, u64>) -> Result<(), String> {
    let text = toml::to_string(&OnDisk {
        notified: notified.clone(),
    })
    .map_err(|e| format!("{e}"))?;
    let staged = path.with_extension("toml.writing");
    std::fs::write(&staged, text).map_err(|e| format!("{}: {e}", staged.display()))?;
    std::fs::rename(&staged, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// **One check against the standing problems**: notify what is due, and keep
/// the record to what still stands. Called from the tray's event loop each time
/// it re-reads the records.
pub fn check(config_path: &Path, problems: &[Problem], now: u64, log_dir: &Path) {
    let path = record_path(config_path);
    let before = load(&path);
    let mut notified = remembered(&before, problems);
    for problem in due(problems, &notified, now) {
        show(&crate::problems::describe(&problem.key), log_dir);
        notified.insert(problem.key.clone(), problem.since);
    }
    if notified != before
        && let Err(why) = save(&path, &notified)
    {
        tracing::error!(
            "the record of which problems have been notified could not be written, so a \
             restart may notify one again: {why}"
        );
    }
}

/// **Rivet Monitor itself has stopped checking** — the sixth critical kind.
/// The tray's event loop calls this once per stoppage, when the worker has
/// been judged stopped (three intervals) AND silent at least
/// [`GRACE_SECONDS`] — at a short cadence the first comes well before the
/// second.
pub fn stopped(log_dir: &Path) {
    show(STOPPED, log_dir);
}

/// **One sample of every kind that notifies**, for the operator to see what
/// they look like — `rivet-monitor notify-preview`. The wording is the real
/// wording, derived from real keys by the same function the tray uses.
pub fn preview(log_dir: &Path) {
    if let Err(why) = register() {
        eprintln!("rivet-monitor: {why} — the samples will show as sent by PowerShell or not at all");
    }
    let samples = [
        "reach/rivet-01",
        "health/rivet-01",
        "register",
        "late/rivet-01/photos-rivet",
        "config",
    ];
    let texts = samples
        .iter()
        .map(|key| crate::problems::describe(key))
        .chain(std::iter::once(STOPPED.to_string()));
    for text in texts {
        match raise(&text, log_dir) {
            Ok(()) => println!("shown:   {text}"),
            Err(why) => println!("REFUSED: {text} — {why}"),
        }
        std::thread::sleep(std::time::Duration::from_secs(4));
    }
}

const STOPPED: &str = "Rivet Monitor has stopped checking. Nothing it shows is current; restart it.";

/// Raise one notification, logging a refusal rather than failing: the menu
/// still shows the problem.
fn show(text: &str, log_dir: &Path) {
    tracing::warn!("[notified] {text}");
    if let Err(why) = raise(text, log_dir) {
        tracing::error!("Windows refused the notification {text:?}: {why}");
    }
}

/// Raise one notification. Clicking it opens the log.
#[cfg(windows)]
fn raise(text: &str, log_dir: &Path) -> Result<(), String> {
    use tauri_winrt_notification::Toast;

    let dir = log_dir.to_path_buf();
    Toast::new(APP_ID)
        .title(DISPLAY_NAME)
        .text1(text)
        .on_activated(move |_| {
            let target = crate::tray::newest_log(&dir).unwrap_or_else(|| dir.clone());
            crate::shell::open_path(&target);
            Ok(())
        })
        .show()
        .map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn raise(_text: &str, _log_dir: &Path) -> Result<(), String> {
    Err("Windows notifications exist only on Windows".to_string())
}

/// **Name this program to Windows' notification system**, under the current
/// user's own key: `HKCU\Software\Classes\AppUserModelId\<APP_ID>`, with the
/// name a notification shows as its sender.
///
/// Written at every start rather than on a click, unlike "Start with Windows":
/// it changes nothing about when or whether this program runs, only what its
/// notifications are called, and a stale or missing one is repaired by the
/// next start.
///
/// # Errors
///
/// The registry's own refusal.
#[cfg(windows)]
pub fn register() -> Result<(), String> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
        RegCreateKeyExW, RegSetValueExW,
    };

    let path = wide(&format!("Software\\Classes\\AppUserModelId\\{APP_ID}"));
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: `path` is NUL-terminated; every other pointer is valid or the
    // documented null.
    let created = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if created != ERROR_SUCCESS {
        return Err(format!(
            "cannot create HKCU\\Software\\Classes\\AppUserModelId\\{APP_ID}: {created}"
        ));
    }
    let name = wide("DisplayName");
    let value = wide(DISPLAY_NAME);
    let bytes = std::mem::size_of_val(&value[..]);
    // SAFETY: `value` is a NUL-terminated UTF-16 buffer and `bytes` its exact
    // length, which is what REG_SZ requires.
    let set = unsafe {
        RegSetValueExW(
            key,
            name.as_ptr(),
            0,
            REG_SZ,
            value.as_ptr().cast::<u8>(),
            u32::try_from(bytes).unwrap_or(0),
        )
    };
    // SAFETY: `key` came from a successful create and is not used again.
    unsafe { RegCloseKey(key) };
    if set == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("the registry refused the notification name: {set}"))
    }
}

#[cfg(not(windows))]
pub fn register() -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt as _;
    std::ffi::OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn p(key: &str, since: u64) -> Problem {
        Problem {
            key: key.to_string(),
            since,
        }
    }

    /// **The kinds ruled critical, and only those.**
    #[test]
    fn only_the_ruled_kinds_are_critical() {
        for key in ["reach/rivet-01", "health/rivet-01", "register", "late/rivet-01/photos-rivet", "config"] {
            assert!(is_critical(key), "{key}");
        }
        for key in [
            "frozen/rivet-01",
            "fetch/dev-01",
            "offers/dev-01",
            "log-stalled/rivet-01",
            "power/rivet-01",
            "capture/rivet-01/thermal",
            "registered",
        ] {
            assert!(!is_critical(key), "{key}");
        }
    }

    /// **Has to last**: a reboot's minute of "not answering" never notifies.
    #[test]
    fn a_problem_younger_than_the_grace_does_not_notify() {
        let problems = [p("reach/rivet-01", 1_000)];
        assert!(due(&problems, &BTreeMap::new(), 1_000 + GRACE_SECONDS - 1).is_empty());
        assert_eq!(due(&problems, &BTreeMap::new(), 1_000 + GRACE_SECONDS).len(), 1);
    }

    /// **Once per problem**: a problem already notified for this beginning is
    /// never notified again, however many checks see it.
    #[test]
    fn a_notified_problem_is_not_notified_again() {
        let problems = [p("reach/rivet-01", 1_000)];
        let mut notified = BTreeMap::new();
        for now in (1_600..20_000).step_by(10) {
            for one in due(&problems, &notified, now) {
                notified.insert(one.key, one.since);
            }
        }
        assert_eq!(notified.len(), 1, "one notification across every check");
        assert!(due(&problems, &notified, 50_000).is_empty());
    }

    /// **A problem that cleared and came back is a new problem**, and
    /// notifies again once it has lasted.
    #[test]
    fn a_problem_that_returns_notifies_again() {
        let first = [p("reach/rivet-01", 1_000)];
        let notified: BTreeMap<String, u64> = [("reach/rivet-01".to_string(), 1_000)].into();
        assert_eq!(remembered(&notified, &first), notified, "still standing, still remembered");
        assert!(remembered(&notified, &[]).is_empty(), "cleared, forgotten");

        let again = [p("reach/rivet-01", 9_000)];
        let kept = remembered(&notified, &again);
        assert!(kept.is_empty(), "a new beginning is not the notified one");
        assert_eq!(due(&again, &kept, 9_000 + GRACE_SECONDS).len(), 1);
    }

    /// Nothing but the ruled kinds ever notifies, however long it stands.
    #[test]
    fn a_problem_not_ruled_critical_never_notifies() {
        let problems = [p("frozen/rivet-01", 0), p("fetch/dev-01", 0)];
        assert!(due(&problems, &BTreeMap::new(), 1_000_000).is_empty());
    }

    /// **Survives restarts**: the record round-trips, and a damaged one reads
    /// as empty rather than stopping notifications.
    #[test]
    fn the_record_survives_a_restart_and_a_damaged_one_is_empty() {
        let dir = std::env::temp_dir().join(format!("rivet-monitor-notify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = record_path(&dir.join("config.toml"));
        let notified: BTreeMap<String, u64> = [("register".to_string(), 42)].into();
        save(&path, &notified).unwrap();
        assert_eq!(load(&path), notified);
        std::fs::write(&path, "not toml [").unwrap();
        assert!(load(&path).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
