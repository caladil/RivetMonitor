# SPEC-01: Configuration, Scheduling, and the Menu

* **Status:** **Normative and BUILT.** §7 is the one place that says what is
  proved, by what method, and what is not. The configuration, the fetch, the
  register and the announcements first ran against the maintainer's live fleet
  on 2026-09-09; sections rewritten since — §3.1's weekly schedule on
  2026-10-02, §4 and §4.4 on 2026-10-11 — are proved only as far as §7 says.
* **Date:** 2026-09-08. **Built 2026-09-09.**
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Code:** `src/config.rs` (§2), `src/switches.rs` (§2.7), `src/schedule.rs`
  with `src/ledger.rs`, `src/timetable.rs` and `src/clock.rs` (§3),
  `src/tls.rs` with `src/register.rs` (§4), `src/tray.rs` with `src/proxy.rs`,
  `src/shell.rs` and `src/autostart.rs` (§5), `src/icon.rs` (§5.4),
  `src/alarm.rs` (§5.5; the monitor's record is `src/conditions.rs`, `SPEC-02`
  §6.7), `src/notify.rs` (§5.5.4), `src/problems.rs` (§5.8), `src/tool.rs`
  (§2.4's preflight and every call into `cargo rivet backup`)
* **Builds on:** `ADR-001` (why this program is separate), Rivet's backup
  contract ([`Rivet ADR-051`](https://github.com/caladil/Rivet/blob/v0.1.137/docs/ADR-051-The-Application-Backup-Contract.md))
  and `cargo rivet backup`

---

## 1. Scope

What Rivet Monitor reads, when it acts, what it keeps, and what its menu offers.

**It does not specify fetching or validating a backup.** Those are Rivet's, in
`cargo rivet backup`, and this document must never restate them — a second
statement of somebody else's rule is a rule that drifts.

---

## 2. The configuration file

TOML, read by Ignition, opened by the menu (§5).

```toml
[backups]
folder    = "E:\\Backups"        # REQUIRED — see §2.1
schedule  = ["Tue 04:00", "Fri 04:00"]   # REQUIRED — §3.1; [] is manual only
retention = 3                    # keep N successful backups  (default: 3)

# Per application, keyed on the machine's REGISTER NAME and the application
# name. §2.3: an override matching neither is an error.
[[backups.override]]
host        = "rivet-01"
application = "photos-rivet"
schedule    = ["Mon 21:00", "Tue 15:00", "Fri 01:00", "Sun 12:00"]
retention   = 14

[[backups.override]]
host        = "rivet-01"
application = "aura-rivet"
schedule    = []                 # manual only: Back Up Now, §3.7

[[backups.override]]
host        = "rivet-01"
application = "archive-rivet"
exempt      = true

# §2.6: which machines are backed up at all. OFF unless a block says `enabled`.
# The menu writes these (§2.7); an operator may too.
[[backups.machine]]
name    = "rivet-01"
enabled = true

[fleet]
server       = "rivet-01.local:4443"   # REQUIRED — rivet-fleetd, §4
identity     = "~/.rivet/identity/operator"  # REQUIRED — the control plane, §4
authority    = "~/.rivet/ca"           # the fleet authority, §4 (default: this)
host-suffix  = "local"                 # §2.5           (default: "local")
# name       = "rivet-01.local"        # only when `server` is an ADDRESS, §4.1
poll-seconds = 300                     # seconds between passes (default: 300)

[tool]
path = "…\\.cargo\\bin\\cargo-rivet.exe"   # REQUIRED — see §2.4

[logging]                              # Ignition's own, unchanged
enabled = true
level   = "info"
```

**Five keys are required and none of them has a default**: `backups.folder`
(§2.1), `backups.schedule` (§3.1), `tool.path` (§2.4), `fleet.server` and
`fleet.identity` (§4). Each missing one is named on its own line, with what it
is for, and **nothing is fetched while any of them is missing**.

**A key this build does not define is refused**, not ignored. A typo that was
quietly dropped is the same failure as §2.3's unmatched override: a setting the
operator believes is in force and is not.

### 2.1 Ignition's locations for ITSELF; the backup folder is REQUIRED

**Rivet Monitor is an Ignition application and uses Ignition's
platform-appropriate locations for its own configuration, logs and state.**
Ruled 2026-09-09. That is Ignition's job and this program does not second-guess
it.

**The backup folder is a different question and has no default.** No folder, no
backups, and the icon says so.

**Two reasons.**

A backup tool that invents a destination is one whose backups are somewhere the
operator did not choose, and he finds out when he needs them. That is Rivet's
own posture — refuse rather than guess.

And **every platform-appropriate default on Windows is on the system drive**,
while this folder holds databases, `.rvt` payloads and — once card images are
kept — **gigabytes**. The system drive is the one Windows, its updates and every
build cache share, and it is the drive most likely to be full; a default there
would put the copies that must survive trouble on the drive most likely to be
causing it.

#### 2.1.1 Where it is NOT written down

The maintainer's own destination is **his**, and it names where his fleet's data
lives. It belongs in his configuration file — which `.gitignore` excludes — and
**not in this document, the README, or any tracked file.**

### 2.2 An override may set `schedule`, `retention`, `folder` or `exempt`

Anything not overridden is inherited. An override's `schedule = []` is **manual
only** for that application, not *inherit* — an absent key is what inherits.
**An override's `retention = 0` is refused**, as `[backups]`'s is: retention
prunes right after a backup is placed and recorded, so a 0 would delete each
backup the moment it landed while reporting it a success. `exempt = true` is
how to keep none.

**A schedule time is a day and a 24-hour time, and nothing looser.** The day is
one of `Mon Tue Wed Thu Fri Sat Sun` (any case) and the time is `HH:MM` from
`00:00` to `23:59` — `"Tue 04:00"`. `"Tues 4am"`, `"Tue 4:00"` and `"Tue 24:00"`
are refused, and the configuration does not load, naming the entry and what
would have been accepted: a time read as something else is a backup at an hour
the operator did not choose, and a time quietly dropped is no backup.

**`cadence` is retired, and refused by name.** A file that still says
`cadence`, in `[backups]` or an override, loads — so the tray can say why — and
refuses every backup with a message saying `schedule` replaced it and how to
write one. It is not left to the unknown-key refusal, which would tell the
operator he had mistyped a key he had used correctly for a month.

`exempt = true` means **never fetched**
— what a very large or very uninteresting application gets. §3.5 says how:
the application is left off the `--service` list this program builds for the
machine it lives on, so its bytes never cross the network at all, rather than
crossing them and being discarded once they arrive. See the History for what
it meant before 2026-09-09 and why that was wrong.

### 2.3 An override that matches nothing is an ERROR, not a silence

**This is the most dangerous line in the file.** Overrides are keyed on a host
name and an application name, and both are renameable. A rename turns an
override into a line matching nothing — and the failure is *silence*: Rivet
Monitor either falls back to defaults or skips the application, with no
complaint.

**A backup that stops quietly is the worst outcome this program can produce.**
An unmatched override MUST be as loud as a failed fetch: in the log, on the
icon, and in whatever the menu shows.

**Two cases, and the message names which**: an override whose `host` is in no
register at all, and one whose `application` the named machine does not offer.
The first lists the names the register does hold; the second lists what that
machine offers.

**And a third case that must NOT be loud.** An override whose machine is in the
register but **could not be asked** is *unverified*, not unmatched — a machine
that is merely switched off must not produce the same message as one that was
renamed, or the message stops meaning anything.

**Two overrides naming one pair are also an error.** Only the first applies, and
the second is a setting the operator believes is in force and is not.

### 2.4 The path to `cargo rivet backup` is configuration, and it is PROVED

**`tool.path` is required and is deliberately not defaulted to the PATH.** The
`cargo-rivet` a workstation has installed is very often an older `cargo
install` — Rivet 0.1.66's, for one, has no `backup` verb at all. A program that
invoked it would be told *"`backup` is not a
`cargo rivet` verb"* on every run and could very reasonably report "nothing to
back up" — which is exactly the silent failure this document exists to prevent.

So at startup, and again whenever the configuration is re-read, Rivet Monitor
**runs the binary and makes it prove the verb**: `--version` for the record,
then `backup` with no arguments, which must print `backup`'s own usage. The
check is a **positive assertion** rather than the absence of an error string,
because an absence is also what a binary that changed its wording produces.

A binary that fails it is a refusal like a missing folder: named, with the
version it reported and the verbs it does have.

**Every refusal about the tool says how to get one that passes**, and it is the
same command everywhere — Rivet's public repository, pinned to the release this
build's proofs were run against:

```
cargo install --git https://github.com/caladil/Rivet --tag v0.1.137 --locked --force cargo-rivet
```

`--force` because whoever reads a refusal already has a `cargo-rivet`: the one
refused. It installs into Cargo's bin directory (`%USERPROFILE%\.cargo\bin`),
and `tool.path` names it there — and it is still proved like any other binary.
**The tag moves only when a build of this program has been proved against the
newer release** (§7), so a refusal never recommends a `cargo-rivet` nothing here
has run.

### 2.5 A register name is a label; TLS verifies a DNS name

The register holds `rivet-01`. The machine's certificate is issued for
`rivet-01.local`. `fleet.host-suffix` is what joins them, and a register label
that already contains a dot is taken as a full name and left alone.

### 2.6 Backups and health monitoring are switched per machine, and both are OFF by default

**Every machine in the register has two switches, and a machine nobody has
switched on is backed up by nothing and watched by nothing.** A development
board shares the fleet with the server whose data matters, and a fleet-wide
default would back up and alarm on the board as eagerly as on the server.

| switch | where it lives | what it gates |
| :--- | :--- | :--- |
| **backups** | `[[backups.machine]]` with `name` and `enabled` | whether the machine's applications are fetched at all (§3.5) |
| **health monitoring** | `enabled` on that machine's `[[monitor.machine]]` (`SPEC-02` §2.2) | the flight recorder's captures (`SPEC-02`), **and** the machine's health read in each pass with its announcements (§5.5) |

**`enabled` defaults to `false` on both**, and a machine with no block is a
machine whose switch is off. **Two `[[backups.machine]]` blocks for one machine
are refused**, as two overrides for one pair are (§2.3): whichever applied, the
other would be a switch the operator believes is in force and is not. `[backups] folder`, schedule, retention and every
`[[backups.override]]` are unchanged: they say **how** a switched-on machine is
backed up, and the switch says **whether**.

**Health monitoring is ONE switch over two mechanisms.** The flight recorder and
the pass's `get status` read both answer *how is this machine*, and a machine
whose recorder was off while its health still raised announcements would be
half-watched — which reads as fully watched to anybody who did not write the
configuration.

**A machine with both switches off is not contacted over the control plane.**
The pass does not ask it what it offers or how it is, so a development board
being unplugged is not an announcement. (The menu's quiet proxy probe still runs
— §5.2 — because it raises nothing.)
Its standing conditions — `offers/<host>`, `health/<host>`, and any
`late/<host>/…` — **clear** on the pass that sees the switch off, because a
condition about a machine nobody is watching is not a condition anybody can act
on. Its stored backups and captures are **kept**: switching a machine off stops
new work and deletes nothing.

**A backup switch naming a machine the register does not list fetches nothing,
and is not an error.** Unlike §2.3's override it carries no setting that is
silently not in force — the machine may simply have left the fleet for now, and
it keeps its switch for when it returns. (A monitoring switch that is ON for such
a machine is refused by the recorder, as `SPEC-02` §2.2 always has; one that is
off asks for nothing and is not.)

### 2.7 The menu writes the switches IN PLACE, and never loses a comment

**This file is written by two authors: the operator and the menu (§5.6).** A
switch flipped from the menu is an edit to this file, because a second place
holding *is this machine backed up* would be a second answer to one question.

**The edit is surgical.** One `enabled` value is set, and every other byte —
comments, key order, blank lines, the operator's own formatting — is left as it
was. A menu that re-serialised the whole file would silently delete every
comment an operator wrote to explain his own choices.

* **Switching ON** a machine with no block creates one: `name` and
  `enabled = true`, appended to the file. A `[[monitor.machine]]` so created
  carries one signal table, `status`, at its defaults (`SPEC-02` §2.3): a switch
  that reads *health monitoring ✓* and recorded nothing would be the quiet
  failure this program exists to catch, and `status` is the one signal that *is*
  health and costs almost nothing. Every heavier signal — the screenshot above
  all — stays an operator's addition. **A block needs a `[monitor] folder` to
  record anything** (`SPEC-02` §2.1), which no switch can choose; until there
  is one, the monitor refuses and that refusal stands on the problems list
  (§5.8) rather than leaving a checkmark that honours nothing.
* **Switching OFF sets `enabled = false`; it never deletes the block.** A block
  can carry an operator's signal settings and overrides, and a switch that threw
  them away on the way off would lose them on the way back on.
* **A file that will not parse is not written.** The switch refuses, says why in
  the log, and the file is left exactly as the operator left it.

The file is re-read every pass (`SPEC-02` §3.5 for the recorder), so a switch
takes effect on the next pass without a restart.

---

## 3. Scheduling

### 3.1 A schedule is weekly times; due is "nothing has succeeded since the last one"

**The operator says when, as zero or more days and times on this PC's own
clock** — `schedule = ["Tue 04:00", "Fri 04:00"]`. Any number, any days, any
times; `[]` is manual only (§3.7). There is no default (§2): a backup tool that
invents an hour is choosing for the operator.

**An application is due when no backup of it has succeeded since its most
recent scheduled time that has passed** — not *"when the clock says 04:00"*.
The pass runs every few minutes, and the first pass at or after the time is the
one that fetches. Rivet Monitor can be not running at the appointed minute for
reasons that have nothing to do with the PC, which it expects to stay on: a Windows
Update reboot, a restart of this program, a configuration it refused until the
operator fixed it. Measured this way, a time that passed while it was down is
caught up by the first pass after it comes back, and cannot be silently missed.

Three consequences, all deliberate:

* **Never backed up is due at once**, whatever the schedule. It is the riskiest
  case, and waiting days for the first time to come round would leave an
  application unprotected for exactly as long as it is newest.
* **Editing the schedule can make an application due at once.** Adding a time
  earlier this week than the last success is a time with no success since it.
* **A scheduled time is converted to an instant at its own date's offset**, by
  Windows (`TzSpecificLocalTimeToSystemTime`), not by subtracting today's
  offset. Across a daylight-saving change, a Friday 04:00 before it is the
  instant it really was; read at Monday's offset it would be an hour off, a
  Friday backup would look older than the time it answered, and it would be
  fetched again. On a spring-forward day a time inside the skipped hour is
  whatever Windows makes of it; on a fall-back day a time in the repeated hour
  is one instant, not two.

### 3.2 Retention counts successes, and a failure rotates nothing

`retention = 3` means three *successful* backups are kept. **A failed fetch must
not retire a good backup**, which is the arithmetic that turns three good copies
into two.

### 3.3 A backup is not "kept" until it has been validated

`cargo rivet backup` performs the validation; Rivet Monitor stores the result
only if that tool says it is sound, and counts only stored files toward
retention.

### 3.4 One application at a time

Fetches are serial. Nothing here is urgent, and a desktop that has just woken
should not open six large transfers at once.

### 3.5 Only what is due is FETCHED, and schedule and retention stay Rivet Monitor's

**`cargo rivet backup` takes a repeatable `--service NAME`.** For each machine,
Rivet Monitor works out which applications are actually due — schedule resolved
through any override, `exempt` and a declared feed already excluded — and
passes exactly that list as `--service`. An application not named never has its
bytes cross the network at all. Retention and schedule are still not that verb's
own (`Rivet ADR-051` §8.2); `--service` says only **which** applications one run
considers, never how often or how many are kept, and Rivet Monitor is still what
decides both.

Rivet Monitor still fetches into a **staging directory on the same volume as the
destination**, and still places only what was actually due before moving it to
where it belongs — filtering the fetch makes staging hold only what was asked
for instead of a whole machine, which makes it smaller, not unneeded. A file
that arrives staged for an application that was not asked for is deleted rather
than kept and is worth noticing, because with `--service` in force it should
never happen at all.

Staging is on the destination's volume so that placing a validated file is a
**rename**, not a second copy of a database; it is emptied before each machine's
run, because `cargo rivet backup` refuses to overwrite an existing file and a
leftover from an interrupted run would stop the next one.

### 3.5.1 The filter is PROVED before it is relied on

**`tool.path` is proved for `--service` the same way it is proved for the
`backup` verb itself** (§2.4) — a positive assertion that the flag's own line
appears in `backup`'s usage, checked once at startup and again whenever the
configuration is re-read, before a single pass schedules a fetch that depends
on it.

**A `cargo-rivet` that can `backup` but predates the filter is refused, not run
in a degraded mode.** It is the same failure §2.4 was written against, one flag
downstream: a build without `--service` would fetch every machine whole on
every pass, with nothing telling Rivet Monitor the filter never took hold — an
exempt application's data would keep crossing the network exactly as before
2026-09-09, silently, because the tool answered every call without complaint.
A degraded mode that fetched whole machines anyway would have to reproduce
today's fallback of discarding what was not due — the very defect this section
exists to close — dressed as a feature rather than reported as a refusal.

### 3.6 The staged file is read from the FILESYSTEM, not from the tool's report

`Rivet ADR-051` §8.3 makes the layout a contract — `<into>/<service>/<file>`,
written to `<name>.partial` and renamed — so a name under that directory only
ever means a file that passed the whole ladder. Reading the directory is
therefore stronger than parsing the report, and it does not break when the
report's wording changes.

### 3.7 Manual only, and Back Up Now

**`schedule = []` is manual only**: the application is never due on its own,
never judged late (§5.5.3), and is backed up only when the operator asks. It is
still discovered and still watched, so an override naming it is still checked
(§2.3) and a lateness it raised under an older schedule clears.

**Back Up Now is how he asks**, from the menu (§5) or as `rivet-monitor
back-up-now`. It runs one pass in which **every application on every machine
whose backups are switched on is due**, whatever its schedule — manual only
included. It changes nothing else: `exempt` and a declared feed are still never
fetched, because a button does not change what *never* means; a machine whose
switch is off is still not contacted (§2.6); and retention, placement and the
ledger are exactly a scheduled pass's.

From the menu, the request is a flag the worker reads before its next pass,
which it starts within a second rather than at the end of its interval. Two
clicks before that pass are one request. A click during a pass is the next
pass's request, not lost.

**The plan's text is still parsed**, because it is the only way to ask a machine
what it offers, and there §2.3's check needs the answer. That parse is
fail-closed: the plan's own count is read first and checked against the lines,
and a plan this build cannot read is an **error**, never an empty list —
because an empty list is indistinguishable from a machine that offers nothing.

---

## 4. Credentials

**Rivet Monitor holds TWO credentials, and they are not interchangeable.**

| | what it is, and where it comes from | what it opens |
| :--- | :--- | :--- |
| **the control-plane identity** | `~/.rivet/identity/operator` — `device.seed` and `device.crt`, an Ed25519 key (`Rivet ADR-025` §2), written by `cargo rivet setup`. Provisioning puts its public key in each machine's allowed peers, and a machine lets in no other (`Rivet SPEC-05`) | **everything said to a machine**: what its applications offer, its health, every backup fetched, and every monitor capture — all over `Rivet ADR-035`'s control plane, through `cargo rivet` |
| **the fleet authority** | `~/.rivet/ca` — `browser.crt`, `browser.key`, `root.crt`, minted by `rivet-provision ca` | **reading the fleet register** from the fleet server (§4.2), which is verified against `root.crt` and presented `browser.crt`; and checking whether each machine's web proxy answers on 443 (§5.2), which accepts a chain to the fleet authority where the machine still takes one (`Rivet ADR-046` §4.1) |

**Neither can do the other's job.** The control-plane identity is not a
certificate the fleet server or a proxy will accept, and the operator
certificate is not a key any machine's control plane will let in. A
configuration with only one of them can read the fleet and reach none of it, or
reach machines it cannot find.

### 4.1 And a NAME, which is neither

A fleet certificate carries a DNS name — `server-<name>.crt` — and an address
matches none of them. So reaching `192.168.50.10` means also saying
`rivet-01.local`, because that is what the certificate is verified against.

**Rivet Monitor takes it from the register** rather than asking, since the
register is already what tells it which machines exist.

### 4.2 The register supplies THREE facts about each machine, and no file lists them

`GET https://<server>/api/fleet` answers with each device's **name**, its
**address and control-plane port**, and its **pin** — the machine's own
control-plane public key, which `Rivet ADR-034` §3.6 publishes because it is what a
client *checks* rather than what it presents.

So the address, the pin and the name all come from one place, and **no machine
appears anywhere in Rivet Monitor's configuration**. A machine renamed or
re-addressed in the register is renamed and re-addressed here, with nothing
edited. The per-machine text in the file is an override's `host` and a switch's
`name` (§2.6) — register names, never addresses — and an override's is exactly
why §2.3 has to be loud.

**A device with no pin is refused, never defaulted.** Connecting without one
would mean trusting whatever answered at that address, which is the single
failure `Rivet ADR-025` §2's trust model exists to prevent.

**An error is not an empty fleet.** `rivet-fleetd` answers its own failures with
HTTP 200 and an `error` object; read carelessly, that deserialises to a register
with no devices — a failure that looks exactly like a fleet with nothing in it.
It is detected and reported as the refusal it is.

### 4.3 No per-application secret, ever

**It holds no token and no password, and does not hold, mint or store any
per-application secret.** An application that needs one to be backed up has a
backup surface that is wrong, and that is Rivet's problem rather than something
to work around here.

### 4.4 A name this program did not choose is held to one safe shape

**The register names each machine, and each machine names its applications.**
Both names become folders — `<folder>/<machine>/<application>/`, and the staging
directory beside it, which every pass empties and retention prunes — and an
application's name becomes a `--service` argument (§3.5). Neither is this
program's to choose, and a joined name that is absolute (`C:\…`, `\\server\…`)
or `..` points those deletions outside the backup folder entirely. The
pre-public review of 2026-10-11 demonstrated exactly that on a scratch folder.

So **a name is usable only as one Windows path component**: not empty, not `.`
or `..`; no `/ \ : * ? " < > |` and no control character; not a name Windows
reserves for a device (`CON`, `PRN`, `AUX`, `NUL`, `CONIN$`, `CONOUT$`, and
`COM` or `LPT` followed by a digit or a superscript `¹`, `²` or `³`, with any
extension); not ending in a dot or a space, because Windows strips both and
`rivet-01.` would share `rivet-01`'s directory and its retention; not
beginning with `-`, which a command line reads as an option; not beginning with
a dot, as this program's own staging directory, `.rivet-monitor-staging`, does
beside the machines' folders; and not carrying `~` followed by a digit, the
shape of a Windows short name, which can name an existing directory under a
second spelling.

* **A register name that fails is refused where the machine is reached** — the
  one door every backup, health read and offers query goes through — so no
  path is ever built from it, and the refusal is as loud as a machine that did
  not answer. It is checked again at the deletion itself.
* **A plan naming an application that fails is refused whole**, like any plan
  this build cannot use (§3.5.1's posture): a partly-trusted plan is not a plan.
  So is a plan naming two applications that differ only in case, compared by
  Unicode lowercase — NTFS gives `Photos` and `photos` one folder, and one's
  retention would prune the other's backups.
* **A backup's file name is the application's, and one ending in an extension
  on this program's list is never placed** — the extensions Windows runs,
  installs or follows as a shortcut when a file is opened (`.exe`, `.lnk`,
  `.url`, `.ps1`, `.msi` and their kind), read as the text after the last dot,
  so a name that is only an extension counts. A backup folder the operator
  browses should not hold something that runs when he opens it. The
  application's backup is reported as not stored (`place/<machine>/<application>`,
  §5.8) and the staged copy goes with the staging directory. It is a list, and
  only as good as the list.
* **The monitor's configured machine names** (`SPEC-02` §2.2) are held to the
  same shape, because they become folders under `monitor.folder`.
* **A machine whose name fails cannot be switched on from the menu** (§5.6):
  its two switches are greyed, and §2.7's writer refuses the name as well. A
  block written for it would only be refused, and a refused
  `[[monitor.machine]]` stops the whole monitor — so one bad name in the
  register could otherwise switch every machine's monitoring off.

---

## 5. The menu

**Ruled by the maintainer, 2026-09-08.**

```
Fleet Portal                      the fleet server's own portal, :4443
--------------------------------------------------
rivet-01          [pill]  ▸       one flyout per fleet member, §5.6
  │  Open rivet-01                  that machine's proxy index
  │  ────────────
  │  Health monitoring   (check)    §2.6
  │  Backups             (check)    §2.6
dev-01                    ▸
--------------------------------------------------
Back Up Now                       one pass that backs up everything, §3.7
View Config                       opens the configuration file
View Log                          opens the log being written
Start with Windows                (checkmark)
--------------------------------------------------
Quit
```

**View Log opens the newest file in Ignition's log directory**, or the directory
when it holds none. The tooltip sends the operator to the log whenever there is
a problem, so the menu has to say where the log is.

### 5.1 Two kinds of door, and they are not the same door

**Fleet Portal** is `rivet-fleetd` on `:4443` — machines, their status, and the
operations a fleet server offers. It is about the **fleet**, and there is exactly
one.

**Each machine's Open item** opens that machine's `rivet-proxyd` index at
`https://<host>/` — the applications routed there, each with the path it answers
on and whether it is gated. It is about **one machine's software**, and it is the
page wanted when the operator is going to *use* something rather than administer
it.

### 5.2 The list is DERIVED, not configured

**The members come from the register**, which Rivet Monitor already reads to know
what to back up. A machine added to the fleet appears in the menu without anybody
editing anything, and one removed stops appearing.

**Every member gets a flyout, proxy or not**, because every member has switches
(§2.6) and a headless machine is exactly the one an operator may want watched.
**Its Open item is disabled when no proxy answered**, rather than the machine
being left out: a disabled door says *there is nothing to open here*, where a
missing line would also hide the switches. An enabled item that opened a refused
connection would be worse than either.

**Whether a proxy answers is MEASURED, not inferred.** Each member is probed on
443 with the operator certificate once per pass; **any HTTP answer counts, 403
included**, because `rivet-proxyd` gates routes and a refusal is still proof that
something is listening. The register's `role` is a human's one-word label and a
machine can carry the word `server` with no proxy installed, so the label is not
evidence. **The probe runs whether or not the machine's switches are on**: it
raises nothing and decides only whether a door is open, and a development board
switched off for backups and monitoring may still be one whose applications the
operator opens. §2.6's *not contacted* is about the control plane —
discovery, health and fetching — not this.

**One member is not a special case** — it is the list with one flyout in it.
There is no shape to change when the fleet grows.

### 5.3 The label is the machine's name

Its register name, as the fleet knows it — `rivet-01`, not
`https://rivet-01.local/`. The operator named these machines; the menu should
say what he called them. The **link** carries the DNS name (§2.5), because a
browser needs one and a label is not one.

### 5.4 The icon is Rivet Monitor's own staleness

A tool that watches everything is the one thing nothing else watches. **The icon
shows its own staleness** — "last successful backup: 9 days ago" — because a
tray icon that has gone quiet looks exactly like one with nothing to report.

Five states. A stopped worker outranks everything, and a refusal outranks an
age:

| state | when |
| :--- | :--- |
| **stopped** (black) | this program's own backup worker has published nothing for three poll intervals. Every other colour would then be an old judgement shown as a live one, so the mark goes dark rather than keep a colour that cannot change. Decided by the tray's event loop, which runs whether or not the worker does — never by a pass |
| **refused** (red) | anything is standing in either record (§5.8) — a backup problem or a health problem |
| **stale** (amber) | any application this program keeps is **late** by §5.5.3's rule — the same rule the announcement uses, so the two cannot disagree |
| **current** (green) | nothing late and nothing complaining |
| **unknown** (grey) | the first pass has not finished |

**A scheduled application never backed up is `stale`, not `current`.** It is
late from the first pass that sees it (§5.5.3), and a quiet icon and a working
one must not look the same, which is the whole reason this section exists. A
manual-only application is never late, so a fleet that is entirely manual only
is green: nothing is owed until the operator asks.

#### 5.4.1 Health is READ, and it colours the icon like any other problem

**Rivet answers the question.** `Rivet ADR-045`'s first hop: the supervisor polls each
application on a handed descriptor, folds the `critical` ones into a machine
word, and `get status` reports both. Rivet Monitor reads it in the same pass,
from the same credential, and §5.5 announces a change in it.

**One icon, one list, one meaning: something here needs you.** A health problem
— the machine's own word, or any of the monitor's conditions in `SPEC-02` §6 —
turns the icon red exactly as a backup problem does, and is named on the same
list (§5.8). The remedy differs, and the list is what says which; the icon's job
is only to be noticed. A health problem that stayed in the log while the icon
stayed green is a problem nobody sees, which is the failure this program exists
to prevent.

**Rivet Monitor never decides whether a degraded application matters.**
`Rivet ADR-045` §4.3 puts that in the machine's own document, as `critical`, and the
supervisor has already applied it before this program sees a word. A monitor
that raised a condition per degraded service would be overruling the fleet's own
policy from outside — and would be back to an author's opinion of their own
importance, which is what the vocabulary took away.

---

### 5.5 Announcements: a CHANGE, never a state

**A monitor announces what changed. It never announces what is.**

TempMon's tray raised a Windows notification on every poll saying that
`https://rivet-01.local/tempmon/api/v1/backup` answered `403`. One fact,
hundreds of notifications, over hours.
That is what announcing a **state** does, and this section exists so that this
program cannot do it.

A **condition** is something wrong, identified by a stable key shaped like a
path — `fetch/rivet-01`, `late/rivet-01/tempmon-rivet`,
`override/rivet-01/photos-rivet/no-machine`. The key is deliberately not the
message: a message that gains a byte count or a reworded sentence must not read
as a new problem.

An **announcement** is a condition appearing or disappearing, and there is
exactly one per transition. The set of conditions already announced is persisted
so a restart repeats nothing: the backup pass's in `state.toml` beside the
ledger, and the monitor's in `monitor-state.toml` beside that. **Each record has
one writer** — the backup pass rewrites `state.toml` whole, and a second thread
writing into it would lose updates — and the two key sets are disjoint, so
neither pass can clear the other's conditions (`SPEC-02` §6.7).

#### 5.5.1 A condition that was not looked at has NOT cleared

This is the rule that matters, and it is worth being blunt about why: a pass
that stops early — the register would not load, the machine is off, the tool is
gone — **has not examined a single application**. *"It did not complain"* is not
the same statement as *"it looked and it was fine"*. A monitor that said
**resolved** because it had stopped looking would be lying at the exact moment
the operator most needs it not to.

So a pass records every check it actually ran, and a standing condition clears
**only** when its own check ran and found nothing. A condition whose check did
not run stays standing, silently, until something can say either way.

#### 5.5.2 And a condition whose SUBJECT was deleted must still clear

The operator removes the bad override from his configuration file. The check
that raised the condition never runs again, so §5.5.1 alone would leave it
standing for ever — invisible, and worse, a re-added override would raise
nothing because the key was still there.

A pass therefore also records **prefixes it enumerated exhaustively**:
`override/` whenever every override in the file was examined, and `late/`
whenever the whole fleet was listed **and every machine answered**. A standing
key under an enumerated prefix and not reported is gone.

**`late/` is not declared when one machine was merely switched off**, because a
machine that is off must never read as its backups having recovered.

#### 5.5.3 What is a condition

| key | raised when |
| :--- | :--- |
| `refusal/<key>` | a required setting is missing or the tool cannot `backup` — nothing is being fetched at all |
| `register` | the fleet register could not be read |
| `offers/<host>` | a machine would not say what it offers |
| `staging/<host>`, `fetch/<host>` | the staging directory or the fetch failed |
| `place/<host>/<app>`, `ledger/…`, `retention/…` | a file arrived and could not be stored, recorded, or rotated |
| `override/<host>/<app>/…` | §2.3's three refusals |
| `late/<host>/<app>` | no successful backup since the scheduled time **before** the most recent one, or none ever — never for manual only |
| `health/<host>` | the machine's own health word is not `healthy` — `Rivet ADR-045` §4 |
| `reach/`, `log-stalled/`, `card-stalled/`, `frozen/`, `power/`, `capture/` | the monitor's, when a `[monitor]` table is configured — `SPEC-02` §6 |

**Health is one condition per MACHINE, never one per service.** The machine's
document decides which services reach the fold, the supervisor has already
applied it, and the message names the critical services and carries their
captions verbatim. A machine that did not answer is absent from the reading
rather than present as `unknown`: *"it did not say"* and *"it said unknown"* are
different facts, and only the second is a statement about an application.

**Late is two scheduled times in a row passing without a success**, and the icon
uses the same rule, so the icon and the announcement can never disagree. One
missed time is a reboot at the wrong minute or a machine briefly off; two is a
backup that has stopped. With `["Tue 04:00", "Fri 04:00"]`, a Friday that fails
is not late; it is late from the following Tuesday 04:00 unless Tuesday's
succeeds in that same pass, because lateness is read after the pass.

**Manual only is never late** — nothing is owed until the operator asks — and
is still evaluated, so a lateness raised under an older schedule clears.

**A scheduled application never backed up is late from the first pass that
sees it.** It is the case
most likely to be a misconfiguration, and the case a quiet monitor would hide
for the longest.

**The recovery is written from the key, not from the failure's own words.** The
raised message is not kept: a standing record holding prose would be a second
copy of a sentence that changes with the build. So a cleared condition says what
is true again — *"rivet-01/tempmon-rivet has been backed up again"* — and how
long it had been that way.

#### 5.5.3a The tray must appear even when the configuration will not load

**A monitor that vanishes when something is wrong is the opposite of a
monitor.** Until 2026-09-09 a configuration file that would not parse stopped
`tray::run` before the icon was created: the operator mistyped a key, logged in,
and there was **no icon at all** — nothing to click, nothing to read, and no way
to reach `View Config` and fix it.

The tray now comes up **red, with the reason in its tooltip**, and the menu
still opens the configuration file. `config` is a condition like any other, and
it is the only one raised outside a pass — because it is the one that stops a
pass from happening.

A pass that runs at all is a configuration that loaded, which is the whole of
the check that clears it. And a pass that could not load one **retries in thirty
seconds rather than the five-minute default**: the operator who broke the file is
most likely editing it right now.

#### 5.5.4 A Windows notification, for the critical few, once

Announcements are produced, settled and logged for every condition. **A Windows
notification interrupts only for the kinds the maintainer named critical**
(2026-09-23):

| kind | key |
| :--- | :--- |
| a watched machine stops answering | `reach/<host>` |
| a watched machine says a critical application is unhealthy | `health/<host>` |
| the fleet server cannot be reached | `register` |
| a backup is overdue | `late/<host>/<app>` |
| the configuration file is broken | `config` |
| Rivet Monitor itself has stopped checking | the tray's own `Stopped` judgement (§5.4) |

**The rules, which are what keep a notification from becoming noise:**

* **Once per problem.** A problem is its key and when it began. It notifies
  once, and never again while it stands, however many checks see it.
* **It has to last ten minutes.** A board rebooted on purpose is not answering
  for under a minute and does not interrupt anybody. This program's own
  stoppage is held to the same ten: the icon goes black at three poll intervals
  (§5.4), which at a short cadence is well under ten minutes, and the
  notification waits until the silence has lasted them.
* **It survives restarts.** What has been notified is kept in `notified.toml`
  beside the configuration file, so restarting this program repeats nothing. A
  problem that clears and comes back is a new beginning and notifies again.
* **No notification when a problem clears.** It leaves the menu quietly.
* **Everything else stays in the menu** (§5.8) and on the icon, and never pops
  up.

Delivered by `tauri-winrt-notification` as a real Windows 11 notification, sent
as `Caladil.RivetMonitor` — an AppUserModelID registered under the current
user's own key each time the tray starts or `notify-preview` runs, so the
sender reads "Rivet Monitor". Clicking one
opens the log. `rivet-monitor notify-preview` shows one of each, a few seconds
apart, and contacts nothing.

`run-once` prints announcements under `CHANGED SINCE THE LAST PASS`, which is how
the whole layer is exercised by hand.

**Only a pass that ACTED settles the record.** `check` fetches nothing and runs
fewer checks; letting it settle would let a diagnostic the operator typed consume
the announcement the working pass was about to make. The monitor has no passes,
and its act is one capture attempt actually made, settling only what that attempt
checked — `SPEC-02` §6.8.

### 5.6 Each machine is a flyout: its door, then its two switches

A machine's line opens a flyout holding, in this order:

1. **Open `<name>`** — the machine's proxy index (§5.1), disabled when no proxy
   answered (§5.2).
2. a separator;
3. **Health monitoring** — a checkmark, §2.6's monitoring switch;
4. **Backups** — a checkmark, §2.6's backup switch.

**A checkmark shows what the FILE says**, read when the menu is built, never a
remembered click. The file is the one authority (§2.7), so a switch the operator
edited by hand shows the same as one flipped here.

**Clicking one writes the file (§2.7) and rebuilds the menu at once**, so the
checkmark the operator sees next is read back from what was written — a write
that failed shows the old state rather than a checkmark nothing honours. The
work itself changes on the next pass.

### 5.7 The pill: which switches are on, beside the machine's name

**When either switch is on, the machine's line carries a mark per switch that
is on**, right-aligned in the line — `♥` for health monitoring, `🡇` for
backups. A machine with both off carries none, so the list reads at a glance as
*which machines this program is looking after*.

It is a convenience and never the record: the flyout's checkmarks (§5.6) are
the same facts, read from the same place. A native menu line is text, so the
pill is two characters in the menu's accelerator column rather than a drawn
shape.

---

### 5.8 The problems list: what is wrong, first, in plain words

**When anything is standing, the menu opens with it**: one line per problem,
newest first, above the fleet portal, then a rule. No lines is the menu saying
nothing is wrong. Past eight, one more line counts the rest.

```
⚠ rivet-01 isn't answering (12 min)
⚠ notes-rivet on dev-01 is overdue for a backup (2 d)
----------------------------------------
Fleet Portal
```

**Both records are read**: `state.toml`'s standing conditions and the monitor's
`monitor-state.toml` (`SPEC-02` §6.7). Neither holds prose, so each line is
**derived from the condition's key** every time it is drawn, and a key the build
does not know is shown as itself rather than dropped. A record that will not read
is itself a line: an unreadable file must never look like nothing wrong.

**The monitor's own refusals are lines too** (`monitor-refusal/<key>`), raised
by the backup pass, which reads the configuration every time, and cleared by it
when fixed. A monitor that is configured and refuses records nothing at all, and
before 2026-10-11 it said so only in the log. A key the backup refusals already
show — a missing tool, fleet server or identity — is one line, not two.

**Clicking a line opens the log**, which is where the detail is. The tooltip
counts the same problems and points at the menu.

**It is re-read every ten seconds, and the menu is rebuilt only when the set of
problems changes** — a menu replaced while it is open can close under the
operator's pointer. The durations refresh with the backup pass's own rebuild.

**There is no acknowledge.** Red lasts until the fault clears. Ruled
2026-09-23: an acknowledge that turned red to amber was offered and declined as
more work than its value, pending whether Windows notifications want it.

---

## 6. What this program must never learn

**Nothing here may become the authority for what an application publishes.**

If Rivet Monitor ever needs to know that TempMon's backup is SQLite, or where
another application keeps its database, that knowledge is in the wrong
repository: it belongs
in the application's own signed manifest, where the person changing the format
is already editing.

**The test:** adding a new application to the fleet must require **no change to
this program**. The day it does, the contract in Rivet is missing something, and
the fix is there.

**It holds today.** Nothing in this program names an application, a format, a
path or a machine. The only place an application's name is written down at all
is an override the operator chose to write, and §2.3 exists because that is the
one place a rename can hurt.

---

## 7. What is proved, by what method, and what is not

**Until it runs, it isn't working**, so each claim below names its method.

**Proved by unit test**, `cargo test --bin rivet-monitor`: the
defaults; a mistyped key refused; an override inheriting what it does not set;
`exempt` carried through; retention keeping the newest N; a `.partial` skipped; a
placement separated by machine and application; the plan parser reading an
empty plan as empty, refusing output that is not a plan, and refusing a count
that disagrees with the lines; the icon refusing to call "never stored"
current, and every state drawing the same silhouette; §5.5's whole layer —
announce once, clear once, a check that did not run never clearing, an
enumerated prefix clearing a deleted subject without reaching past itself, and
a restart re-announcing nothing; and, added 2026-09-09 for §3.5.1's filter —
the `--service` argument list built for one due application and for several;
nothing due building an empty argument list rather than an unfiltered fetch;
an exempt application and a declared feed never appearing in what is due;
`tool::preflight` accepting a build whose `backup` usage names `--service`;
and `tool::preflight` refusing one that can `backup` but not filter it, naming
the missing flag and how to install a build that has it.

**§3.1 and §3.7, the weekly schedule, 2026-10-02** — by unit test: a time
parsed strictly and every loose spelling refused naming what is accepted; due
at a scheduled time and not again after a success; a time passed while the
program was down caught up on the next pass; never backed up due at once;
manual only never due and never late, yet still watched; late exactly when no
success follows the time before the most recent; four times a week resolving
correctly; a local time converted at a fixed offset; a daylight-saving change
between two times not moving the older one; Back Up Now taking manual-only
applications and never an exempt one; the missing schedule and the retired
`cadence` refused by name. **By running Windows' own conversion on this PC**:
local times on both sides of this year's daylight-saving changes round-trip.
**By running the built binary** against scratch configurations that contact
nothing: `check` refuses a file still saying `cadence`, with the replacement,
and a file saying `"Fri 4am"` does not load and names the entry.

**Not yet proved:** a scheduled backup fetched by the running tray at its time
against the live fleet; the **Back Up Now** menu item clicked in the running
tray; and an icon turning amber on a real late application.

**§2.4's install route, 2026-10-10** — by running it on the development PC: the
command, against `github.com/caladil/Rivet` (confirmed public by an
unauthenticated request), installed `cargo-rivet 0.1.136` into an
empty install root, and the same command run again replaced it. `check`, with a
scratch configuration naming only that binary, accepted it through
`tool::preflight` itself. It also passed every other proof `src/tool.rs` makes,
each run by hand with the same arguments and contacting nothing:
`screenshot --help` listing `--holder` and `--json` as options; the log's
cursor; and `get` for the seven other signals. `check` against a configuration
with no `tool.path`, and against one naming a program that is not `cargo-rivet`,
printed the command in each refusal. By unit test, every refusal about the tool
carries it: no `tool.path`; a binary that will not start, for `preflight` and
for each of the monitor's proofs; no `backup`; no `--service`; and a
`screenshot`, `get` or log proof that the build fails (the four that lacked it
were found and fixed on 2026-10-11).

**Re-pinned to `v0.1.137`, 2026-10-11**, the release Rivet was re-cut into. The
command at that tag — run through the git CLI with the maintainer's own
credentials, because the re-cut repository was still private — installed
`cargo-rivet 0.1.137 (1d61fcd)` into an empty install root; `check` accepted it
through `tool::preflight`, and it passed the screenshot, log and seven `get`
proofs by hand, contacting nothing. Every citation of Rivet's record in the
published files — 79 documents, sections and links — was checked against that
tree's headings, by a script whose section check was itself shown to fail on a
section that does not exist. The log line `card-stalled/` matches is unchanged
there. **Then without credentials, once Rivet was public the same day**: the
tag listed by `git ls-remote` with every credential helper disabled, and the
command exactly as §2.4 prints it — cargo's own git client, no git CLI —
fetched the repository and installed `cargo-rivet 0.1.137 (1d61fcd)` from a
fresh target into an empty root, which `check` accepted through
`tool::preflight`. Every link into Rivet's documents answered 200 to a browser.
**Not proved:** the route on a machine that has never built Rust, or whose
Cargo has never fetched Rivet before — this one had a toolchain and Cargo's git
cache already held the repository; and the public build against a fleet, which
was not contacted.

**§4.4's names, 2026-10-11** — by unit test: every rule refuses (`..`, absolute,
drive, UNC, separators, wildcards, control characters, trailing dot or space,
leading `-`, reserved device names with and without an extension) and ordinary
fleet names pass; a register device with an unusable name has no target; a plan
naming `--into`, `..`, `C:\x` or `nul` is refused; a monitor machine so named
is refused. Added after the review of what was first pushed, the same day: a
leading dot and a short-name `~1` refuse; a plan naming two applications that
differ only in case refuses; a backup name ending in `.exe`, `.lnk`, `.url`,
`.ps1`, `.cmd` or `.website`, in any case — or that is only `.url` or `.exe` —
is not placed, while `.db`, `.json`, `.tar`, `.csv` and no extension are; and an
override's `retention = 0` is refused. **Not proved:**
against a real hostile register or machine — no fleet server was made to serve
one.

**§5.8's monitor refusals, 2026-10-11** — by unit test: a monitor switched on
with no `[monitor] folder` stands as `monitor-refusal/monitor.folder`; once the
folder is set none stands and the enumerated prefix clears it; no `[monitor]`
table is no problem; and a key the backup refusals already show is not shown
twice. By running the built binary: `check` against the README's smallest
configuration, less its fleet server, loads it, resolves `~\.cargo\bin\…`, and
refuses only the missing server. **Not proved:** the problem line in the
running tray.

**Proved against the maintainer's live fleet, 2026-09-09**, with a scratch
configuration and a scratch folder:

* the register read off `rivet-01`'s fleet server over the operator certificate
  — one device, `rivet-01`, `192.168.50.10:7777`, with its pin;
* a **real** TempMon backup fetched through the whole scheduler and placed at
  `<folder>/rivet-01/tempmon-rivet/tempmon-backup-…db`, then opened
  **independently** with SQLite: `PRAGMA integrity_check` = ok. A second capture
  minutes later held one more reading than the first — a live database, not the
  same bytes twice;
* the refusal with no folder configured, naming all four required keys;
* the refusal when pointed at an old `cargo-rivet` on the PATH, which answered
  *"`backup` is not a `cargo rivet` verb. This build has 9…"*;
* §2.3's loudness: one override matching nothing on the application, one on the
  host, and a third that matched — with no noise from the third;
* §3.2: a deliberately broken fetch (a wrong `host-suffix`, so discovery
  succeeded and TLS then refused the name) **retired nothing**, wrote no ledger
  entry, and left no staging debris;
* §5.2's proxy gate in both directions: `rivet-01` answered on 443 and got a
  line; under the broken-name run it did not answer and got none.

**Proved against the maintainer's live fleet, 2026-09-09, second sitting**, with
his own configuration and his own destination:

* a real TempMon backup fetched and placed in the folder he chose, opened
  independently: `PRAGMA integrity_check` = ok, its readings and sensors
  intact;
* §5.5's whole cycle against the real register: an override naming a machine the
  register does not hold **raised once**; three further passes over the same
  fact announced **nothing**; a pass pointed at a dead port raised the register
  failure and left the override condition standing rather than reporting it
  fixed; and removing the override from the file cleared it on the enumerated
  prefix, reading *"the override for … is no longer wrong — it had been that way
  only briefly."*
* **The running tray**, not just `run-once`: with `poll-seconds = 30` it fetched
  a real backup unattended, then noticed an override **appended to its
  configuration file while it was running** and announced it within one tick,
  then cleared it when the line was removed. That is the timer, the reload and
  the announcement layer, all in the process that actually runs;
* §5.5.3a: the tray started **with a configuration that would not parse**, stayed
  up, announced `config` once, and cleared it — *"the configuration file loads
  again"* — within thirty seconds of the typo being removed.
* **Every format the fleet offered, fetched and opened independently**:
  TempMon's SQLite (`integrity_check` ok), a JSON export (parsed whole), and a
  tar — read back by both `bsdtar` and Python's `tarfile`;
* **§5.4.1's health, read off the real board**: `rivet-01` answered `healthy`
  with `tempmon-rivet` reporting *"no broadcast from 1 of 2 sensor(s):
  GVH5075_xxxx"* (the sensor's identifier masked), which is a true sentence
  about a sensor that had actually gone quiet. Every other service answered
  `unknown`, because none of them adopts the channel yet.

**Proved by the maintainer, 2026-09-09: the icon and the menu.** He ran the tray
and reported the fleet portal, the machine's own page and `View Config` all
opening, and switched `Start with Windows` on himself. The icon is now the Rivet
mark tinted by state rather than a plain disc, at his request.

**Proved against the maintainer's live fleet, 2026-09-09, third sitting —
§3.5's `--service` filter**, with an isolated scratch configuration (its own
directory, so its ledger started empty) and a scratch destination:

* `tool::preflight` proving `--service` against a real binary rebuilt from the
  Rivet tree that added it, and refusing the release build that predated it in
  the same run, naming the missing flag;
* `confirm`, from an empty ledger so every non-exempt application on
  `rivet-01` was due at once: `tempmon-rivet` (SQLite, its magic verified) and
  a JSON application were fetched and placed;
* `archive-rivet`, overridden `exempt` in the scratch configuration, **never
  appeared anywhere** — not staged, not placed, not in a discard note. This is
  `Rivet ADR-051` §11's own "NOT proved" — `confirm --service` against a
  machine offering more than one application, checked that only the named
  ones' bytes moved — closed for Rivet Monitor's side of that call.

**NOT proved: `Start with Windows` surviving a real login.** The registry value
carries the executable's path with no arguments, which is correct only because
the program reads `%APPDATA%\RivetMonitor\config.toml` by default — and that file
did not exist when the value was written. It does now. The first proof is his
next login.

**NOT proved: backups from a second machine.** The register has two members and
the menu lists both, but only one has its backup switch on, so the serial
one-machine-at-a-time rule and the per-host directory have been exercised with
one.

**NOT proved: the flyout on the glass.** §2.7's writer is tested against text —
comments kept, blocks created and kept, a refused file not written, `\r\n` kept
— and `check` reads the switches back from the live file. §5.7's marks were
chosen by the operator on a dark-mode menu, where two thinner arrows failed. Whether a click redraws the checkmark at once is proved only by
clicking one.

**NOT proved: a feed.** `archive-rivet` does not declare one yet, so the path
that reports a feed and declines to fetch it has run only in a unit test.

---

## History

### 2026-09-09 — §2's configuration block was incomplete, and building it said how

The block written on 2026-09-08 named four keys. Three more turned out to be
required before anything could run, and each is now in the body with its reason:

* **`tool.path`** (§2.4). The document assumed *"Rivet Monitor calls `cargo
  rivet backup`"* named a single binary. It does not: the `cargo-rivet` on a
  workstation's PATH can be an old build with no `backup` verb. The path is configuration
  and the verb is proved at startup.
* **`fleet.identity`** (§4). §4 already said the two credentials are not
  interchangeable; the configuration block named neither of them.
* **`fleet.host-suffix`** (§2.5). The register holds `rivet-01` and the
  certificate is for `rivet-01.local`. §4.1 had the principle and nothing joined
  the two spellings.

The superseded block also carried `poll-seconds = 300  # health polling`. There
is no health polling: see §5.4.1. The key survives with an honest meaning — how
often a pass runs.

And its overrides were keyed `host = "rivet-01.local"`. The register's label is
`rivet-01`, so every override in that example would have matched nothing — which
§2.3 would now report loudly, and which is a small demonstration of why it has
to.

### 2026-09-09 — `exempt` cannot be honoured as written, and the fix is in Rivet

§2.2 says `exempt = true` *"means never fetched"*. **It is not.** `cargo rivet
backup` fetches a machine whole and has no per-application filter, so an exempt
application's bytes cross the network and are then deleted unread. Retention,
cadence and the destination are all correct; the transfer is not avoided.

`ADR-001` §4 forbids routing around that here, so the body (§3.5) states what
actually happens and this entry states the fix: **a `--service NAME` filter on
`cargo rivet backup`**, repeatable, restricting the run to the named
applications. That is a small addition to a verb that already parses a table of
them, it does not give the verb any memory (`Rivet ADR-051` §8.2's line), and it
would make `exempt` mean what §2.2 says as well as saving the transfer for a
very large application — which is the case `exempt` exists for in the first
place.

Recorded here rather than fixed here, because the tool is the one that must be
right.

### 2026-09-09 — §5 had two sections numbered 5.3

The label and the icon both carried `### 5.3`. The icon is now §5.4 and has
gained its states, which had never been written down.

### 2026-09-09 — the fetch is filtered, closing the entry above the same day it was written

The previous entry named the fix and left it for Rivet. Rivet landed it a few
hours later: `cargo rivet backup` takes a repeatable `--service NAME`
(`Rivet ADR-051` §8.5). §2.2 and §3.5 are rewritten to the new behaviour rather
than struck, because the body states only what is true now — what follows is
what changed and why, not a second copy of the body.

**What changed.** `schedule::due_here` already computed, per machine, exactly
which applications are due — cadence resolved through any override, `exempt`
and a declared feed excluded. That list previously drove only *placement*:
`fetch_and_place` asked `cargo rivet backup` for the whole machine and then
kept or discarded each staged file against it. It now drives the *fetch*
too: the same list is passed straight through as `--service`, so an
application never in it is never named to the tool and its bytes never
reach this workstation. `SPEC-01` §2.2's `exempt` now reads *never fetched*
because it is.

**The trap this closed a second time.** A `cargo-rivet` with `backup` but
without `--service` would pass the verb check §2.4 already made and then
silently reproduce the exact defect just fixed — fetching every machine
whole, with nothing telling Rivet Monitor the filter never took hold.
`tool::preflight` now proves `--service` the same positive-assertion way it
proves the verb itself (§3.5.1), and refuses rather than degrading: a
degraded mode that fetched unfiltered machines anyway would have to
reproduce today's discard-after-arrival fallback, which is the failure this
whole change exists to retire, not a feature to keep available.

**Proved:** by unit test — the `--service` argument list for one due
application and for several, an exempt application and a feed never
appearing in it, nothing due building no fetch at all, and `preflight`
accepting a build that documents `--service` and refusing one that does
not, by name. And against the real fleet, `confirm`, from an empty ledger so
every non-exempt application on `rivet-01` was due at once: `tempmon-rivet`
and a JSON application were fetched and stored — real files, `PRAGMA
integrity_check` clean on the SQLite one — and `archive-rivet`, overridden
exempt in the scratch configuration, never appeared anywhere under the
staging directory or the destination. This is the case `Rivet ADR-051` §11
lists as its own "NOT proved" — `confirm --service` against a machine
offering more than one application, watched to confirm only the named ones'
bytes moved — closed here, for Rivet Monitor's side of the call, with a
scratch configuration and a scratch destination.

### 2026-09-15 — §5.5: the announced set is two records, because the monitor is a second writer

The superseded sentence, verbatim:

> An **announcement** is a condition appearing or disappearing, and there is
> exactly one per transition. The set of conditions already announced is persisted
> in `state.toml` beside the ledger, so a restart repeats nothing.

**Why it changed.** `SPEC-02` §6 routes the monitor's conditions through this
layer, and the monitor runs on its own threads, one per machine. `state.toml`
is rewritten whole by the backup pass (`src/ledger.rs`), so a monitor writing
its announcements into the same file would race it and lose whichever update
renamed first — a condition announced twice, or a recovery never said. The
monitor's announced set is therefore `monitor-state.toml`, with one writer, and
the two key sets are disjoint so neither pass's enumerated prefixes reach the
other's. §5.5's rules — once per transition, persisted, a check that did not run
never clears, a deleted subject clears — are unchanged and apply to both.
`SPEC-02` §6.7 has the design and the test that interleaves the two; §5.5.4
gained the monitor's meaning of "acted" (`SPEC-02` §6.8).

### 2026-09-23 — the menu became flyouts, and backups stopped being fleet-wide

The maintainer ruled that backups and health monitoring are switched **per machine and off by default**, from a flyout on each machine's line (§2.6, §2.7, §5.6, §5.7). Two decisions changed with it.

**Backups were fleet-wide.** Every application with a backup surface, on every machine in the register, was fetched on its cadence, and the only way to stop one was a per-application `exempt`. That backed up a development board as eagerly as the server whose data matters. `exempt` survives, for the application an enabled machine should skip.

**§5.2 was a flat list, and only machines with a proxy got a line:**

> ~~**Only machines that actually run a proxy get a line.** A headless member with no `rivet-proxyd` has no index to open, and a menu item that opens a refused connection is worse than no item.~~
>
> ~~**A flat list rather than a submenu**, and one entry is not a special case.~~

Both held while a line was only a door. Once a line also carries switches, leaving a proxy-less machine out would hide switches it needs, so every member gets a flyout and a machine with no proxy gets a **disabled** door instead — which keeps the argument (never offer a door that opens onto a refusal) and drops the omission.

### 2026-09-23 — health problems colour the icon

Superseded text of §5.4.1:

> ~~**It is deliberately not a fifth icon state, and not folded into these four.**
> The icon says one thing — *how stale is the last successful backup* — and a
> colour that also meant *something on the fleet is degraded* would be a colour
> that cannot be acted on, because the two have different remedies and different
> urgencies. A machine that is degraded and backing up perfectly is not a backup
> problem.~~

**Why it changed:** the monitor's conditions went only to the log, and a
`frozen/rivet-01` stood for two days while the icon stayed green — it happened
to be false, and nobody knew either way. The remedy argument is answered by the
problems list (§5.8), which names each problem; the icon only has to be noticed.
The maintainer ruled one icon and one list on 2026-09-23.

### 2026-10-02 — cadence became a weekly schedule, and manual only became possible

Superseded text of §3.1:

> ### 3.1 Cadence is measured from the last SUCCESS, never from a wall clock
>
> **`daily` means "it has been at least a day since the last successful backup of
> this application", not "at 02:00".**
>
> The machine this runs on **sleeps**. A wall-clock schedule silently never fires
> on a desktop that is off at the appointed hour, and the operator discovers that
> when he needs the backup. An elapsed-time rule fires the moment the machine
> wakes and cannot be missed.

and of §5.5.3:

> **Lateness uses twice the cadence, the same threshold as the icon**, so the icon
> and the announcement can never disagree. One missed weekly backup is a machine
> switched off on the wrong evening; two is a machine that has stopped.

The configuration said `cadence = "daily" | "weekly" | "monthly"` (default
`weekly`, a month being thirty days), and the icon was amber when the newest
success anywhere was older than twice the strictest cadence configured.

**Why it changed.** The maintainer ruled on 2026-10-02 (`R-002`) that the
configuration says *when* — any number of weekly days and times, including
none — rather than *how often*, with an empty schedule meaning manual only and
no Windows wake timer. **The premise §3.1 rested on was also wrong**: the
program is meant for a PC that stays on — the flight recorder (`SPEC-02`)
captures every few minutes, which it could not do otherwise.

**What survived, rewritten as a positive statement in §3.1:** catch-up.
*"Nothing has succeeded since the most recent scheduled time"* keeps the one
property the elapsed-time rule had — a time cannot be silently missed — and the
reason is now the true one: Rivet Monitor itself can be down at the minute
(a reboot, a restart, a refused configuration). Lateness kept its *"two missed,
not one"* argument, counted in scheduled times instead of multiples of a
cadence, and the icon was moved onto the same per-application rule rather than
a fleet-wide newest-success age, so the two still cannot disagree.

**Added with it:** `schedule` is a required key, as `folder` is; `cadence` is
refused by name rather than as an unknown key; and Back Up Now (§3.7), without
which *manual only* would have meant *never*.

### 2026-10-11 — §4: the certificate no longer fetches anything

The superseded table rows and the paragraph under them, verbatim:

> | **the control-plane identity** | `~/.rivet/identity/<name>` — `device.seed` and `device.crt`, a pinned Ed25519 key (`Rivet ADR-025` §2) | asking a machine **what its applications declare** |
> | **the fleet authority** | `~/.rivet/ca` — `browser.crt`, `browser.key`, `root.crt` | **fetching the payload**, because `rivet-proxyd` requires a chain to the fleet authority (`Rivet ADR-046` §4.2) |
>
> **The control-plane identity is not a certificate the proxy will accept**, and
> the operator certificate is not a control-plane pin. A tool holding one of them
> can discover backups it cannot fetch, or fetch backups it cannot discover.

**Why it changed.** Rivet moved backups onto the control plane
(`Rivet ADR-051` §8.1, `Rivet R-234`): a machine now fetches its application's
backup over loopback and streams it back, and `cargo rivet backup` refuses the
browser certificate's flags. Rivet Monitor stopped passing them on 2026-10-04,
so the certificate reads the register and checks the proxies and fetches
nothing. The pre-public review of 2026-10-11 found the table still saying
otherwise; §4 was rewritten to what the code does, with where each credential
comes from.
