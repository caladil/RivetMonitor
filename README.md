# Rivet Monitor

**A Windows 11 taskbar application that operates a Rivet fleet on its owner's
behalf** — it watches the fleet's health and keeps the backups its applications
offer.

It is deliberately **not** part of [Rivet](https://github.com/caladil/Rivet).
Rivet defines what an application publishes; Rivet Monitor is one person's
opinion about what to do with it. If somebody else wants a different monitor,
they write one, and Rivet does not change.

---

## What it does

* **Keeps the backups.** Any application whose signed manifest declares a backup
  surface is fetched on a weekly schedule you write — `["Tue 04:00", "Fri
  04:00"]`, or `[]` for manual only — validated by Rivet's own tool, and kept
  to a retention policy. A scheduled time that passed while Rivet Monitor was
  not running is caught up on its next pass. **Backups get the data off the device**, which is the part a
  copy on the same machine never did.
* **Reads the fleet, and is not told about it.** The machines, their addresses
  and the key each one is pinned by all come from the fleet register. No machine
  is named in the configuration file.
* **Watches itself.** The icon shows how stale its own last successful backup
  is, because a tray icon that has gone quiet looks exactly like one with
  nothing to report.
* **Replaces a tray application per server** — the author's own TempMon and
  other applications each shipped one — with one that reads the fleet.

* **Says what CHANGED, once.** A condition appearing or disappearing is worth a
  word; a condition that is simply still there is not. TempMon's tray raised the
  same `403` on every poll for hours, and `SPEC-01` §5.5 exists so this program
  cannot. **Only the critical few become a Windows notification** — a watched
  machine not answering, a watched machine's critical application unhealthy,
  the fleet server unreachable, a backup overdue, the configuration file
  broken, and Rivet Monitor itself stopped — once each, after ten minutes.
  `rivet-monitor notify-preview` shows what they look like.

* **Reads the fleet's health.** Rivet's supervisor polls each application and
  folds the ones the machine calls critical into one word; Rivet Monitor reads
  that in the same pass and announces a change in it. It never decides whether a
  degraded application matters — that is the machine's own document, and the
  supervisor has already applied it (`SPEC-01` §5.4.1).

* **Keeps a flight recorder, if asked.** An optional `[monitor]` table captures
  a machine's status, log, screen and readings on their own cadences, onto the
  operator's drive — so a machine whose own storage failed can still be
  explained from somewhere else (`ADR-002`, `SPEC-02`). It needs its own
  `[monitor] folder` before it records anything; a machine's *Health
  monitoring* switch without one is a problem on the menu, not a quiet nothing.
  **If you keep several Fleets in Rivet's Armada Registry** (`~/.rivet/armada`),
  the flight recorder reads every listed Fleet's register, so it contacts fleet
  servers this program's own file does not name (`SPEC-02` §2.2). Backups use
  `fleet.server` alone.

## What it does NOT do

* **It does not define the contract.** The manifest keys, the fetch and the
  validation are Rivet's, in `cargo rivet backup`. Rivet Monitor schedules that
  and stores the result.
* **It does not decide whether a degraded application matters.** That is fleet
  policy, expressed in the machine's own document
  ([`Rivet ADR-045`](https://github.com/caladil/Rivet/blob/v0.1.137/docs/ADR-045-Health-and-Notifications.md) §4.3).
* **It does not run anywhere but Windows 11**, and does not pretend it might.

---

## Stack

| | |
| :--- | :--- |
| platform | Windows 11 |
| shape | notification-area (taskbar) application |
| language | Rust |
| libraries | [`caladil/Ignition`](https://github.com/caladil/Ignition) — configuration, structured logging, CLI surface |
| runs | `cargo-rivet` from [`caladil/Rivet`](https://github.com/caladil/Rivet), release `v0.1.137` — every fetch and validation is Rivet's |
| talks to | the fleet server (`rivet-fleetd`) over HTTPS, for the register; and each machine over [`Rivet ADR-035`](https://github.com/caladil/Rivet/blob/v0.1.137/docs/ADR-035-The-API-First-Control-Plane.md)'s control plane, through `cargo-rivet` |

**It holds two credentials, and no other** (`docs/SPEC-01` §4):

* **a control-plane identity** — what `cargo-rivet` presents to each machine to
  ask what it offers, read its health, fetch its backups and take monitor
  captures;
* **the fleet authority's operator certificate** — what reads the fleet
  register from the fleet server, and checks whether each machine's web proxy
  answers.

Neither can do the other's job. It holds no per-application token or password,
and an application that needs one to be backed up is a Rivet problem rather
than something worked around here.

---

## Layout

```
docs/               ADR / SPEC — the design record
src/
  main.rs           the command line, and the tray as the default
  config.rs         the configuration file and its refusals   (SPEC-01 §2, SPEC-02 §2)
  switches.rs       a machine's two switches, written in place (SPEC-01 §2.7)
  tool.rs           every call into cargo-rivet, and its proof (SPEC-01 §2.4, §3.5.1)
  register.rs       the fleet register, from the fleet server (SPEC-01 §4)
  armada.rs         the Armada Registry's list of Fleets
  tls.rs            presenting the operator certificate
  proxy.rs          which machines actually answer on 443     (SPEC-01 §5.2)
  ledger.rs         when each application last SUCCEEDED      (SPEC-01 §3.1)
  timetable.rs      weekly times, due, and late               (SPEC-01 §3.1)
  clock.rs          dates, and local time through Windows     (SPEC-01 §3.1)
  schedule.rs       one pass: due, fetch, place, retain       (SPEC-01 §3)
  icon.rs           what the icon says                        (SPEC-01 §5.4)
  alarm.rs          what CHANGED, and saying it once          (SPEC-01 §5.5)
  notify.rs         the Windows notification, for the few     (SPEC-01 §5.5.4)
  problems.rs       everything wrong right now, in plain words (SPEC-01 §5.8)
  tray.rs           the notification area and the menu        (SPEC-01 §5)
  monitor.rs        the flight recorder: one worker a machine (SPEC-02 §3)
  capture.rs        one capture, and what its outcome was     (SPEC-02 §4)
  store.rs          where a capture goes, how long it stays   (SPEC-02 §5)
  conditions.rs     the monitor's conditions                  (SPEC-02 §6)
  shell.rs          opening a URL or the config file
  autostart.rs      "Start with Windows"
```

A citation written `Rivet ADR-NNN` is to Rivet's own design record, read in
[its `docs/` at `v0.1.137`](https://github.com/caladil/Rivet/tree/v0.1.137/docs)
(`docs/ADR-003` §5).

---

## Getting started

### What you need first

* **Windows 11**, and **Rust 1.88 or newer** — built and tested on 1.95 — with
  the MSVC build tools (`rustup` installs Rust and offers to install the build
  tools).
* **A Rivet fleet** — machines running Rivet and a fleet server
  (`rivet-fleetd`) that holds their register.
  [Rivet's `SPEC-05`](https://github.com/caladil/Rivet/blob/v0.1.137/docs/SPEC-05-Provisioning-a-Fleet.md)
  is how a fleet is provisioned, and
  [its `COMMANDS.md`](https://github.com/caladil/Rivet/blob/v0.1.137/docs/COMMANDS.md)
  lists every tool used below.
* **The fleet authority**, in `~/.rivet/ca` — `rivet-provision ca` mints it,
  with your operator certificate, when the fleet is provisioned.
* **Your control-plane identity**, in `~/.rivet/identity/operator` —
  `cargo rivet setup` writes it, before any machine is provisioned, and
  provisioning puts its public key in each machine's allowed peers. A machine
  lets in no other key, so this is the directory `fleet.identity` names.

### Install and build

Rivet Monitor fetches and validates nothing itself; it runs Rivet's
`cargo-rivet`. Install that first, from Rivet's public repository:

```
cargo install --git https://github.com/caladil/Rivet --tag v0.1.137 --locked --force cargo-rivet
```

That is the release this build's checks were run against (`docs/SPEC-01` §2.4).
It lands in Cargo's bin directory, `%USERPROFILE%\.cargo\bin`. Then get and
build this program:

```
git clone https://github.com/caladil/RivetMonitor
cargo build --release --manifest-path RivetMonitor/Cargo.toml
```

The program is `RivetMonitor\target\release\rivet-monitor.exe`; put it wherever
you keep programs, and the commands below assume that place is on your `PATH`.

### Configure it

**Five settings are required and none has a default** — the backups folder,
the backup schedule, the path to a `cargo-rivet` that has the `backup` verb,
the fleet server, and the control-plane identity. Rivet Monitor names each
missing one and fetches nothing until they are all there. That is deliberate: a
backup tool that invents a destination is one whose backups are somewhere the
operator did not choose.

Create `%APPDATA%\RivetMonitor\config.toml` before the first run. The smallest
one that works, with your own folder, schedule and fleet server:

```toml
[backups]
folder   = 'E:\Backups'
schedule = ["Tue 04:00", "Fri 04:00"]   # or [] for Back Up Now only

[fleet]
server   = "rivet-01.local:4443"
identity = '~\.rivet\identity\operator'

[tool]
path = '~\.cargo\bin\cargo-rivet.exe'
```

A path may begin with `~`, which is your home folder; `%USERPROFILE%` is not
expanded. **Every machine starts with backups and health monitoring off** —
switch them on from its flyout in the menu. Health monitoring also needs a
`[monitor]` table with its own `folder`. `docs/SPEC-01` §2 and `docs/SPEC-02` §2
are the whole schema; `rivet-monitor check` names whatever is still missing,
and `View Config` in the menu opens the file.

Everything a pass decides is also reachable from the command line, which is how
it is tested; the per-machine switches and *Start with Windows* are the menu's
own:

```
rivet-monitor check           the config, the tool, the register, the overrides — fetches nothing
rivet-monitor run-once        one scheduled pass: fetch what is due, place it, apply retention
rivet-monitor back-up-now     one pass that fetches everything not exempt, whatever its schedule
rivet-monitor register        what the fleet server says the fleet is
rivet-monitor status          what has been backed up, when, and how stale that is
rivet-monitor notify-preview  raise each kind of Windows notification, contacting nothing
rivet-monitor monitor         the flight recorder alone, in the foreground; Ctrl+C stops it
rivet-monitor                 the tray icon
```

Add `--config PATH` to run any of them against another file.

---

## User guide

The icon sits in the notification area. Its menu:

```
⚠ rivet-01 isn't answering (12 min)   only while something is wrong
------------------------------------------------------------------
Fleet Portal            the fleet server's portal, in the system browser
------------------------------------------------------------------
rivet-01       ♥ 🡇  ▸   one flyout per fleet member:
dev-01              ▸     Open dev-01          its applications, via its proxy
                          -----------------
                          Health monitoring    (checkmark)
                          Backups              (checkmark)
------------------------------------------------------------------
Back Up Now             backs up everything now, manual-only included
View Config             opens the configuration file
View Log                opens the log being written
Start with Windows      (checkmark)
------------------------------------------------------------------
Quit
```

**The member list is derived from the fleet register**, not configured — a
machine joins the fleet and appears in the menu without anybody editing
anything. Its *Open* item is greyed if its proxy does not answer.

**Health monitoring and backups are both off for every machine until switched
on**, from its flyout or in the configuration file. The flyout writes the file
in place and keeps every comment in it. The marks after a machine's name show
what is on: ♥ health monitoring, 🡇 backups. A machine with both off is not
contacted over its control plane at all.

**When something is wrong, the menu says what, first.** One plain line per
problem, backup or health, with how long it has lasted; clicking one opens the
log. The icon is red while any of them stands, and clears itself when they do.

**The icon is the Rivet mark, and its colour is the monitor's own staleness.** A
tool that watches everything is the one thing nothing else watches, so the mark
is tinted by whether its backups are keeping up: green when nothing is late,
amber when any application has missed two scheduled times in a row or has never
been backed up, red when any problem is standing, and grey until the first pass
has finished. A manual-only application is never late. **Black means do not
believe the icon**: Rivet Monitor's own worker has stopped, so any colour it
still showed would be an old one.

The whole mark takes the colour rather than a badge in one corner. At the size a
taskbar draws an icon, the rivet at the centre is under five pixels across, so a
two-tone icon would hide the one thing that has to be read at a glance in the
part of the icon too small to read.

**What it writes on your PC**, besides its configuration, state and logs under
`%APPDATA%\RivetMonitor` (unless `logging.dir` puts the logs elsewhere) and the
backups and captures in the folders you name:
the per-user `Run` value for Rivet Monitor when *Start with Windows* is ticked,
and `HKCU\Software\Classes\AppUserModelId\Caladil.RivetMonitor` each time the
tray starts or `notify-preview` runs, which is how Windows shows its
notifications under its own name.
Nothing is written for every user, and nothing outside your own registry hive.

---

## License

Licensed under either of

* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
* MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option. Copyright © 2026 Joel Lindell.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this work shall be dual licensed as above, without any
additional terms or conditions.

The licence matches Rivet's, for Rivet's reason: Apache-2.0's explicit patent
grant removes an adoption barrier MIT leaves in place. `docs/ADR-003` §3.

---

## Acknowledgement

Built with **Claude Code** (Anthropic), models **Claude Opus 5** and **Claude
Opus 5.5**, working with Joel Lindell.
