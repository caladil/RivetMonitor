//! **One pass over the fleet** — `SPEC-01` §2.3 and §3.
//!
//! Read the register, ask each machine what its applications offer, decide
//! what is due, fetch it through `cargo rivet backup`, place what was due and
//! retire what is beyond retention.
//!
//! # The three rules this module exists to keep
//!
//! * **Due is "no SUCCESS since the most recent scheduled time"** (§3.1).
//!   `crate::ledger` holds the successes and nothing else; a failure writes
//!   nothing, and `crate::timetable` turns a schedule into instants.
//! * **Retention counts successes, and a failure rotates nothing** (§3.2).
//!   [`prune`] is called from the success path only — it is not reachable from
//!   a failure, which is the arithmetic that would turn three good copies into
//!   two.
//! * **An override that matches nothing is LOUD** (§2.3). It is the most
//!   dangerous line in the configuration file, because a rename turns a working
//!   override into silence.
//!
//! # The fetch is filtered, and the staging directory still matters
//!
//! `cargo rivet backup` gained a repeatable `--service NAME` filter in
//! `Rivet ADR-051` §8.5. [`due_here`] decides what is actually
//! due for one machine — cadence resolved, `exempt` and a feed already
//! excluded — and [`fetch_and_place`] passes exactly that list to
//! `crate::tool::fetch` as `--service`. An application not in it never has
//! its bytes cross the network, rather than crossing them and being
//! discarded once they arrive: `SPEC-01` §3.5's History has what this
//! replaced and why the fix had to be in Rivet rather than here.
//!
//! **The staging directory stays.** `cargo rivet backup` still chooses
//! nothing beyond `<service>/<file>` under the directory it is given, and
//! placement, naming and retention are still this module's, never that
//! verb's (`Rivet ADR-051` §8.2). Filtering the fetch makes staging hold only what
//! was actually asked for instead of a whole machine — smaller, not
//! unnecessary — and §3.6's rule is unchanged: a staged file is read back
//! off the filesystem, not out of the tool's report, because only a name
//! under that directory is one that passed the whole validation ladder.

use std::path::{Path, PathBuf};

use crate::config::{Config, Refusal};
use crate::ledger::{self, Ledger};
use crate::register::{Device, Register};
use crate::timetable;
use crate::tool::{self, Offer, Tool};

/// The staging directory's name, beside the destination.
///
/// **On the same volume as the backups on purpose**, so placing a file is a
/// rename rather than a copy of a database.
const STAGING: &str = ".rivet-monitor-staging";

/// One machine, as the menu needs it (`SPEC-01` §5.2, §5.3).
#[derive(Debug, Clone)]
pub struct Member {
    /// The register's label — what the menu says.
    pub label: String,
    /// The DNS name, which is what a link must carry.
    pub dns: String,
    /// `Some` only when the machine actually answered on 443.
    pub index_url: Option<String>,
}

/// **Something loud enough to change the icon**, and one day to reach the
/// desktop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    /// **A stable identity, deliberately not the message.**
    ///
    /// `SPEC-01` §5.5 keys announcements on this, so a message that gains a
    /// byte count, a timestamp or a reworded sentence does not read as a new
    /// problem. TempMon's tray raised the same `403` on every poll for hours,
    /// and this field is what makes that impossible here: one fact has one key
    /// however its wording changes.
    ///
    /// Shaped like a path — `fetch/rivet-01/tempmon-rivet` — so the thing it
    /// names is legible in a log without a lookup table.
    pub key: String,
    /// What a person is shown.
    pub message: String,
}

/// What one pass did and found.
#[derive(Debug, Default)]
pub struct Pass {
    /// Blocking: nothing was fetched and the icon says so.
    pub refusals: Vec<Refusal>,
    /// **Loud.** An unmatched override, a machine that would not answer, a
    /// fetch that failed. `SPEC-01` §2.3 puts these on the icon.
    pub alerts: Vec<Alert>,
    /// Worth reading and not an alarm.
    pub notes: Vec<String>,
    /// The fleet portal's URL, when a server is configured.
    pub portal_url: Option<String>,
    /// The menu's member list, derived from the register.
    pub members: Vec<Member>,
    /// Files placed by this pass.
    pub stored: Vec<PathBuf>,
    /// The most recent success anywhere, after this pass.
    pub newest_success: Option<u64>,
    /// The tool that was proved, for the report.
    pub tool_version: Option<String>,
    /// **Every check this pass actually RAN**, whether or not it found
    /// anything — `SPEC-01` §5.5.
    ///
    /// A pass that stops early because the register would not load has not
    /// examined a single application, and *"it did not complain"* is not the
    /// same statement as *"it looked and it was fine"*. Without this set, a
    /// monitor would announce that a problem had cleared when all that happened
    /// is that it stopped looking — which is the one lie that would make the
    /// whole program untrustworthy.
    pub evaluated: std::collections::BTreeSet<String>,
    /// **Key prefixes this pass enumerated exhaustively** — `SPEC-01` §5.5.
    ///
    /// [`Pass::evaluated`] answers *"was this exact thing checked?"*, and it
    /// cannot answer for a thing that no longer exists. Delete a bad override
    /// from the configuration and the check that raised the condition stops
    /// running, so the condition can never clear and the operator is never told
    /// his fix worked.
    ///
    /// A completed prefix says *"I looked at ALL of these, and what I did not
    /// report is not there"*. It is only ever declared where the pass genuinely
    /// enumerated the whole set.
    pub completed: std::collections::BTreeSet<String>,
    /// The applications this pass is responsible for keeping, and when each
    /// would be late. Recorded whether or not one was due, because lateness is
    /// judged after the pass.
    pub watched: Vec<Watched>,
    /// **What each machine says about its own health** — `Rivet ADR-045`.
    ///
    /// One entry per machine that answered. A machine that did not answer is
    /// absent rather than present-and-unknown, because *"it did not say"* and
    /// *"it said unknown"* are different facts and only the second is a
    /// statement about an application.
    pub health: Vec<(String, tool::Health)>,
}

