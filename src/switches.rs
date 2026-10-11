//! **A machine's two switches, written into the operator's own file** —
//! `SPEC-01` §2.6 and §2.7.
//!
//! The configuration file is the one authority for whether a machine is backed
//! up or watched, and it is also a file its operator writes by hand, with
//! comments explaining his choices. So a switch is **one value changed in
//! place**: `toml_edit` holds the file as the text it was, and everything the
//! edit does not touch is written back byte for byte.
//!
//! Two rules keep the file trustworthy while two authors share it:
//!
//! * **Switching off sets `enabled = false` and never deletes a block.** A block
//!   can carry signal settings and overrides, and a switch that dropped them on
//!   the way off would lose them on the way back on.
//! * **A file this program would refuse is never written**, before or after the
//!   edit. The operator may be half way through changing it by hand.

use std::path::Path;

use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

use crate::config::Config;

/// Which of a machine's two switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    /// `enabled` on the machine's `[[monitor.machine]]`.
    Monitoring,
    /// `enabled` on the machine's `[[backups.machine]]`.
    Backups,
}

impl Switch {
    /// Both, in the order the menu shows them.
    pub const ALL: [Self; 2] = [Self::Monitoring, Self::Backups];

    /// What the menu calls it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Monitoring => "Health monitoring",
            Self::Backups => "Backups",
        }
    }

    /// Its name in a log line or a refusal.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Monitoring => "monitoring",
            Self::Backups => "backups",
        }
    }

    /// The table whose `machine` array holds it.
    const fn section(self) -> &'static str {
        match self {
            Self::Monitoring => "monitor",
            Self::Backups => "backups",
        }
    }

    /// Whether `config` has it on for `host`.
    #[must_use]
    pub fn is_on(self, config: &Config, host: &str) -> bool {
        match self {
            Self::Monitoring => config.monitoring_on(host),
            Self::Backups => config.backups_on(host),
        }
    }
}

/// **Set one machine's switch in the file at `path`.**
///
/// An absent file is edited as an empty one, which creates it. Writing is
/// replace-by-rename, so a reader never sees half a file.
///
/// # Errors
///
/// A file that cannot be read, that this program would refuse, or that cannot
/// be replaced. **In every one of those cases nothing is written.**
pub fn set(path: &Path, switch: Switch, host: &str, on: bool) -> Result<(), String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let edited = edit(&text, switch, host, on)?;
    if edited == text {
        return Ok(());
    }
    let staged = path.with_extension("toml.switching");
    std::fs::write(&staged, &edited)
        .map_err(|e| format!("{}: {e}", staged.display()))?;
    std::fs::rename(&staged, path).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        format!("{}: {e}", path.display())
    })
}

/// **The edit, on the file's text** — what [`set`] writes.
///
/// # Errors
///
/// Text this program would refuse, before or after the edit, a `[backups]`
/// or `[monitor]` that is not shaped as `SPEC-01` §2 says, or a name that could
/// not be a folder (`SPEC-01` §4.4) — the menu greys those, and this is the
/// same rule where the file is written.
pub fn edit(text: &str, switch: Switch, host: &str, on: bool) -> Result<String, String> {
    crate::register::usable_name(host).map_err(|why| {
        format!("the {} switch was NOT written: {why}", switch.word())
    })?;
    toml::from_str::<Config>(text).map_err(|e| {
        format!(
            "the configuration will not load, so the {} switch for {host} was NOT written — the \
             file is left exactly as it is: {e}",
            switch.word()
        )
    })?;
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e| format!("the configuration is not TOML: {e}"))?;

    let section = doc.entry(switch.section()).or_insert_with(|| {
        // **Implicit**, so a `[monitor]` created only to hold a machine is not
        // written as an empty header of its own.
        let mut table = Table::new();
        table.set_implicit(true);
        Item::Table(table)
    });
    let section = section
        .as_table_mut()
        .ok_or_else(|| format!("`{}` is not a table", switch.section()))?;
    let machines = section
        .entry("machine")
        .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()));
    let machines = machines.as_array_of_tables_mut().ok_or_else(|| {
        format!(
            "`{}.machine` is not a list of [[{}.machine]] blocks",
            switch.section(),
            switch.section()
        )
    })?;

    let mut found = false;
    for block in machines.iter_mut() {
        if block.get("name").and_then(Item::as_str) == Some(host) {
            found = true;
            set_enabled(block, on);
        }
    }
    // Off, with no block, is already off: nothing to write.
    if !found && on {
        machines.push(new_block(switch, host));
    }

    let mut out = doc.to_string();
    // `toml_edit` ends every line it generates with `\n`. A file written in
    // `\r\n` stays in `\r\n`, rather than becoming a mixture.
    if text.contains("\r\n") {
        out = out.replace("\r\n", "\n").replace('\n', "\r\n");
    }

    // **Read back what is about to be written**, as this program will read it.
    let written = toml::from_str::<Config>(&out)
        .map_err(|e| format!("the edited configuration would not load, so it was NOT written: {e}"))?;
    if switch.is_on(&written, host) != on {
        return Err(format!(
            "the {} switch for {host} would still read {} after the edit, so it was NOT \
             written; two blocks for one machine are the likeliest reason",
            switch.word(),
            if on { "off" } else { "on" }
        ));
    }
    Ok(out)
}

