//! **The fleet register, read from the fleet server** — `SPEC-01` §4 and §5.2.
//!
//! `GET https://<server>/api/fleet`, presenting the operator's certificate.
//! What comes back is `rivet-fleetd`'s own rendering of `Rivet ADR-034`'s register:
//! every device the fleet was told about, its name, where it is, and — the part
//! that matters here — **its control-plane pin**, which is a public key and
//! costs nothing to publish (`Rivet ADR-034` §3.6).
//!
//! That single fetch is why nothing in this program's configuration lists a
//! machine. The register already knows them, `SPEC-01` §5.2 makes the menu a
//! picture of it, and `SPEC-01` §4.1's name — the one TLS verifies — comes from
//! here rather than from somebody typing it twice.

use std::path::Path;
use std::time::Duration;

/// How long the register fetch gets. It is a few kilobytes of JSON from a
/// machine on the same network; thirty seconds is "something is wrong".
const TIMEOUT: Duration = Duration::from_secs(30);

/// The largest register this will read.
const MAX_BODY: u64 = 4 * 1024 * 1024;

/// One machine, as the fleet knows it.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Device {
    /// `Rivet ADR-025` §4.1's stable identifier. Never changes.
    pub id: String,
    /// **The label the operator gave it**, and what the menu says (`SPEC-01`
    /// §5.3).
    pub name: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    /// **This machine's own control-plane public key.** It is what `--pin`
    /// takes, and it is a fact of the machine rather than of this program.
    #[serde(default)]
    pub pin: Option<String>,
    /// Where to dial. `None` when the register cannot place it.
    #[serde(default)]
    pub address: Option<String>,
    /// The register's own reason a device has no address, carried through
    /// rather than dropped: a machine that cannot be placed must appear saying
    /// so, or an operator sees one simply missing and has nothing to act on.
    #[serde(default)]
    pub unreachable: Option<String>,
    /// Where `rivet-mcpd` listens.
    pub port: u16,
}

/// The whole answer.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Register {
    #[serde(default)]
    pub fleet: FleetName,
    #[serde(default)]
    pub devices: Vec<Device>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct FleetName {
    #[serde(default)]
    pub name: Option<String>,
}

/// Split `host:port`, defaulting the port.
///
/// # Errors
///
/// A port that is not a number.
pub fn split_server(server: &str, default_port: u16) -> Result<(String, u16), String> {
    match server.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => {
            let port = port
                .parse::<u16>()
                .ok()
                .filter(|p| *p != 0)
                .ok_or_else(|| format!("fleet.server {server:?}: {port:?} is not a port"))?;
            Ok((host.to_string(), port))
        }
        _ => Ok((server.to_string(), default_port)),
    }
}