/// An application Rivet Monitor keeps, and when it would be late.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watched {
    /// The register's name for the machine.
    pub host: String,
    /// The application's name on that machine.
    pub service: String,
    /// **A success at or after this instant, or the application is late** —
    /// the scheduled time before the most recent one, already resolved through
    /// any override so the alarm layer and the icon never re-derive a decision
    /// this pass has taken. `None` is manual only, which is never late.
    pub late_unless_since: Option<u64>,
}

impl Watched {
    /// **Is it late, given its last success?** — `SPEC-01` §5.5.3. The one
    /// rule the announcement and the icon both use, so they cannot disagree.
    #[must_use]
    pub fn late(&self, last_success: Option<u64>) -> bool {
        self.late_unless_since
            .is_some_and(|since| last_success.is_none_or(|then| then < since))
    }
}

impl Pass {
    /// Is anything wrong that the operator should see on the icon?
    #[must_use]
    pub fn healthy(&self) -> bool {
        self.refusals.is_empty() && self.alerts.is_empty()
    }

    /// Raise an alert under a stable key, and record that the check ran.
    fn alert(&mut self, key: impl Into<String>, message: String) {
        let key = key.into();
        self.evaluated.insert(key.clone());
        self.alerts.push(Alert { key, message });
    }

    /// **A check ran and found nothing wrong.**
    ///
    /// Called on the success path of everything [`Pass::alert`] is called on
    /// the failure path of. The pairing is the whole of §5.5's correctness: a
    /// key that appears in neither is a check that did not happen.
    fn ok(&mut self, key: impl Into<String>) {
        self.evaluated.insert(key.into());
    }
}

/// **A refused monitor is a standing problem** — `SPEC-01` §5.8.
///
/// Its refusals stop every capture, and until 2026-10-11 they went only to the
/// log, so a *Health monitoring* switch could read ✓ while nothing was
/// recorded — the exact trap `SPEC-01` §2.7 names. They are read here because
/// this pass reads the configuration every time, and enumerated like
/// `refusal/` so one the operator fixes clears. A key the backup refusals
/// already stand under — `tool.path`, `fleet.server`, `fleet.identity` — is
/// one problem, said once.
fn monitor_refusals(config: &Config, pass: &mut Pass) {
    let already: Vec<&str> = config.refusals().iter().map(|r| r.key).collect();
    for refusal in config.monitor_refusals() {
        if already.contains(&refusal.key) {
            continue;
        }
        pass.alert(
            format!("monitor-refusal/{}", refusal.key),
            format!("health monitoring records nothing: {}", refusal.why),
        );
    }
    pass.completed.insert(String::from("monitor-refusal/"));
}

/// Everything one pass needs, resolved once.
pub struct Context<'a> {
    pub config: &'a Config,
    pub ledger: &'a mut Ledger,
    pub now: u64,
    /// The clock a schedule is written in — this PC's, outside a test.
    pub zone: &'a dyn crate::clock::Zone,
    /// False for `check` and the menu refresh: read the fleet, fetch nothing.
    pub confirm: bool,
    /// **Back Up Now** (`SPEC-01` §3.7): everything a switched-on machine
    /// offers is due, whatever its schedule — manual only included. `exempt`
    /// and a feed are still never fetched.
    pub force: bool,
}

