//! **The Armada Registry — the operator's list of Fleets** — Rivet `ADR-059`
//! §6.3, "Rivet Monitor".
//!
//! Rivet Monitor has so far monitored exactly one Fleet, named by `[fleet]
//! server`/`name` in its own configuration. An operator running more than one
//! Fleet keeps them listed once, centrally, at `~/.rivet/armada` — the same
//! directory the `armada` CLI reads and writes. This module is Rivet
//! Monitor's own read of that list: **`fleets/<name>.toml`**, one file per
//! Fleet, each carrying the Fleet's `name` and its server's `[server]`.
//!
//! **Every Fleet listed is read; none is acted on because it is listed.**
//! Which machines are watched or backed up is decided per machine by this
//! program's own switches (`SPEC-01` §2.6), and nowhere else. A Fleet file's
//! `monitor`/`backup` keys, which older Armada records still carry, mean
//! nothing here and are skipped with every other unread key.
//!
//! **`[server] host` is the CONTROL PLANE, not the portal.** Rivet writes the
//! address the operator reaches the server's control plane at — `:7777` — and
//! the register is served by `rivet-fleetd` on the portal port of the same
//! machine. [`Fleet::portal`] keeps the host and drops the port, so
//! `register::fetch` connects on the portal's default. Reading `host` as the
//! portal sent the register read to the control plane's TLS, which refused the
//! operator's certificate, and the flight recorder captured nothing until it
//! was found.
//!
//! **Stay decoupled.** `ADR-001` §3: Rivet Monitor lives outside Rivet.
//! This module defines its own small `Fleet` type and reads only the two
//! fields it needs — `name` and `server.host` — rather than depending on a
//! Rivet crate's schema. Every other key in a Fleet file (`server.pin`,
//! anything a later Rivet version adds) is read and
//! discarded: no `deny_unknown_fields` here, unlike `config.rs`. That
//! severity is right for **this program's own configuration**, which it owns
//! and which should refuse a typo; it is wrong for **another tool's
//! registry**, which this program only reads and must not break against.
//!
//! **`pin` is deliberately not read.** It is the control-plane's key
//! (`Rivet ADR-025` §2), and this module exists to reach a Fleet's *portal*
//! (`GET /api/fleet`, `register::fetch`), which is what verifies the
//! operator's browser certificate instead — `SPEC-01` §4.1. Reading `pin`
//! here and not using it would be a field this program appears to honour and
//! does not, the exact failure class `config.rs`'s module doc warns against.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::expand_tilde;

/// One Fleet in the operator's Armada Registry, as this program needs it:
/// enough to read that Fleet's own register (`register::fetch`).
///
/// **Not `armada.toml`'s settings** — only the list of Fleets and where each
/// one's server is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fleet {
    /// The Fleet's name, which is its server machine's register name. The
    /// portal's certificate is issued for that name with the fleet's DNS
    /// suffix — `config::Config::dns_name` joins them, exactly as it does for
    /// every machine in a register.
    pub name: String,
    /// `[server] host` as Rivet wrote it: the CONTROL PLANE's `HOST:PORT`, or a
    /// bare `<name>.local`. Never connected to as written — see [`Self::portal`].
    pub host: String,
}

