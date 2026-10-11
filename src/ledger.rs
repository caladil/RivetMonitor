//! **When each application was last SUCCESSFULLY backed up** — `SPEC-01` §3.1.
//!
//! The whole schedule turns on this file. An application is due when nothing
//! has succeeded since its most recent scheduled time (`crate::timetable`), so
//! a time that passed while this program was not running is caught up on the
//! next pass rather than missed.
//!
//! **Only a success is written here.** A failed fetch leaves the previous entry
//! exactly where it was, which is what makes the application stay due and the
//! icon keep aging — the failure has to remain visible until a real backup
//! replaces it.
//!
//! It is TOML beside the configuration file, and epoch seconds rather than a
//! civil date: this program has no date library, and adding one to spell a
//! timestamp nothing but this file reads would be a dependency for a cosmetic.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Seconds since the Unix epoch, now.
#[must_use]
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// The record of successes.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Ledger {
    /// `host/application` → epoch seconds of the last stored, validated file.
    #[serde(default)]
    pub success: BTreeMap<String, u64>,
    /// **Conditions the operator has already been told about**, and when each
    /// was first announced — `SPEC-01` §5.5.
    ///
    /// It lives here, in the same file, because it is the same kind of fact:
    /// something that must survive a restart or the program repeats itself. A
    /// notification record kept only in memory would announce every standing
    /// problem afresh at every login, which is the behaviour this whole design
    /// exists to avoid.
    ///
    /// `serde(default)` so a `state.toml` written before this field existed
    /// still loads. An absent record means nothing has been announced, which is
    /// true of a machine that had none.
    #[serde(default)]
    pub standing: BTreeMap<String, u64>,
    #[serde(skip)]
    path: PathBuf,
}

/// `host/application`.
#[must_use]
pub fn key(host: &str, application: &str) -> String {
    format!("{host}/{application}")
}

impl Ledger {
    /// Read the ledger beside the configuration file.
    ///
    /// **An absent ledger is an empty one**, not an error: a first run has
    /// backed nothing up, which is exactly what an empty ledger says.
    ///
    /// # Errors
    ///
    /// A file that exists and is not this shape — which is worth refusing,
    /// because reading it as empty would make every application due at once
    /// and then overwrite the record of what really happened.
    pub fn load(config_path: &Path) -> Result<Self, String> {
        let path = beside(config_path);
        if !path.exists() {
            return Ok(Self {
                path,
                ..Self::default()
            });
        }
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut ledger: Self =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        ledger.path = path;
        Ok(ledger)
    }

    /// Where it lives.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// When this application last succeeded.
    #[must_use]
    pub fn last_success(&self, host: &str, application: &str) -> Option<u64> {
        self.success.get(&key(host, application)).copied()
    }

    /// The most recent success across the whole fleet — what `check` and
    /// `run-once` print as the last successful backup. The icon judges each
    /// application on its own (`SPEC-01` §5.4).
    #[must_use]
    pub fn newest(&self) -> Option<u64> {
        self.success.values().copied().max()
    }

    /// Record a success and write the file.
    ///
    /// # Errors
    ///
    /// A file that cannot be written, which is worth surfacing: a lost ledger
    /// means the next run re-fetches everything, which is wasteful rather than
    /// wrong, and a silently lost one means nobody knows that happened.
    pub fn record(&mut self, host: &str, application: &str, at: u64) -> Result<(), String> {
        self.success.insert(key(host, application), at);
        self.save()
    }

    /// Write it, whole, through a temporary file so a reader never sees half.
    ///
    /// # Errors
    ///
    /// Anything the filesystem refuses.
    pub fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|e| format!("the ledger will not serialise: {e}"))?;
        let temporary = self.path.with_extension("toml.tmp");
        std::fs::write(&temporary, text.as_bytes())
            .map_err(|e| format!("cannot write {}: {e}", temporary.display()))?;
        std::fs::rename(&temporary, &self.path)
            .map_err(|e| format!("cannot rename {}: {e}", temporary.display()))
    }

    /// Point the ledger at a path, for a test or a `--config` run.
    pub fn with_path(mut self, path: PathBuf) -> Self {
        self.path = path;
        self
    }
}

/// `state.toml`, beside the configuration file.
fn beside(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("state.toml")
}

/// "9 days ago", "4 hours ago", "just now" — for the tooltip and the report.
#[must_use]
pub fn ago(then: u64, now: u64) -> String {
    let seconds = now.saturating_sub(then);
    let days = seconds / 86_400;
    if days > 0 {
        return format!("{days} day{} ago", if days == 1 { "" } else { "s" });
    }
    let hours = seconds / 3_600;
    if hours > 0 {
        return format!("{hours} hour{} ago", if hours == 1 { "" } else { "s" });
    }
    let minutes = seconds / 60;
    if minutes > 0 {
        return format!(
            "{minutes} minute{} ago",
            if minutes == 1 { "" } else { "s" }
        );
    }
    "just now".to_string()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn ago_reads_as_a_person_would_say_it() {
        assert_eq!(ago(0, 86_400 * 9), "9 days ago");
        assert_eq!(ago(0, 86_400), "1 day ago");
        assert_eq!(ago(0, 7_200), "2 hours ago");
        assert_eq!(ago(0, 120), "2 minutes ago");
        assert_eq!(ago(0, 5), "just now");
    }
}