/// **Run a pass.**
///
/// It never returns `Err`: every failure is a refusal, an alert or a note, and
/// the caller shows all three. A pass that gave up on the first bad machine
/// would be a monitor that stopped monitoring.
pub fn run(context: &mut Context<'_>) -> Pass {
    let mut pass = Pass {
        refusals: context.config.refusals(),
        ..Pass::default()
    };
    // **A pass that is running is a configuration that loaded.** That is the
    // whole of the check, and it is what lets `alarm::CONFIG` clear — the
    // condition is raised by a caller that never reaches this function.
    pass.ok(crate::alarm::CONFIG);

    // Every refusal was decided by `Config::refusals`, which examines the whole
    // configuration whatever it finds — so a refusal the operator has FIXED is
    // one this pass can say is gone, and not one that quietly stands for ever
    // because the check that raised it stopped running.
    pass.completed.insert(String::from("refusal/"));
    monitor_refusals(context.config, &mut pass);

    let authority = context.config.authority();
    if let Some(server) = &context.config.fleet.server {
        pass.portal_url = portal_url(server, context.config.fleet.name.as_deref());
    }

    // **The tool is proved before anything else**, so a build that cannot
    // `backup` is one message rather than one per machine.
    let tool = match context.config.tool.path.as_deref() {
        Some(path) => match tool::preflight(&crate::config::expand_tilde(path)) {
            Ok(tool) => {
                pass.tool_version = Some(tool.version.clone());
                pass.ok("refusal/tool.path");
                Some(tool)
            }
            Err(why) => {
                pass.evaluated.insert(String::from("refusal/tool.path"));
                pass.refusals.push(Refusal {
                    key: "tool.path",
                    why,
                });
                None
            }
        },
        None => None,
    };

    // A server that is not configured is already a refusal; reporting it a
    // second time as an alert would be two spellings of one fact.
    let register = match context.config.fleet.server.as_deref() {
        None => None,
        Some(_) => match read_register(context.config, &authority) {
            Ok(register) => {
                pass.ok("register");
                Some(register)
            }
            Err(why) => {
                pass.alert("register", why);
                None
            }
        },
    };
    let Some(register) = register else {
        pass.newest_success = context.ledger.newest();
        return pass;
    };

    pass.members = members(context.config, &register, &authority);

    // Discovery runs whether or not anything will be fetched: §2.3's check
    // needs to know what each machine really offers, and an operator wants to
    // be told about a dead override before the next scheduled time comes round.
    let mut discovered: Vec<(Device, Result<Vec<Offer>, String>)> = Vec::new();
    if let Some(tool) = &tool {
        let identity = context
            .config
            .fleet
            .identity
            .as_deref()
            .map(crate::config::expand_tilde);
        if let Some(identity) = identity {
            let plan_only = std::env::temp_dir().join("rivet-monitor-plan-only");
            for device in &register.devices {
                // **A machine with both switches off is not contacted** —
                // `SPEC-01` §2.6. Its `offers/` and `health/` conditions are
                // marked evaluated-and-clear rather than skipped: a check that
                // simply stopped running would leave whatever it last raised
                // standing for ever, and a condition about a machine nobody is
                // watching is not one anybody can act on.
                if !context.config.backups_on(&device.name)
                    && !context.config.monitoring_on(&device.name)
                {
                    pass.ok(format!("offers/{}", device.name));
                    pass.ok(format!("health/{}", device.name));
                    continue;
                }
                let asked = match device.target() {
                    Err(why) => Err(format!("{}: {why}", device.name)),
                    Ok(mut target) => {
                        target.name = context.config.dns_name(&device.name);
                        tool::offers(tool, &target, &identity, &plan_only)
                    }
                };
                discovered.push((device.clone(), asked));
            }
        }
    }
    for (device, asked) in &discovered {
        match asked {
            Ok(_) => pass.ok(format!("offers/{}", device.name)),
            Err(why) => pass.alert(
                format!("offers/{}", device.name),
                format!("could not ask {} what it offers: {why}", device.name),
            ),
        }
    }

    // **How each machine is** — `Rivet ADR-045`. Read in the same pass and from
    // the same credential as everything else, because a second schedule for
    // health would be a second thing to go quiet.
    //
    // A machine that will not say is NOT recorded as unknown: `offers/<host>`
    // has already reported that it did not answer, and a second spelling of one
    // fact is what §5.5 exists to prevent.
    if let Some(tool) = &tool {
        let identity = context
            .config
            .fleet
            .identity
            .as_deref()
            .map(crate::config::expand_tilde);
        if let Some(identity) = identity {
            for (device, asked) in &discovered {
                if asked.is_err() {
                    continue;
                }
                // Asked because its BACKUP switch is on, but not watched: no
                // health read, and nothing it raised earlier stays standing.
                if !context.config.monitoring_on(&device.name) {
                    pass.ok(format!("health/{}", device.name));
                    continue;
                }
                let Ok(mut target) = device.target() else {
                    continue;
                };
                target.name = context.config.dns_name(&device.name);
                match tool::health(tool, &target, &identity) {
                    Ok(health) => pass.health.push((device.name.clone(), health)),
                    Err(why) => pass.notes.push(format!(
                        "{}: this build could not read `get status`, so its health is not \
                         reported: {why}",
                        device.name
                    )),
                }
            }
        }
    }

    check_overrides(context.config, &register, &discovered, &mut pass);
    // **Every override in the file was just examined**, so an override the
    // operator has DELETED is one this pass can say is gone. Without this the
    // condition it raised would stand for ever: the check that raised it stops
    // running the moment the line is removed, which is the moment the operator
    // most deserves to be told his fix worked.
    pass.completed.insert(String::from("override/"));

    if !pass.refusals.is_empty() {
        pass.newest_success = context.ledger.newest();
        return pass;
    }
    let (Some(tool), Some(identity)) = (
        tool.as_ref(),
        context
            .config
            .fleet
            .identity
            .as_deref()
            .map(crate::config::expand_tilde),
    ) else {
        pass.newest_success = context.ledger.newest();
        return pass;
    };

    // **One machine at a time** (§3.4). Nothing here is urgent, and a desktop
    // that has just woken should not open six large transfers at once.
    for (device, asked) in &discovered {
        let Ok(offers) = asked else { continue };
        // Asked because its MONITORING switch is on: nothing of it is fetched,
        // and nothing of it is judged late — `late/` below clears what was.
        if !context.config.backups_on(&device.name) {
            continue;
        }
        let due = due_here(context, device, offers, &mut pass);
        if due.is_empty() || !context.confirm {
            continue;
        }
        let Ok(mut target) = device.target() else {
            continue;
        };
        target.name = context.config.dns_name(&device.name);
        fetch_and_place(
            context,
            tool,
            &target,
            &device.name,
            &due,
            &identity,
            &mut pass,
        );
    }

    // **Every machine this program is responsible for was asked and answered**,
    // so the set of applications it is responsible for is completely known.
    // Only then can an application that has been removed from a machine — a
    // machine that has left the fleet, or one whose backup switch was turned
    // off (`SPEC-01` §2.6) — stop being reported overdue. If ONE switched-on
    // machine did not answer, this is not declared: a machine that is merely
    // unplugged must never read as its backups having recovered.
    //
    // **An empty set is a complete one.** This point is reached only once the
    // tool and the identity are both in hand, so discovery ran over every
    // switched-on machine; none being switched on means responsibility for
    // nothing, and anything still reported late must clear.
    if discovered.iter().all(|(_, asked)| asked.is_ok()) {
        pass.completed.insert(String::from("late/"));
    }

    pass.newest_success = context.ledger.newest();
    pass
}

/// The fleet server's own portal.
fn portal_url(server: &str, name: Option<&str>) -> Option<String> {
    let (host, port) = crate::register::split_server(server, 4443).ok()?;
    let name = name.unwrap_or(&host);
    Some(format!("https://{name}:{port}/"))
}

/// Read the register, saying which of the three configured things was wrong.
fn read_register(config: &Config, authority: &Path) -> Result<Register, String> {
    let server = config
        .fleet
        .server
        .as_deref()
        .ok_or("fleet.server is not configured")?;
    crate::register::fetch(server, config.fleet.name.as_deref(), authority, 4443)
        .map_err(|why| format!("the register could not be read: {why}"))
}