/// Set `enabled` on an existing block, keeping any comment beside it.
fn set_enabled(block: &mut Table, on: bool) {
    if let Some(existing) = block.get_mut("enabled").and_then(Item::as_value_mut) {
        let decor = existing.decor().clone();
        *existing = on.into();
        *existing.decor_mut() = decor;
    } else {
        block.insert("enabled", value(on));
    }
}

/// A block for a machine that had none — `SPEC-01` §2.7.
///
/// A monitoring block carries the `status` signal at its defaults: the switch
/// reads *health monitoring* and must record something, and `status` is the
/// signal that is health and costs almost nothing. Heavier signals stay the
/// operator's addition.
fn new_block(switch: Switch, host: &str) -> Table {
    let mut block = Table::new();
    block.decor_mut().set_prefix("\n");
    block.insert("name", value(host));
    block.insert("enabled", value(true));
    if switch == Switch::Monitoring {
        block.insert("status", Item::Table(Table::new()));
    }
    block
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// A configuration shaped like a real one: comments everywhere, a monitored
    /// machine with signal tables, and no backup switches at all.
    const COMMENTED: &str = "\
# The operator's own words about this file.

[backups]
# Where they go.
folder    = \"D:\\\\Backups\"
schedule  = [\"Tue 04:00\", \"Fri 04:00\"] # twice a week
retention = 3

[fleet]
server   = \"rivet-01.local:4443\"
identity = \"~/.rivet/identity/operator\"

[tool]
path = \"cargo-rivet.exe\"

[monitor]
folder = 'D:\\Monitor'

[[monitor.machine]]
name = \"rivet-01\"
# why these signals
[monitor.machine.status]
[monitor.machine.log]
";

    fn load(text: &str) -> Config {
        toml::from_str(text).expect("the edited file loads")
    }

    /// **The reason this module exists**: an edit that dropped the operator's
    /// comments would silently delete his reasoning.
    #[test]
    fn a_switch_leaves_every_comment_where_it_was() {
        let out = edit(COMMENTED, Switch::Backups, "rivet-01", true).unwrap();
        for comment in [
            "# The operator's own words about this file.",
            "# Where they go.",
            "# twice a week",
            "# why these signals",
        ] {
            assert!(out.contains(comment), "lost {comment:?}:\n{out}");
        }
        assert!(load(&out).backups_on("rivet-01"));
    }

    /// Switching ON a machine with no block adds one, and nothing else changes.
    #[test]
    fn switching_on_a_machine_with_no_block_adds_one() {
        let out = edit(COMMENTED, Switch::Backups, "rivet-01", true).unwrap();
        assert!(out.contains("[[backups.machine]]"), "{out}");
        let config = load(&out);
        assert!(config.backups_on("rivet-01"));
        assert!(!config.backups_on("dev-01"), "another machine was switched on");
        // Every line that was there is still there.
        for line in COMMENTED.lines() {
            assert!(out.contains(line), "lost {line:?}:\n{out}");
        }
    }

    /// **Off sets `false`; it never deletes.** The block's settings — here, its
    /// signal tables — are still in the file for when it is switched back on.
    #[test]
    fn switching_off_keeps_the_block_and_its_settings() {
        let on = edit(COMMENTED, Switch::Monitoring, "rivet-01", true).unwrap();
        let off = edit(&on, Switch::Monitoring, "rivet-01", false).unwrap();
        assert!(off.contains("[monitor.machine.status]"), "{off}");
        assert!(off.contains("[monitor.machine.log]"), "{off}");
        assert!(off.contains("# why these signals"), "{off}");
        let config = load(&off);
        assert!(!config.monitoring_on("rivet-01"));
        assert_eq!(config.monitor.as_ref().unwrap().machines.len(), 1);
        let back = edit(&off, Switch::Monitoring, "rivet-01", true).unwrap();
        let resolved = load(&back).monitor.unwrap().resolve();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].signals.len(), 2, "the signals came back with it");
    }

    /// **A block written before switches existed has no `enabled`, and is off**
    /// — `enabled` defaults to `false`. Switching it on adds the key and keeps
    /// the rest.
    #[test]
    fn a_block_without_enabled_is_off_and_switching_it_on_adds_the_key() {
        assert!(!load(COMMENTED).monitoring_on("rivet-01"));
        let out = edit(COMMENTED, Switch::Monitoring, "rivet-01", true).unwrap();
        assert!(load(&out).monitoring_on("rivet-01"));
        assert_eq!(out.matches("[[monitor.machine]]").count(), 1, "{out}");
    }

    /// A comment beside `enabled` belongs to the operator, and survives a flip.
    #[test]
    fn a_comment_beside_enabled_survives_the_flip() {
        let text = format!(
            "{COMMENTED}\n[[backups.machine]]\nname = \"rivet-01\"\nenabled = true # the server\n"
        );
        let out = edit(&text, Switch::Backups, "rivet-01", false).unwrap();
        assert!(out.contains("enabled = false # the server"), "{out}");
    }

    /// **A monitoring block the menu creates records something** — `status`,
    /// at its defaults — rather than reading *on* and capturing nothing.
    #[test]
    fn a_created_monitoring_block_captures_status() {
        let out = edit(COMMENTED, Switch::Monitoring, "dev-01", true).unwrap();
        let resolved = load(&out).monitor.unwrap().resolve();
        let dev = resolved.iter().find(|m| m.name == "dev-01").expect("dev-01 is on");
        assert_eq!(dev.signals.len(), 1);
        assert_eq!(dev.signals[0].signal, crate::config::Signal::Status);
    }

    /// **A file this program would refuse is never written** — the operator may
    /// be half way through an edit, and a switch must not finish it for him.
    #[test]
    fn a_file_that_will_not_load_is_refused_and_not_edited() {
        let broken = format!("{COMMENTED}\n[backups.nonsense]\n");
        let why = edit(&broken, Switch::Backups, "rivet-01", true).unwrap_err();
        assert!(why.contains("NOT written"), "{why}");
    }

    /// Setting a switch to what it already is changes nothing at all.
    #[test]
    fn a_switch_already_in_that_state_changes_nothing() {
        assert_eq!(edit(COMMENTED, Switch::Backups, "dev-01", false).unwrap(), COMMENTED);
        let on = edit(COMMENTED, Switch::Backups, "rivet-01", true).unwrap();
        assert_eq!(edit(&on, Switch::Backups, "rivet-01", true).unwrap(), on);
    }

    /// A file written in `\r\n` stays in `\r\n`.
    #[test]
    fn crlf_stays_crlf() {
        let crlf = COMMENTED.replace('\n', "\r\n");
        let out = edit(&crlf, Switch::Backups, "rivet-01", true).unwrap();
        assert!(!out.replace("\r\n", "").contains('\n'), "a bare \\n crept in:\n{out:?}");
        assert!(load(&out).backups_on("rivet-01"));
    }

    /// An empty file — no configuration yet — takes a switch and still loads.
    #[test]
    fn an_empty_file_takes_a_switch() {
        let out = edit("", Switch::Backups, "rivet-01", true).unwrap();
        assert!(load(&out).backups_on("rivet-01"), "{out}");
    }

    /// **`SPEC-01` §4.4: a name that could not be a folder is never written.**
    /// A `[[monitor.machine]]` for it would be refused, and that refusal stops
    /// the whole monitor — so one bad register name could switch every
    /// machine's monitoring off.
    #[test]
    fn a_name_that_could_not_be_a_folder_is_not_written() {
        for switch in Switch::ALL {
            for bad in ["..", "nul", "rivet-02.", "C:\\x"] {
                let refused = edit("", switch, bad, true).expect_err(bad);
                assert!(refused.contains("was NOT written"), "{refused}");
            }
        }
    }

    /// Two blocks for one machine: both are set, so the switch reads what was
    /// asked whichever block a reader takes.
    #[test]
    fn two_blocks_for_one_machine_are_both_set() {
        let text = format!(
            "{COMMENTED}\n[[backups.machine]]\nname = \"rivet-01\"\n\n[[backups.machine]]\nname = \"rivet-01\"\n"
        );
        let out = edit(&text, Switch::Backups, "rivet-01", true).unwrap();
        assert_eq!(out.matches("enabled = true").count(), 2, "{out}");
    }

    /// The file on disk: absent is created, present is replaced, and the
    /// staging file does not outlive the write.
    #[test]
    fn set_writes_the_file_and_leaves_no_staging_behind() {
        let dir = std::env::temp_dir().join(format!("rivet-monitor-switch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, COMMENTED).unwrap();
        set(&path, Switch::Backups, "rivet-01", true).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(load(&written).backups_on("rivet-01"));
        assert!(!path.with_extension("toml.switching").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