impl Fleet {
    /// **Where this Fleet's portal is: the server's host, without the control
    /// plane's port**, so `register::fetch` connects on the portal's default.
    /// A bracketed IPv6 address keeps its brackets.
    #[must_use]
    pub fn portal(&self) -> String {
        let host = self.host.trim();
        if let Some(end) = host.find(']') {
            return host[..=end].to_string();
        }
        match host.rsplit_once(':') {
            Some((name, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
                name.to_string()
            }
            _ => host.to_string(),
        }
    }
}

/// One `fleets/<name>.toml`, read loosely (see the module doc): only `name`
/// and `server.host` are extracted, and no other key is refused for being
/// present or being unrecognised.
#[derive(Debug, Deserialize)]
struct FleetFile {
    name: String,
    server: ServerTable,
}

#[derive(Debug, Deserialize)]
struct ServerTable {
    host: String,
}

/// `~/.rivet/armada`, expanded the same way `config::expand_tilde` expands
/// every other path this program reads content from (`USERPROFILE`, then
/// `HOME`).
#[must_use]
pub fn default_dir() -> PathBuf {
    expand_tilde(Path::new("~/.rivet/armada"))
}

/// **Read every Fleet in the Armada Registry.**
///
/// `dir` overrides `~/.rivet/armada` — tests use it; production callers pass
/// `None`.
///
/// **Absent is empty, not an error.** No `~/.rivet/armada` directory, or one
/// with no `fleets/` subdirectory, or an empty `fleets/`, are all the same
/// case: an operator with a single Fleet and no Armada Registry at all. The
/// caller's job is then the fallback — `Supervisor::tick`'s single-fleet path
/// through `[fleet] server`/`name`, unchanged by this module's existing.
///
/// **A `fleets/*.toml` that will not parse is reported, not silently
/// dropped, and does not stop the others.** One malformed Fleet file must not
/// blind every other Fleet — the same reasoning the register's own
/// `unreachable` field carries for one unplaceable device rather than failing
/// the whole register (`register.rs`). The second element of the return is
/// one line per file that could not be read as a Fleet, `path: why`.
///
/// Fleets are returned **sorted by file name**, so the same registry always
/// produces the same order — tests rely on it, and a stable order is also
/// what makes "which Fleet did this name come from" reproducible when two
/// Fleets happen to share a device name.
#[must_use]
pub fn read(dir: Option<&Path>) -> (Vec<Fleet>, Vec<String>) {
    let armada = dir.map_or_else(default_dir, Path::to_path_buf);
    let fleets_dir = armada.join("fleets");
    let entries = match std::fs::read_dir(&fleets_dir) {
        Ok(entries) => entries,
        Err(_) => return (Vec::new(), Vec::new()),
    };

    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("toml")))
        .collect();
    paths.sort();

    let mut fleets = Vec::new();
    let mut problems = Vec::new();
    for path in paths {
        match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| toml::from_str::<FleetFile>(&text).map_err(|e| e.to_string()))
        {
            Ok(file) => fleets.push(Fleet {
                name: file.name,
                host: file.server.host,
            }),
            Err(why) => problems.push(format!("{}: {why}", path.display())),
        }
    }
    (fleets, problems)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("rivet-monitor-tests")
            .join("armada")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    fn write_fleet(dir: &Path, file: &str, text: &str) {
        std::fs::create_dir_all(dir.join("fleets")).expect("fleets dir");
        std::fs::write(dir.join("fleets").join(file), text).expect("a fleet file");
    }

    /// **Absent is empty, not an error** — no directory at all.
    #[test]
    fn an_absent_armada_directory_is_empty_and_not_an_error() {
        let dir = scratch("absent");
        std::fs::remove_dir_all(&dir).expect("remove the scratch dir itself");
        let (fleets, problems) = read(Some(&dir));
        assert!(fleets.is_empty());
        assert!(problems.is_empty());
    }

    /// **Absent is empty**: the directory exists but `fleets/` does not.
    #[test]
    fn an_armada_directory_with_no_fleets_subdirectory_is_empty() {
        let dir = scratch("no-fleets-subdir");
        let (fleets, problems) = read(Some(&dir));
        assert!(fleets.is_empty());
        assert!(problems.is_empty());
    }

    /// An empty `fleets/` is the same as no directory.
    #[test]
    fn an_empty_fleets_directory_is_empty() {
        let dir = scratch("empty-fleets");
        std::fs::create_dir_all(dir.join("fleets")).expect("fleets dir");
        let (fleets, problems) = read(Some(&dir));
        assert!(fleets.is_empty());
        assert!(problems.is_empty());
    }

    /// **A Fleet as the `armada` CLI wrote it reads, and its old switches mean
    /// nothing.** Hand-composed in the exact shape of a record `armada` wrote
    /// on a real workstation — top-level `monitor`/`backup`, `[server]`
    /// carrying the control-plane address, `pin` and `operator_key` — with the
    /// values replaced. Whether it says `false` or `true`, the Fleet is read
    /// the same way: this program's own per-machine switches decide.
    #[test]
    fn a_fleet_the_armada_cli_wrote_is_read_whatever_its_old_switches_say() {
        let dir = scratch("as-armada-writes-it");
        let record = "name = \"rivet-01\"\nmonitor = false\nbackup = false\n\n[server]\n\
                      host = \"192.0.2.10:7777\"\npin = \"aa\"\noperator_key = \"bb\"\n";
        for text in [record.to_string(), record.replace("= false", "= true")] {
            write_fleet(&dir, "rivet-01.toml", &text);
            let (fleets, problems) = read(Some(&dir));
            assert!(problems.is_empty(), "{problems:?}");
            assert_eq!(
                fleets,
                vec![Fleet { name: "rivet-01".to_string(), host: "192.0.2.10:7777".to_string() }]
            );
        }
    }

    /// **The portal is the server's host without the control plane's port.**
    /// Connecting to `[server] host` as written reached the control plane,
    /// whose TLS refused the operator's portal certificate.
    #[test]
    fn the_portal_drops_the_control_planes_port_and_keeps_the_host() {
        let fleet = |host: &str| Fleet { name: "rivet-01".to_string(), host: host.to_string() };
        assert_eq!(fleet("192.0.2.10:7777").portal(), "192.0.2.10");
        assert_eq!(fleet("rivet-01.local:7777").portal(), "rivet-01.local");
        assert_eq!(fleet("rivet-01.local").portal(), "rivet-01.local", "bare .local stays");
        assert_eq!(fleet("[2001:db8::1]:7777").portal(), "[2001:db8::1]");
        assert_eq!(fleet("[2001:db8::1]").portal(), "[2001:db8::1]");
    }

    /// **One Fleet file reads its name and host**; `pin`, `monitor` and
    /// `backup` are present and ignored, per the module doc.
    #[test]
    fn one_fleet_file_reads_name_and_host_and_ignores_the_rest() {
        let dir = scratch("one-fleet");
        write_fleet(
            &dir,
            "rivet-01.toml",
            "name = \"rivet-01\"\nmonitor = true\nbackup = true\n\
             [server]\nhost = \"rivet-01.local:4443\"\npin = \"aa\"\n",
        );
        let (fleets, problems) = read(Some(&dir));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            fleets,
            vec![Fleet {
                name: "rivet-01".to_string(),
                host: "rivet-01.local:4443".to_string(),
            }]
        );
    }

    /// **Several Fleets, sorted by file name.**
    #[test]
    fn several_fleets_are_returned_sorted_by_file_name() {
        let dir = scratch("several");
        write_fleet(&dir, "b-fleet.toml", "name = \"b\"\nmonitor = true\n[server]\nhost = \"b.local\"\n");
        write_fleet(&dir, "a-fleet.toml", "name = \"a\"\nmonitor = true\n[server]\nhost = \"a.local\"\n");
        let (fleets, problems) = read(Some(&dir));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            fleets.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }

    /// **A file that is not `.toml` is ignored.**
    #[test]
    fn a_non_toml_file_in_fleets_is_ignored() {
        let dir = scratch("non-toml");
        write_fleet(&dir, "readme.txt", "not a fleet");
        write_fleet(&dir, "rivet-01.toml", "name = \"rivet-01\"\nmonitor = true\n[server]\nhost = \"h\"\n");
        let (fleets, problems) = read(Some(&dir));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(fleets.len(), 1);
    }

    /// **A malformed Fleet file is reported and skipped, and does not blind
    /// the others** — one bad file must not cost every other Fleet.
    #[test]
    fn a_malformed_fleet_file_is_reported_and_does_not_block_the_others() {
        let dir = scratch("malformed");
        write_fleet(&dir, "broken.toml", "name = \"broken\"\n# no [server] table\n");
        write_fleet(&dir, "good.toml", "name = \"good\"\nmonitor = true\n[server]\nhost = \"good.local\"\n");
        let (fleets, problems) = read(Some(&dir));
        assert_eq!(fleets, vec![Fleet { name: "good".to_string(), host: "good.local".to_string() }]);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("broken.toml"), "{}", problems[0]);
    }

    /// **A Fleet file missing `name`** is reported the same way.
    #[test]
    fn a_fleet_file_missing_name_is_reported() {
        let dir = scratch("no-name");
        write_fleet(&dir, "x.toml", "[server]\nhost = \"h\"\n");
        let (fleets, problems) = read(Some(&dir));
        assert!(fleets.is_empty());
        assert_eq!(problems.len(), 1, "{problems:?}");
    }
}