/// The name TLS verifies the fleet server against.
///
/// **An address is refused when no name was configured**, which is `SPEC-01`
/// §4.1 and `cargo rivet backup`'s own rule: a fleet certificate carries a DNS
/// name and an address matches none of them. Saying it here beats a handshake
/// failure about subject alternative names three layers down.
///
/// # Errors
///
/// An address with no `fleet.name` beside it.
pub fn verify_name(host: &str, configured: Option<&str>) -> Result<String, String> {
    if let Some(name) = configured {
        return Ok(name.to_string());
    }
    let numeric = host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[');
    if numeric {
        return Err(format!(
            "fleet.server names the address {host} and fleet.name is not set, so there is no \
             name to verify the fleet server's certificate against. A fleet certificate carries \
             a DNS name — `rivet-provision ca` issues `server-<name>.crt` for one — and an \
             address matches none of them. Set fleet.name to the machine's fleet name."
        ));
    }
    Ok(host.to_string())
}

/// **Whether a name this program did not choose may become a folder** —
/// `SPEC-01` §4.4.
///
/// A register name becomes a directory under the backup folder and under its
/// staging directory, which a pass empties and prunes; an application name
/// becomes a directory beneath it and a `--service` argument. Neither name is
/// this program's, so each is held to the one shape that cannot leave the
/// folder it is joined to or be read as an option: one Windows path component,
/// not `.` or `..`, with no separator, drive colon, wildcard or control
/// character, not a name Windows reserves for a device, not ending in a dot or
/// a space — Windows strips both, so `rivet-01.` would share `rivet-01`'s
/// directory and its retention — not beginning with `-`, not beginning with a
/// dot — this program's own staging directory, `.rivet-monitor-staging`, sits
/// beside the machines' folders — and not carrying `~` and a digit, the shape
/// of a Windows short name, which can name an existing directory under a second
/// spelling.
///
/// # Errors
///
/// The name, escaped, and which rule it breaks.
pub fn usable_name(name: &str) -> Result<(), String> {
    const FORBIDDEN: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
    let short_name = name
        .as_bytes()
        .windows(2)
        .any(|pair| pair[0] == b'~' && pair[1].is_ascii_digit());
    let why = if name.trim().is_empty() {
        "it is empty"
    } else if name == "." || name == ".." {
        "it names a directory relative to another"
    } else if name.starts_with('.') {
        "it begins with a dot, as this program's own staging directory does"
    } else if short_name {
        "it carries `~` and a digit, which Windows reads as a short name for another folder"
    } else if name.contains(FORBIDDEN) {
        "it carries a path separator, a drive colon or a wildcard"
    } else if name.chars().any(char::is_control) {
        "it carries a control character"
    } else if name.ends_with('.') || name.ends_with(' ') {
        "it ends in a dot or a space, which Windows strips"
    } else if name.starts_with('-') {
        "it begins with `-`, which a command line reads as an option"
    } else if reserved_by_windows(name) {
        "it is a name Windows reserves for a device"
    } else {
        return Ok(());
    };
    Err(format!("{name:?} cannot be used as a folder name: {why} (SPEC-01 §4.4)"))
}

/// `CON`, `PRN`, `AUX`, `NUL`, `CONIN$`, `CONOUT$`, and `COM` or `LPT` followed
/// by a digit or a superscript one, two or three — in any case and with any
/// extension: `nul.txt` is the device too.
fn reserved_by_windows(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end()
        .to_ascii_uppercase();
    let numbered = (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.chars().count() == 4
        && stem
            .chars()
            .nth(3)
            .is_some_and(|c| c.is_ascii_digit() || matches!(c, '¹' | '²' | '³'));
    numbered || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
}

/// **Read the register off the fleet server.**
///
/// # Errors
///
/// A name that does not resolve, a certificate this workstation does not hold,
/// a server that refuses the operator, or an answer that is not the register.
pub fn fetch(
    server: &str,
    configured_name: Option<&str>,
    authority: &Path,
    default_port: u16,
) -> Result<Register, String> {
    let (host, port) = split_server(server, default_port)?;
    let name = verify_name(&host, configured_name)?;
    let socket = crate::tls::resolve(&host, port)?;
    let tls = crate::tls::config(authority)?;
    let agent = crate::tls::agent_for(tls, socket, TIMEOUT);

    let url = format!("https://{name}:{port}/api/fleet");
    let mut response = agent
        .get(&url)
        .call()
        .map_err(|e| format!("get {url}: {e}"))?;
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_BODY)
        .read_to_string()
        .map_err(|e| format!("reading {url}: {e}"))?;

    parse(&body).map_err(|why| format!("{url}: {why}"))
}

/// Read the register's JSON.
///
/// # Errors
///
/// Anything that is not this shape, including `rivet-fleetd`'s own error
/// object — which arrives with HTTP 200 and would otherwise deserialise to a
/// fleet with no devices, i.e. **a failure that looks like an empty fleet**.
pub fn parse(body: &str) -> Result<Register, String> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("the answer is not JSON ({e}). It said:\n{body}"))?;
    if let Some(error) = value.get("error") {
        return Err(format!(
            "the fleet server refused: {}",
            error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(&error.to_string())
        ));
    }
    if value.get("devices").is_none() {
        return Err(format!(
            "the answer carries no `devices`, so it is not the register. An answer read as an \
             empty fleet would be a failure that looks like a fleet with nothing in it. It \
             said:\n{body}"
        ));
    }
    serde_json::from_value(value).map_err(|e| format!("the register does not read: {e}"))
}