/// The menu's member list — **derived, and only where a proxy answers**.
fn members(config: &Config, register: &Register, authority: &Path) -> Vec<Member> {
    register
        .devices
        .iter()
        .map(|device| {
            let dns = config.dns_name(&device.name);
            let index_url = match &device.address {
                Some(address) if crate::proxy::answers(&dns, address, authority) => {
                    Some(crate::proxy::index_url(&dns))
                }
                _ => None,
            };
            Member {
                label: device.name.clone(),
                dns,
                index_url,
            }
        })
        .collect()
}

/// **§2.3 — an override that matches nothing is an ERROR, not a silence.**
///
/// The distinction that keeps this from crying wolf: an override whose machine
/// is in the register but could not be asked is **unverified**, not unmatched.
/// A host that is merely switched off must not produce the same message as a
/// host that was renamed, or the message stops meaning anything.
fn check_overrides(
    config: &Config,
    register: &Register,
    discovered: &[(Device, Result<Vec<Offer>, String>)],
    pass: &mut Pass,
) {
    let mut seen: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for over in &config.backups.overrides {
        *seen.entry(over.key()).or_default() += 1;

        // The register was read, so *"does this machine exist?"* is a question
        // this pass answered whichever way it came out.
        pass.ok(format!("override/{}/no-machine", over.key()));

        let device = register.devices.iter().find(|d| d.name == over.host);
        let Some(device) = device else {
            pass.alert(
                format!("override/{}/no-machine", over.key()),
                format!(
                    "the override for {} matches NO MACHINE: the register holds no device named \
                     {:?}. Machines get renamed, and an override that quietly matched nothing \
                     would be a backup that stopped without saying so (SPEC-01 §2.3). The \
                     register's names are: {}",
                    over.key(),
                    over.host,
                    names(register)
                ),
            );
            continue;
        };
        // **An override on a machine whose backups are switched off is inert**
        // (`SPEC-01` §2.6), not unverified: nothing it says is in force, so
        // there is no silent failure for it to cause and nothing to raise.
        if !config.backups_on(&device.name) {
            pass.ok(format!("override/{}/no-application", over.key()));
            pass.notes.push(format!(
                "the override for {} is inert: {}'s backups are switched off.",
                over.key(),
                device.name
            ));
            continue;
        }
        let asked = discovered
            .iter()
            .find(|(d, _)| d.id == device.id)
            .map(|(_, a)| a);
        match asked {
            Some(Ok(offers)) => {
                // **Only in this arm.** The other arm is the machine that did
                // not answer, and §2.3 is explicit that a machine merely
                // switched off leaves this question UNANSWERED rather than
                // answered in the negative.
                pass.ok(format!("override/{}/no-application", over.key()));
                if !offers.iter().any(|o| o.service == over.application) {
                    pass.alert(
                        format!("override/{}/no-application", over.key()),
                        format!(
                            "the override for {} matches NO APPLICATION: {} offers a backup for \
                             {}. An application that was renamed leaves an override matching \
                             nothing, and the failure is silence (SPEC-01 §2.3).",
                            over.key(),
                            device.name,
                            offered(offers)
                        ),
                    );
                }
            }
            _ => pass.notes.push(format!(
                "the override for {} could not be checked: {} did not answer, so this says \
                 nothing about whether the application still exists.",
                over.key(),
                device.name
            )),
        }
    }
    for (key, count) in seen {
        pass.ok(format!("override/{key}/duplicated"));
        if count > 1 {
            pass.alert(
                format!("override/{key}/duplicated"),
                format!(
                    "{count} overrides name {key}. Only the first applies, and the others are \
                     settings you believe are in force and are not."
                ),
            );
        }
    }
}

