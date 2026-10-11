//! **Which machines actually run a proxy** — `SPEC-01` §5.2.
//!
//! The menu's member list is derived from the register, and every member gets a
//! flyout; its *Open* item is enabled only when the machine actually answers on
//! 443, because *a menu item that opens a refused connection is worse than no
//! item*. That is a fact about the machine right now, so it is **measured
//! rather than inferred** — the register's `role` is a human's one-word label,
//! and a machine can carry the word `server` with no `rivet-proxyd` installed.
//!
//! **Any HTTP answer counts, including 403.** `rivet-proxyd` gates routes on
//! the operator certificate (`Rivet ADR-046` §4), and a refusal is still proof that
//! something is listening and that the link will open a page rather than a
//! browser error.

use std::path::Path;
use std::time::Duration;

/// How long the probe gets.
///
/// **Short on purpose.** This runs once per member per poll and its answer
/// decides whether a menu item exists; a machine that has not answered in three
/// seconds on a home network is off.
const TIMEOUT: Duration = Duration::from_secs(3);

/// The port `rivet-proxyd` listens on — `Rivet ADR-046` §7.
pub const PROXY_PORT: u16 = 443;

/// Does this machine answer on its front door?
///
/// `name` is what TLS verifies; `address` is where the connection goes.
#[must_use]
pub fn answers(name: &str, address: &str, authority: &Path) -> bool {
    let Ok(socket) = crate::tls::resolve(address, PROXY_PORT) else {
        return false;
    };
    let Ok(tls) = crate::tls::config(authority) else {
        return false;
    };
    let agent = crate::tls::agent_for(tls, socket, TIMEOUT);
    match agent.get(&format!("https://{name}:{PROXY_PORT}/")).call() {
        Ok(_) => true,
        // **A status is an answer.** 403 means the proxy is there and gating,
        // which is precisely a page worth opening in a browser that holds the
        // certificate this program holds.
        Err(ureq::Error::StatusCode(_)) => true,
        Err(_) => false,
    }
}

/// The URL that machine's line opens — its `rivet-proxyd` index.
#[must_use]
pub fn index_url(name: &str) -> String {
    format!("https://{name}/")
}