impl Device {
    /// Everything needed to talk to this machine, or why it cannot be.
    ///
    /// # Errors
    ///
    /// A name [`usable_name`] refuses, no address, or no pin. **The name is
    /// checked here, first**, because every backup, health read and offers
    /// query reaches a machine through this and nothing else — so a name that
    /// could become a path outside the backup folder stops before any path is
    /// built from it. **A missing pin is a refusal and never a default**:
    /// connecting without one would mean trusting whatever answered at that
    /// address, which is the single failure `Rivet ADR-025` §2's trust model
    /// exists to prevent.
    pub fn target(&self) -> Result<crate::tool::Target, String> {
        usable_name(&self.name).map_err(|why| format!("the register's name for it is refused: {why}"))?;
        let address = self.address.clone().ok_or_else(|| {
            self.unreachable
                .clone()
                .unwrap_or_else(|| "the register gives it no address".to_string())
        })?;
        let pin = self.pin.clone().ok_or(
            "the register holds no pin for it, and connecting without one would mean trusting \
             whatever answered at that address (Rivet ADR-025 §2)",
        )?;
        Ok(crate::tool::Target {
            name: self.name.clone(),
            address,
            control_port: self.port,
            pin,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    const BODY: &str = r#"{"fleet":{"name":"home-fleet"},"devices":[
      {"id":"abc","name":"rivet-01","role":"server","location":"rack","pin":"aa","address":"192.168.50.10","unreachable":null,"port":7777},
      {"id":"def","name":"board","role":null,"location":null,"pin":null,"address":null,"unreachable":"no address recorded","port":7777}]}"#;

    #[test]
    fn the_register_reads_and_carries_the_pin_and_the_name() {
        let register = parse(BODY).expect("a register");
        assert_eq!(register.fleet.name.as_deref(), Some("home-fleet"));
        assert_eq!(register.devices.len(), 2);
        let first = register.devices[0].target().expect("reachable");
        assert_eq!(first.name, "rivet-01");
        assert_eq!(first.address, "192.168.50.10");
        assert_eq!(first.control_port, 7777);
        assert_eq!(first.pin, "aa");
    }

    /// A machine with no pin is refused, not defaulted.
    #[test]
    fn a_device_the_register_cannot_place_says_why() {
        let register = parse(BODY).expect("a register");
        let why = register.devices[1].target().expect_err("unplaceable");
        assert!(why.contains("no address recorded"), "{why}");
    }

    /// **`rivet-fleetd` answers its own errors with HTTP 200 and an `error`
    /// object**, which would otherwise read as a fleet with nothing in it.
    #[test]
    fn an_error_object_is_a_failure_and_not_an_empty_fleet() {
        let refused = parse(r#"{"error":{"code":-32001,"message":"no register"}}"#)
            .expect_err("an error object");
        assert!(refused.contains("no register"), "{refused}");
    }

    #[test]
    fn an_address_with_no_name_is_refused_and_says_what_to_set() {
        let refused = verify_name("192.168.50.10", None).expect_err("an address is not a name");
        assert!(refused.contains("fleet.name"), "{refused}");
        assert_eq!(
            verify_name("192.168.50.10", Some("rivet-01.local")).expect("named"),
            "rivet-01.local"
        );
    }

    /// **§4.4: a name that could leave the folder it is joined to is refused**,
    /// whichever rule it breaks — the review of 2026-10-11 deleted files
    /// outside a scratch backup folder with `..` and with an absolute name.
    #[test]
    fn a_name_that_could_leave_its_folder_is_refused() {
        for bad in [
            "", " ", ".", "..", "..\\x", "../x", "C:\\Windows", "C:", "\\\\server\\share",
            "a/b", "name:stream", "a*", "a?", "a\"b", "a<b", "a>b", "a|b", "tab\there",
            "line\nbreak", "rivet-01.", "rivet-01 ", "--into", "-x", "CON", "nul", "Aux.txt",
            "com1", "LPT9.log", "conin$", "CONOUT$.txt", "COM¹", "lpt³",
            ".rivet-monitor-staging", ".hidden", "LONGAP~1", "a~2b",
        ] {
            assert!(usable_name(bad).is_err(), "{bad:?} must be refused");
        }
    }

    /// …and the names a fleet actually uses are not.
    #[test]
    fn an_ordinary_name_is_usable() {
        for good in [
            "rivet-01", "dev-01", "rivet-08ecc", "tempmon-rivet", "a.b", "console", "com",
            "lpt10", "con-1", "Rivet 01", "a~b", "tilde~",
        ] {
            assert_eq!(usable_name(good), Ok(()), "{good:?}");
        }
    }

    /// **The refusal is in `target`, so it comes before any path.** Every
    /// backup, health read and offers query reaches a machine through it.
    #[test]
    fn a_register_name_that_is_not_usable_has_no_target() {
        let body = BODY.replace("\"name\":\"rivet-01\"", "\"name\":\"..\"");
        let register = parse(&body).expect("a register");
        let why = register.devices[0].target().expect_err("refused");
        assert!(why.contains("cannot be used as a folder name"), "{why}");
        assert!(why.contains("§4.4"), "{why}");
    }

    #[test]
    fn a_server_may_carry_its_port_or_take_the_default() {
        assert_eq!(
            split_server("rivet-01.local:4443", 4443).expect("valid"),
            ("rivet-01.local".to_string(), 4443)
        );
        assert_eq!(
            split_server("rivet-01.local", 4443).expect("valid"),
            ("rivet-01.local".to_string(), 4443)
        );
    }
}