fn names(register: &Register) -> String {
    if register.devices.is_empty() {
        return "(the register is empty)".to_string();
    }
    register
        .devices
        .iter()
        .map(|d| d.name.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

fn offered(offers: &[Offer]) -> String {
    if offers.is_empty() {
        return "nothing at all".to_string();
    }
    offers
        .iter()
        .map(|o| o.service.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Which of this machine's applications are due right now.
///
/// **This is the exact list `fetch_and_place` later hands `crate::tool::fetch`
/// as `--service`.** `exempt` and a feed are excluded here, before the fetch
/// is ever asked for — not after a whole machine's bytes have already
/// crossed the network — which is what makes `exempt` mean *never fetched*
/// (`SPEC-01` §2.2, §3.5).
fn due_here(
    context: &Context<'_>,
    device: &Device,
    offers: &[Offer],
    pass: &mut Pass,
) -> Vec<String> {
    let mut due = Vec::new();
    for offer in offers {
        let Ok(resolved) = context.config.resolve(&device.name, &offer.service) else {
            continue;
        };
        if resolved.exempt {
            // **Excluded here, before `--service` is ever built** — this is
            // what makes the note true. Until 2026-09-09 this application
            // would still be fetched and only discarded afterwards, so the
            // note said "never kept" because that was the honest limit of
            // what this program controlled (SPEC-01 §3.5's History). Now
            // that `fetch_and_place` passes exactly `due` as `--service`, an
            // application excluded here is never named to the tool at all.
            pass.notes.push(format!(
                "{}/{}: exempt, so it is never fetched.",
                device.name, offer.service
            ));
            continue;
        }
        if offer.feed {
            pass.notes.push(format!(
                "{}/{}: declares a FEED, and `cargo rivet backup` reports a feed and does not \
                 fetch it (Rivet ADR-051 §8.4). Nothing here pretends otherwise.",
                device.name, offer.service
            ));
            continue;
        }
        // **Watched from here on**, whether or not it is due today. Lateness is
        // judged after the pass, and the alarm layer must not re-derive a
        // schedule this function has already resolved through the overrides.
        pass.watched.push(Watched {
            host: device.name.clone(),
            service: offer.service.clone(),
            late_unless_since: timetable::late_unless_since(
                &resolved.schedule,
                context.zone,
                context.now,
            ),
        });

        let last = context.ledger.last_success(&device.name, &offer.service);
        if context.force
            || timetable::due(&resolved.schedule, last, context.zone, context.now)
        {
            due.push(offer.service.clone());
        } else {
            let since = last.map_or_else(
                || "never backed up".to_string(),
                |last| format!("last success {}", ledger::ago(last, context.now)),
            );
            pass.notes.push(format!(
                "{}/{}: not due — {}, {since}.",
                device.name, offer.service, resolved.schedule
            ));
        }
    }
    due
}

/// Fetch one machine and place what was due.
fn fetch_and_place(
    context: &mut Context<'_>,
    tool: &Tool,
    target: &tool::Target,
    host: &str,
    due: &[String],
    identity: &Path,
    pass: &mut Pass,
) {
    let Some(root) = context.config.backups.folder.clone() else {
        return;
    };
    let root = crate::config::expand_tilde(&root);
    // **Checked again at the deletion itself** (`SPEC-01` §4.4). `Device::target`
    // has already refused a name like this, and this path is only reached
    // through it; but `reset` and `prune` delete, and a joined name that is
    // absolute or `..` would point them outside the backup folder entirely.
    if let Err(why) = crate::register::usable_name(host) {
        pass.alert(format!("staging/{host}"), why);
        return;
    }
    let staging = root.join(STAGING).join(host);
    if let Err(why) = reset(&staging) {
        pass.alert(format!("staging/{host}"), format!("{host}: {why}"));
        return;
    }
    pass.ok(format!("staging/{host}"));

    // **`due`, not the whole table.** `cargo rivet backup` fetches only the
    // named applications (`Rivet ADR-051` §8.5) — an `exempt` application, or one
    // simply not due yet, is never handed to it and its bytes never cross
    // the network. See the module note and `SPEC-01` §3.5.
    let outcome = tool::fetch(tool, target, identity, &staging, due);
    match &outcome {
        Ok(_) => pass.ok(format!("fetch/{host}")),
        // **Not a return.** `cargo rivet backup` does not stop at the first
        // failed application (`Rivet ADR-051` §8.3), so the ones that succeeded are
        // already in the staging directory and must still be placed.
        Err(why) => pass.alert(format!("fetch/{host}"), format!("{host}: {why}")),
    }

    let staged = match walk(&staging) {
        Ok(staged) => staged,
        Err(why) => {
            pass.alert(format!("staging/{host}"), format!("{host}: {why}"));
            return;
        }
    };
    for (service, file) in staged {
        if !due.contains(&service) {
            // **This should not happen any more.** The fetch just above was
            // called with `--service` naming exactly `due`, so a file under a
            // service that is not in it means the filter did not hold on
            // THIS run despite `tool::preflight` proving the binary supports
            // it in general — a version-skew edge `preflight` cannot see
            // (the binary at `tool.path` changing between the check and the
            // fetch) rather than the routine case this branch used to
            // handle. Still deleted rather than kept, for the reason it
            // always was — keeping it would make cadence and retention mean
            // nothing — and still noted rather than silently dropped,
            // because a note that never fires costs nothing and one that
            // fires unexpectedly is worth reading.
            let _ = std::fs::remove_file(&file);
            pass.notes.push(format!(
                "{host}/{service}: arrived even though --service did not name it. Discarded; \
                 this should not happen now that the fetch is filtered (SPEC-01 §3.5)."
            ));
            continue;
        }
        let Ok(resolved) = context.config.resolve(host, &service) else {
            continue;
        };
        match place(&file, &resolved.folder, host, &service, context.now) {
            Ok(destination) => {
                pass.ok(format!("place/{host}/{service}"));
                // **Only here.** Retention is pruned in the success path and
                // nowhere else, so a failed fetch rotates nothing (§3.2).
                match context.ledger.record(host, &service, context.now) {
                    Ok(()) => pass.ok(format!("ledger/{host}/{service}")),
                    Err(why) => pass.alert(
                        format!("ledger/{host}/{service}"),
                        format!("{host}/{service}: {why}"),
                    ),
                }
                match prune(
                    destination.parent().unwrap_or(&resolved.folder),
                    resolved.retention,
                ) {
                    Ok(retired) => {
                        pass.ok(format!("retention/{host}/{service}"));
                        for one in retired {
                            pass.notes
                                .push(format!("{host}/{service}: retired {}", one.display()));
                        }
                    }
                    Err(why) => pass.alert(
                        format!("retention/{host}/{service}"),
                        format!("{host}/{service}: {why}"),
                    ),
                }
                pass.stored.push(destination);
            }
            Err(why) => pass.alert(
                format!("place/{host}/{service}"),
                format!("{host}/{service}: {why}"),
            ),
        }
    }
    let _ = std::fs::remove_dir_all(&staging);
}

/// An empty staging directory, whatever was there before.
///
/// `cargo rivet backup` refuses to overwrite an existing file — that refusal is
/// a retention decision it does not make — so a leftover from an interrupted
/// run would stop the next one.
fn reset(staging: &Path) -> Result<(), String> {
    if staging.exists() {
        std::fs::remove_dir_all(staging)
            .map_err(|e| format!("cannot clear {}: {e}", staging.display()))?;
    }
    std::fs::create_dir_all(staging)
        .map_err(|e| format!("cannot create {}: {e}", staging.display()))
}

/// Every `<service>/<file>` under the staging directory.
///
/// **The filesystem is read rather than the tool's report parsed.**
/// `Rivet ADR-051` §8.3 makes the layout a contract — one directory per application,
/// and a name under it only ever means a file that passed the ladder, because
/// bytes are written to `<name>.partial` and renamed.
fn walk(staging: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir(staging)
        .map_err(|e| format!("cannot read {}: {e}", staging.display()))?;
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let Some(service) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let files = std::fs::read_dir(entry.path())
            .map_err(|e| format!("cannot read {}: {e}", entry.path().display()))?;
        for file in files.flatten() {
            let path = file.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) != Some("partial") {
                out.push((service.clone(), path));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// **Extensions Windows runs, installs, or follows as a shortcut** when a file is
/// opened — `SPEC-01` §4.4. A backup's file name is the application's, so this
/// is the one thing about it this program refuses: a backup folder the operator
/// browses should not hold something that runs when he opens it. A list, so a
/// promise only as good as the list; it covers the kinds Windows treats as
/// programs, installers, scripts and shortcuts.
const UNSAFE_EXTENSIONS: [&str; 43] = [
    "exe", "com", "scr", "pif", "cpl", "dll", "sys", "drv", "ocx", "msi", "msp", "mst", "msc",
    "msix", "appx", "appinstaller", "application", "appref-ms", "bat", "cmd", "ps1", "psm1",
    "vbs", "vbe", "js", "jse", "wsf", "wsh", "wsc", "hta", "jar", "lnk", "url", "website",
    "scf", "reg", "inf", "chm", "settingcontent-ms", "library-ms", "searchconnector-ms",
    "diagcab", "xll",
];

/// Whether this name **ends in** an extension on [`UNSAFE_EXTENSIONS`] — by
/// the text after its last dot, so a name that is nothing but an extension
/// (`.url`), which `Path::extension` reads as having none, is caught too.
fn windows_would_run(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, e)| UNSAFE_EXTENSIONS.iter().any(|u| u.eq_ignore_ascii_case(e)))
}

/// Move one staged file to `<folder>/<host>/<service>/<name>`.
///
/// A rename when the staging directory is on the same volume, a copy when an
/// override sent this application somewhere else.
///
/// **A name ending in an extension on [`UNSAFE_EXTENSIONS`] is never placed**
/// (`SPEC-01` §4.4); the staged copy goes with the staging directory at the
/// end of the pass.
fn place(
    staged: &Path,
    folder: &Path,
    host: &str,
    service: &str,
    now: u64,
) -> Result<PathBuf, String> {
    let name = staged
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("the staged file has no name")?;
    if windows_would_run(name) {
        return Err(format!(
            "the backup is named {name:?}, and Windows would run or follow a file with that \
             extension when it is opened, so it was not placed (SPEC-01 §4.4). An application's \
             backup has no reason to carry it."
        ));
    }
    let directory = folder.join(host).join(service);
    std::fs::create_dir_all(&directory)
        .map_err(|e| format!("cannot create {}: {e}", directory.display()))?;
    let mut destination = directory.join(name);
    if destination.exists() {
        // The service names its own backup, so an identical name means an
        // identical instant. Keep both rather than choose: losing one is not
        // this program's decision to make silently.
        let stem = destination
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(service)
            .to_string();
        let extension = destination
            .extension()
            .and_then(|s| s.to_str())
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        destination = directory.join(format!("{stem}-{now}{extension}"));
        if destination.exists() {
            return Err(format!(
                "{} already exists and so does the name this would fall back to, so nothing was \
                 placed.",
                directory.join(name).display()
            ));
        }
    }
    if std::fs::rename(staged, &destination).is_ok() {
        return Ok(destination);
    }
    std::fs::copy(staged, &destination)
        .map_err(|e| format!("cannot place {}: {e}", destination.display()))?;
    let _ = std::fs::remove_file(staged);
    Ok(destination)
}

/// **Keep the newest `retention` files and retire the rest** — §3.2.
///
/// Newest by modification time, which is when this program placed it. The
/// filenames sort chronologically when the service supplies them, and do not
/// when it does not, so the clock is the one answer that is right either way.
///
/// # Errors
///
/// A directory that cannot be read, or a file that cannot be removed.
pub fn prune(directory: &Path, retention: u32) -> Result<Vec<PathBuf>, String> {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(directory)
        .map_err(|e| format!("cannot read {}: {e}", directory.display()))?
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let modified = e.metadata().and_then(|m| m.modified()).ok()?;
            Some((modified, e.path()))
        })
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));

    let mut retired = Vec::new();
    for (_, path) in files.into_iter().skip(retention as usize) {
        std::fs::remove_file(&path)
            .map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
        retired.push(path);
    }
    Ok(retired)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("rivet-monitor-tests").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    fn touch(dir: &Path, name: &str, age: std::time::Duration) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, name.as_bytes()).expect("write");
        let when = std::time::SystemTime::now() - age;
        let file = std::fs::File::options()
            .write(true)
            .open(&path)
            .expect("open");
        file.set_modified(when).expect("set mtime");
        path
    }

    /// **Retention keeps the newest N and nothing else.**
    #[test]
    fn prune_keeps_the_newest_and_retires_the_rest() {
        let dir = scratch("prune");
        let oldest = touch(&dir, "a.db", std::time::Duration::from_secs(300));
        let middle = touch(&dir, "b.db", std::time::Duration::from_secs(200));
        let newest = touch(&dir, "c.db", std::time::Duration::from_secs(100));

        let retired = prune(&dir, 2).expect("prunes");
        assert_eq!(retired, vec![oldest.clone()]);
        assert!(!oldest.exists());
        assert!(middle.exists());
        assert!(newest.exists());
    }

    /// Fewer files than the retention is not a reason to delete anything.
    #[test]
    fn prune_retires_nothing_when_there_is_room() {
        let dir = scratch("prune-room");
        touch(&dir, "a.db", std::time::Duration::from_secs(100));
        assert!(prune(&dir, 3).expect("prunes").is_empty());
    }

    /// **The staging walk ignores `.partial`**, because a file under a real
    /// name is one that passed the ladder and a `.partial` is one that did not
    /// finish.
    #[test]
    fn walking_the_staging_directory_skips_partials() {
        let dir = scratch("walk");
        let service = dir.join("tempmon-rivet");
        std::fs::create_dir_all(&service).expect("mkdir");
        std::fs::write(service.join("good.db"), b"x").expect("write");
        std::fs::write(service.join("bad.db.partial"), b"x").expect("write");

        let found = walk(&dir).expect("walks");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "tempmon-rivet");
        assert_eq!(
            found[0].1.file_name().and_then(|n| n.to_str()),
            Some("good.db")
        );
    }

    /// Placing puts it under `<folder>/<host>/<service>/`, so two machines
    /// running the same application cannot collide.
    #[test]
    fn placing_separates_by_machine_and_application() {
        let dir = scratch("place");
        let staged = dir.join("staged.db");
        std::fs::write(&staged, b"x").expect("write");
        let destination =
            place(&staged, &dir.join("out"), "rivet-01", "tempmon-rivet", 7).expect("places");
        assert!(destination.ends_with("rivet-01/tempmon-rivet/staged.db"));
        assert!(destination.exists());
        assert!(!staged.exists(), "the staged copy is moved, not copied");
    }

    /// **§4.4: a backup named so that Windows would run it is never placed**,
    /// whatever its case; an ordinary data file is.
    #[test]
    fn a_backup_windows_would_run_is_not_placed() {
        let dir = scratch("place-unsafe");
        for name in ["backup.exe", "Backup.LNK", "x.url", "notes.ps1", "a.db.cmd", ".url", ".exe", "x.website"] {
            let staged = dir.join(name);
            std::fs::write(&staged, b"x").expect("write");
            let refused = place(&staged, &dir.join("out"), "h", "a", 1).expect_err(name);
            assert!(refused.contains("was not placed"), "{refused}");
            assert!(!dir.join("out").join("h").join("a").join(name).exists(), "{name}");
        }
        for name in ["backup.db", "export.json", "blobs.tar", "readings.csv", "noextension"] {
            assert!(!windows_would_run(name), "{name}");
        }
    }

    /// A second file of the same name keeps both rather than choosing.
    #[test]
    fn placing_over_an_existing_name_keeps_both() {
        let dir = scratch("place-twice");
        let out = dir.join("out");
        for _ in 0..2 {
            let staged = dir.join("same.db");
            std::fs::write(&staged, b"x").expect("write");
            place(&staged, &out, "h", "a", 99).expect("places");
        }
        let kept: Vec<_> = std::fs::read_dir(out.join("h").join("a"))
            .expect("read")
            .flatten()
            .collect();
        assert_eq!(kept.len(), 2, "both copies are kept");
    }

    // -----------------------------------------------------------------------
    // due_here — the list that becomes --service
    // -----------------------------------------------------------------------

    /// A register device with a real address and pin, so `device.target()`
    /// would succeed if anything here called it.
    fn device(name: &str) -> Device {
        Device {
            id: name.to_string(),
            name: name.to_string(),
            role: None,
            location: None,
            pin: Some("aa".to_string()),
            address: Some("192.168.50.10".to_string()),
            unreachable: None,
            port: 7777,
        }
    }

    fn cfg(text: &str) -> Config {
        toml::from_str(text).expect("valid test configuration")
    }

    /// **A refused monitor is a standing problem, and a fixed one clears** —
    /// the *Health monitoring* switch written for a machine before any
    /// `[monitor] folder` exists must not read ✓ while nothing is recorded.
    #[test]
    fn a_refused_monitor_is_a_standing_problem_until_it_is_fixed() {
        let switched_on = cfg(
            "[monitor]\n[[monitor.machine]]\nname = \"rivet-01\"\nenabled = true\n\
             [monitor.machine.status]\n",
        );
        let mut pass = Pass::default();
        monitor_refusals(&switched_on, &mut pass);
        let alert = pass
            .alerts
            .iter()
            .find(|a| a.key == "monitor-refusal/monitor.folder")
            .expect("raised");
        assert!(alert.message.contains("records nothing"), "{}", alert.message);
        assert!(pass.completed.contains("monitor-refusal/"));

        let fixed = cfg(
            "[monitor]\nfolder = \"E:/Monitor\"\n[[monitor.machine]]\nname = \"rivet-01\"\n\
             enabled = true\n[monitor.machine.status]\n",
        );
        let mut pass = Pass::default();
        monitor_refusals(&fixed, &mut pass);
        assert!(pass.alerts.is_empty(), "{:?}", pass.alerts);
        assert!(pass.completed.contains("monitor-refusal/"), "so the old one clears");

        let no_monitor = cfg("");
        let mut pass = Pass::default();
        monitor_refusals(&no_monitor, &mut pass);
        assert!(pass.alerts.is_empty(), "no [monitor] table is no monitor, not a problem");
    }

    /// **An exempt application never appears in what is due.** `due_here`'s
    /// answer is the exact list `fetch_and_place` hands `tool::fetch` as
    /// `--service` — so an exempt application missing from it here is the
    /// proof that `exempt` closes at the argument list this program builds,
    /// never after a fetch that already happened (`SPEC-01` §2.2, §3.5).
    #[test]
    fn an_exempt_application_never_appears_in_what_is_due() {
        let config = cfg(
            "[backups]\nfolder = \"D:/b\"\nschedule = [\"Fri 04:00\"]\n\n[[backups.override]]\n\
             host = \"rivet-01\"\napplication = \"archive-rivet\"\nexempt = true\n",
        );
        let mut ledger = Ledger::default();
        let device = device("rivet-01");
        let offers = vec![
            Offer {
                service: "tempmon-rivet".to_string(),
                feed: false,
            },
            Offer {
                service: "archive-rivet".to_string(),
                feed: false,
            },
        ];
        let context = Context {
            config: &config,
            ledger: &mut ledger,
            now: 1_000_000,
            zone: &crate::clock::Fixed(0),
            confirm: true,
            force: false,
        };
        let mut pass = Pass::default();

        let due = due_here(&context, &device, &offers, &mut pass);

        assert_eq!(due, vec!["tempmon-rivet".to_string()]);
        assert!(!due.contains(&"archive-rivet".to_string()), "{due:?}");
    }

    /// **Several due applications all appear**, in the order the plan gave
    /// them, and nothing about being more than one changes the answer.
    #[test]
    fn several_due_applications_are_all_in_what_is_due() {
        let config = cfg("[backups]\nfolder = \"D:/b\"\nschedule = [\"Fri 04:00\"]\n");
        let mut ledger = Ledger::default();
        let device = device("rivet-01");
        let offers = vec![
            Offer {
                service: "tempmon-rivet".to_string(),
                feed: false,
            },
            Offer {
                service: "notes-rivet".to_string(),
                feed: false,
            },
        ];
        let context = Context {
            config: &config,
            ledger: &mut ledger,
            now: 1_000_000,
            zone: &crate::clock::Fixed(0),
            confirm: true,
            force: false,
        };
        let mut pass = Pass::default();

        let due = due_here(&context, &device, &offers, &mut pass);

        assert_eq!(
            due,
            vec!["tempmon-rivet".to_string(), "notes-rivet".to_string()]
        );
    }

    /// **Nothing due is an EMPTY list, not an unfiltered one.** `run`'s own
    /// loop skips `fetch_and_place` entirely when this is empty (`if
    /// due.is_empty() ... continue`), so an empty answer here is what stops
    /// a whole machine being fetched to back up nothing at all — the same
    /// shape as `Rivet ADR-051` §8.5's no-`--service` identity case, but reached
    /// from the opposite direction: everything is offered and none of it is
    /// due yet.
    #[test]
    fn nothing_due_is_an_empty_list_not_an_unfiltered_one() {
        let config = cfg("[backups]\nfolder = \"D:/b\"\nschedule = [\"Fri 04:00\"]\n");
        let mut ledger = Ledger::default();
        // A success recorded a hundred seconds ago — 1970-01-12, a Monday —
        // after the most recent Friday 04:00.
        ledger
            .success
            .insert(ledger::key("rivet-01", "tempmon-rivet"), 1_000_000);
        let device = device("rivet-01");
        let offers = vec![Offer {
            service: "tempmon-rivet".to_string(),
            feed: false,
        }];
        let context = Context {
            config: &config,
            ledger: &mut ledger,
            now: 1_000_100,
            zone: &crate::clock::Fixed(0),
            confirm: true,
            force: false,
        };
        let mut pass = Pass::default();

        let due = due_here(&context, &device, &offers, &mut pass);

        assert!(due.is_empty(), "{due:?}");
    }

    /// A feed is excluded from what is due for the same reason `exempt` is:
    /// `cargo rivet backup` reports a feed and does not fetch it (`Rivet ADR-051`
    /// §8.4), so naming it with `--service` would ask for something that
    /// can never arrive.
    #[test]
    fn a_feed_is_never_in_what_is_due() {
        let config = cfg("[backups]\nfolder = \"D:/b\"\nschedule = [\"Fri 04:00\"]\n");
        let mut ledger = Ledger::default();
        let device = device("rivet-01");
        let offers = vec![Offer {
            service: "archive-rivet".to_string(),
            feed: true,
        }];
        let context = Context {
            config: &config,
            ledger: &mut ledger,
            now: 1_000_000,
            zone: &crate::clock::Fixed(0),
            confirm: true,
            force: false,
        };
        let mut pass = Pass::default();

        let due = due_here(&context, &device, &offers, &mut pass);

        assert!(due.is_empty(), "{due:?}");
    }

    /// The offers every manual-only and Back Up Now test below is asked about:
    /// one manual-only application, one exempt, one on the default schedule.
    fn manual_exempt_and_scheduled() -> (Config, Vec<Offer>) {
        let config = cfg(
            "[backups]\nfolder = \"D:/b\"\nschedule = [\"Fri 04:00\"]\n\n\
             [[backups.override]]\nhost = \"rivet-01\"\napplication = \"aura-rivet\"\n\
             schedule = []\n\n\
             [[backups.override]]\nhost = \"rivet-01\"\napplication = \"archive-rivet\"\n\
             exempt = true\n",
        );
        let offers = ["aura-rivet", "archive-rivet", "tempmon-rivet"]
            .into_iter()
            .map(|service| Offer {
                service: service.to_string(),
                feed: false,
            })
            .collect();
        (config, offers)
    }

    /// **Manual only is never due on its own, even never backed up — and is
    /// watched but never late** (`SPEC-01` §3.7).
    #[test]
    fn manual_only_is_watched_but_never_due_and_never_late() {
        let (config, offers) = manual_exempt_and_scheduled();
        let mut ledger = Ledger::default();
        let context = Context {
            config: &config,
            ledger: &mut ledger,
            now: 1_000_000,
            zone: &crate::clock::Fixed(0),
            confirm: true,
            force: false,
        };
        let mut pass = Pass::default();

        let due = due_here(&context, &device("rivet-01"), &offers, &mut pass);

        assert_eq!(due, vec!["tempmon-rivet".to_string()]);
        let aura = pass
            .watched
            .iter()
            .find(|w| w.service == "aura-rivet")
            .expect("manual only is still watched");
        assert!(!aura.late(None), "manual only is never late");
        let tempmon = pass.watched.iter().find(|w| w.service == "tempmon-rivet").unwrap();
        assert!(tempmon.late(None), "scheduled and never backed up is late");
    }

    /// **Back Up Now takes everything, manual only included — and still not
    /// what is exempt.** `exempt` means never fetched, and a button does not
    /// change what never means.
    #[test]
    fn back_up_now_takes_manual_only_but_never_exempt() {
        let (config, offers) = manual_exempt_and_scheduled();
        let mut ledger = Ledger::default();
        // Just backed up: nothing is due on its schedule.
        for service in ["aura-rivet", "tempmon-rivet"] {
            ledger.success.insert(ledger::key("rivet-01", service), 999_000);
        }
        let context = Context {
            config: &config,
            ledger: &mut ledger,
            now: 1_000_000,
            zone: &crate::clock::Fixed(0),
            confirm: true,
            force: true,
        };
        let mut pass = Pass::default();

        let due = due_here(&context, &device("rivet-01"), &offers, &mut pass);

        assert_eq!(due, vec!["aura-rivet".to_string(), "tempmon-rivet".to_string()]);
    }
}
