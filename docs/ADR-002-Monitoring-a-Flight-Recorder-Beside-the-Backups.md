# ADR-002: Monitoring — a Flight Recorder Beside the Backups

* **Status:** Accepted
* **Date:** 2026-09-15
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Ruled:** by the maintainer, 2026-09-15, in a grilling session (thirteen
  questions, every recommendation accepted except one: he chose to skip the
  screenshot rather than fall back to the launcher's frame)
* **Amended:** 2026-09-15, by two further rulings — a sixth condition,
  `card-stalled/`, for the failure §1 actually describes; and the screenshot's
  cadence and count, after its size was measured. §2, and its History.
  2026-10-10: §2, §5 and §6 brought to the rulings of 2026-09-16 to 2026-10-02;
  the History says which.
* **Specified by:** `SPEC-02`

---

## 1. Context

On 2026-09-15 the maintainer found `rivet-01`'s screen frozen. The machine's own
journal could not say why, because **the journal was one of the casualties**:
nothing had reached the card for about 21 hours before the frame he saw. The
clock floor the supervisor writes hourly agreed — its last durable write came
nine minutes before the journal's. The same shape had already happened on
2026-09-11.

Every piece of evidence about a machine lives on that machine. When the machine's
storage is what fails, the evidence fails with it. **The question could only have
been answered from somewhere else**, and Rivet Monitor is already that somewhere:
it holds the credential, reads the fleet's register, calls `cargo-rivet`, and
has an announcement layer that says a thing once.

---

## 2. Decision: Rivet Monitor gains an optional monitor

**An optional feature, off by default, that captures signals from named
machines on independent cadences and keeps them on the operator's drive.** It is
configured in the file; the only thing the menu changes is each machine's
*Health monitoring* switch, which writes that file (`SPEC-01` §2.6, §5.6). Its
conditions go through `SPEC-01` §5.5's existing layer, so the menu lists them
and the icon counts them like any other problem (`SPEC-01` §5.4, §5.8).

| decided | why |
| :--- | :--- |
| **the unit is a machine**, with an optional per-signal service filter | the log, the status answer and the screen are each ONE per machine; a service is a tag inside them, not a separate source |
| **signals:** `status`, `log`, `screenshot`, `thermal`, `memory`, `boot`, `storage`, `slot`, and `process` | each is a question the board already answers; `thermal` carries under-voltage, the leading cause of card corruption on a Pi. `process` was added on 2026-09-16, when an incident asked for it (`SPEC-02` §4.1) |
| **each signal has its own cadence and retention** | a stored screenshot is 6.2 MB against about 3 KB for a status line — three orders of magnitude, measured — so the screen is captured every five minutes and kept by COUNT (the newest three), while everything else is kept by date. `SPEC-02` §2.3 and §5.4 |
| **a separate required `monitor.folder`** | monitoring is high-volume and disposable; backups are precious and small. They do not share a folder, and neither gets a default on the system drive (`SPEC-01` §2.1) |
| **every reading is stored**, except screenshots, which are stored when the frame changes | a flight recorder's "the same at one minute and the next" is evidence; storing only changes would make it indistinguishable from a gap |
| **the log backfills** from a stored cursor | the stretch missed while Rivet Monitor was not running is exactly the stretch wanted afterwards |
| **six conditions**: `reach/`, `log-stalled/`, `card-stalled/`, `frozen/`, `power/`, `capture/` | each is a way a machine fails that nobody would otherwise see; each threshold is configurable and each can be switched off. `card-stalled/` was added on 2026-09-15: §1's incident is a machine whose writes stop reaching its card while it runs on, and `log-stalled/` cannot see that, because `get log` reads the journal back through the page cache (`SPEC-02` §6.9) |
| **its own timer**, never behind a backup fetch | a large backup can take minutes, and a capture that waits behind one is not a one-minute capture |

---

## 3. Decision: machines come from the register, and nowhere else

**A monitored machine must be in the register Rivet Monitor already reads.** A
name the register does not hold is refused as loudly as `SPEC-01` §2.3's
unmatched override. A hand-typed address and pin in this file is exactly the line
that goes stale silently, and `SPEC-01` §4.2 keeps every address and pin in one
place for that reason.

**Several fleet servers are read, through the Armada Registry.** When the
operator keeps more than one Fleet, the monitor reads every Fleet the registry
lists and resolves names against their union (`SPEC-02` §2.2), so a machine in
another Fleet's register is monitored once that Fleet is listed there. A machine
in no register at all is still not monitored, and that is the point of this
section.

---

## 4. Decision: the screenshot's location is Rivet's to know

A frame lives at a path set by a Rivet SDK convention. **Rivet Monitor does not
compose that path.** `cargo rivet screenshot` learns to find the frame of the
application holding the display, and Rivet Monitor calls it. Ruled when this ADR
was decided, and Rivet recorded the same decision on its side (`Rivet R-190`).

This is `ADR-001` §4 applied, not a new principle: the tool that must be right
owns the rule, and a second copy of it here would drift the first time the SDK
renamed the file. `SPEC-01` §6's test still holds — adding an application to the
fleet changes nothing in this program.

**Fetch only, never press.** Pressing `apply input capture` while an application
holds the display photographs the wrong moment and reports success (the reason
Rivet's verb never presses). And **no application holding the display means no
screenshot**: the capture records that fact and fetches nothing.

---

## 5. Consequences

* **It watches only while it runs.** A tray application runs while someone is
  signed in, so it covers an update restart only when the workstation signs
  back in afterwards — Windows 11 has a setting that does — rather than through
  a Windows Service and a second process. Whether startup applications run
  after that automatic sign-in is unproven.
* **Gaps are written down.** A signal that was not captured for a stretch gets a
  marker saying so, so an empty stretch reads as *not looking*, never as *the
  machine was silent*.
* **It costs the board's own journal.** Every control-plane connection writes
  records there, so polling shortens how far back the on-board journal reaches.
  That is Rivet's to address and is noted there; the cadence floor of 30 s bounds
  it here.
* **A standing condition turns the icon red**, as every problem does, and is
  named on the menu's problems list, which is what says which remedy applies
  (`SPEC-01` §5.4.1, §5.8). The icon's job is only to be noticed.
* **`reach/` is one of the critical few that reach the Windows desktop** — a
  watched machine that stops answering raises one notification, once, after
  ten minutes (`SPEC-01` §5.5.4). Every other condition stays on the menu.

---

## 6. Rejected

| alternative | why not |
| :--- | :--- |
| a Windows Service for overnight coverage | a second process, a second install story and shared state to coordinate, when a sign-in setting closes the same gap |
| per-application monitoring as the unit | the screen does not split by application, and a log split at source loses the kernel's records, which belong to no application and explain hangs |
| falling back to the launcher's frame when nothing holds the display | the launcher saves a frame only when pressed, so the file may be hours old; ruled out by the maintainer |
| composing the screenshot path here | a second copy of an SDK convention; §4 |
| hand-configured machines outside the register | §3 |
| `cpu` and `network` | no incident has asked for them; each is one table to add later, as `process` was when one did (`SPEC-02` §4.1) |
| a heartbeat pushed by the board | a wedged board pushes nothing, and pulling measures reachability for free |
| compressing the stored frame, or keeping the QOI the board sent | the cheaper answers to 6.2 MB a frame. Ruled against on 2026-09-15 in favour of cadence and count: a compressed PNG is a Rivet change, and a QOI is a file nothing on this desktop opens. §2's History |

---

## History

### 2026-10-11 — §2: the menu switches monitoring per machine

The superseded sentences, verbatim:

> **A configuration-only feature, off by default, that captures signals from named
> machines on independent cadences and keeps them on the operator's drive.** No
> menu item.

**Why it changed.** On 2026-09-23 the maintainer ruled backups and health
monitoring switched per machine, from a flyout on each machine's line
(`SPEC-01` §2.6, §5.6, and its History of that date); the switch writes the
file, so the file is still the one authority. This ADR still said there was no
menu item, and the review of the first push found it.

### 2026-10-11 — §3: several fleet servers are read

The superseded paragraph, verbatim:

> **The consequence is accepted:** a machine that is its own fleet server
> (`rivet-08ecc` today) is not monitored until one register holds it. Several fleet
> servers is the eventual answer and is out of scope here.

**Why it changed.** The eventual answer arrived on 2026-09-23: the monitor reads
every Fleet the Armada Registry lists (`SPEC-02` §2.2). This ADR still called it
out of scope, and the pre-public review of 2026-10-11 found the gap. The rule the
section exists for — a machine comes from a register, never from a hand-typed
line — is unchanged.

### 2026-10-10 — §2, §5 and §6 brought to the rulings that had overtaken them

The superseded text, verbatim:

> | **the log backfills** from a stored cursor | the stretch missed while the PC slept is exactly the stretch wanted afterwards |

> | `cpu`, `process`, `network` in the first version | no incident has asked for them; each is one table to add later |

> * **The icon does not change.** It means backup staleness (`SPEC-01` §5.4), and
>   a colour that also meant *a machine is unwell* could not be acted on — the
>   reasoning §5.4.1 already applies to health.
> * **Nothing reaches the Windows desktop yet.** The toast is still `SPEC-01`
>   §5.5.4's deliberately unbuilt piece.

**Why it changed.** The program is meant for a PC that stays on; the backfill matters
because Rivet Monitor itself can be down (`SPEC-01` §3.1's History, 2026-10-02).
On 2026-09-23 the maintainer ruled one icon and one problems list — a monitor
condition that stood for two days while the icon stayed green was a problem
nobody saw (`SPEC-01` §5.4.1's History) — and the Windows notification was built
for the critical few, `reach/` among them. The remedy argument the old bullet
made is answered by the problems list, which names each problem. The rows of
§6 that listed `process` as unbuilt went the same way: it was added on
2026-09-16.

### 2026-09-15 — §2: a screenshot is not "40× a status line", and what was ruled instead

The superseded row, verbatim:

> | **each signal has its own cadence and retention** | a screenshot is 40× a status line and does not need keeping as long |

**Why it changed.** It was an estimate, and it was wrong by three orders of
magnitude. Building `SPEC-02` §4.3 measured the real thing: `cargo rivet
screenshot` writes an **uncompressed** PNG by design (Rivet `ADR-009` §5), which
for `rivet-01`'s 1920×1080 display is **6,222,418 bytes**, against about 3 KB for
a status line. Aura saves a new frame every 60.0 s, so every fetch stored one:
the specified minute's cadence and `keep-days = 3` came to roughly **36 GB per
machine**, on a feature whose whole premise (§2) is that monitoring is
high-volume and disposable.

**The maintainer ruled the cadence and the count, not compression**, on
2026-09-15 (`R-001`): a screenshot every five minutes, and only the newest
three kept. So the screenshot's `every-seconds` defaults to
300, its retention is `keep-frames` (default 3) rather than `keep-days`, and the
cost is about **19 MB per machine** — three frames, whatever the cadence. The
alternatives §6 now lists — a QOI on disk, or asking Rivet for a compressed PNG —
were left unbuilt: both are work, and neither is needed once three frames is the
whole cost.

What the old row was reaching for is kept in the body and is now true rather than
estimated: the screen is not like the other signals, and it does not get their
retention.
