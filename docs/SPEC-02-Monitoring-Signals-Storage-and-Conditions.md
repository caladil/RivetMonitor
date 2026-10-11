# SPEC-02: Monitoring — Signals, Storage, and Conditions

* **Status:** Normative. **Every section is BUILT** (2026-09-15): §2, §3, §4.1
  and §5 for the seven spot readings, §4.2 the log, §4.3 the screenshot, and §6
  the conditions — **six of them**, `frozen/` fed by §4.3 and §6.9's
  `card-stalled/` by §4.2. The screenshot's cadence and its counted retention
  are the maintainer's rulings of 2026-09-15 (§2.3, §5.4, `ADR-002` §2's
  History). **§7's acceptance is not yet met**: the hour against `rivet-01` and
  the maintainer's look for a console window are still to run.
* **Date:** 2026-09-15
* **Code:** `src/config.rs` (§2), `src/monitor.rs` (§2.2, §3, §4.3's flow),
  `src/capture.rs` with `src/tool.rs` (§3.3, §4.1, §4.2, §4.3's call, proof and
  reading), `src/store.rs` (§2.4's log filter, §4.2's cursor, §4.3's staging,
  verdict and index, §5), `src/clock.rs` (§5.1's dates), `src/conditions.rs` (§2.4's
  service check, §6), `src/alarm.rs` (`SPEC-01` §5.5's layer, which §6 uses),
  `src/armada.rs` (§2.2's Fleets)
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Decided by:** `ADR-002`
* **Builds on:** `SPEC-01` §2 (configuration rules), §4 (credentials and the
  register), §5.5 (announcements)
* **Depends on Rivet:** `cargo rivet screenshot --holder --json`, which finds
  the display holder's frame (`Rivet R-190`); §4.3 is built on it.

---

## 1. Scope

What the monitor captures, how often, where it keeps it, how long, and which
conditions it raises. **It is optional**: no `[monitor]` table means no monitor.
It is configured in the file, and the only thing the menu changes is each
machine's *Health monitoring* switch (`SPEC-01` §2.6, §5.6), which writes that
file. What the monitor raises, the menu lists and the icon shows alongside the
backups' own problems (`SPEC-01` §5.4, §5.8).

It does not specify what any answer *means* beyond the fields §6 reads. The
answers are Rivet's (`get status`, `get log`, `get thermal`, …) and are stored
verbatim.

---

## 2. The configuration

Added to `SPEC-01` §2's file. Everything in `SPEC-01` §2 still applies —
**above all, a key this build does not define is refused.**

```toml
[monitor]
folder               = "E:\\Monitor"    # REQUIRED — §2.1
timeout-seconds      = 20               # one capture's limit — §3.3
# Condition defaults every machine inherits — §6. 0 / false switches one off.
reach-misses         = 3
log-stalled-minutes  = 10
card-stalled-minutes = 70
frozen-minutes       = 15
power                = true

[[monitor.machine]]
name                 = "rivet-01"       # a REGISTER name — §2.2
enabled              = true             # the machine's switch — §2.2 (default: false)
log-stalled-minutes  = 20               # optional override of any default above

[monitor.machine.status]                # a signal's table present = captured
every-seconds = 60
keep-days     = 14
services      = ["aura-renderer"]       # optional — §2.4

[monitor.machine.log]
every-seconds = 60

[monitor.machine.screenshot]
every-seconds = 300
keep-frames   = 3                       # frames are counted, not dated — §2.3
```

### 2.1 `monitor.folder` is required and has no default

For `SPEC-01` §2.1's reasons, and it is **not** `backups.folder`. A `[monitor]`
table without it is a refusal naming the key, and nothing is captured. The folder
may be on the same drive as the backups; it may not be the same folder or inside
it, and the reverse is refused too.

### 2.2 A machine is named by its register name

Resolved against the register exactly as `SPEC-01` §4.2 resolves every machine:
address, port, pin and DNS name all come from there, and none may be written in
this table. **Three outcomes, mirroring `SPEC-01` §2.3:**

* the name is in no register → **refused, loudly**, listing the names the
  register does hold (`capture/<name>/register`);
* the register could not be read → **unverified**, not refused. **If no register
  has been read since Rivet Monitor started, nothing is captured** and nothing is
  cleared (§6.1). **If one has, capture continues against the last register
  read**, and the failure is reported, naming when that register was read;
* **two entries naming one machine** → refused; only the first would apply.

**Why a failed re-read does not stop capture.** The register is served by
`rivet-fleetd`, and the fleet server very often runs **on a monitored machine** —
on this fleet it is `rivet-01` itself. A monitor that stopped capturing whenever
the register could not be read would stop writing `missed` lines at exactly the
moment that machine wedged, which is the moment they are the evidence (`ADR-002`
§1). The last register read is still the register — nothing about it was typed
by hand, which is what `ADR-002` §3 forbids — and it is kept in memory only, so a
restart reads it afresh. The register is re-read every `fleet.poll-seconds`, and
every 30 s while no register has been read.

A machine name must also be usable as a directory name (§5.1), by `SPEC-01`
§4.4's rule: one Windows path component, which `.`, `..`, a separator, a drive
colon, a reserved device name or a trailing dot is not.

**Which register, when the operator keeps several Fleets.** When the Armada
Registry at `~/.rivet/armada` lists Fleets (`Rivet ADR-059` §6.3), the monitor
reads every listed Fleet's register — at that Fleet's portal, verifying the
certificate for the Fleet's name, with the same `fleet.authority` — and resolves
names against their union. A device name in more than one Fleet resolves to the
Fleet whose file sorts first, and the others are named in a note rather than
silently shadowed. A Fleet file that will not parse is reported once and
skipped; it never blocks the Fleets that did read. With no Fleets listed, the
monitor reads `fleet.server` alone, as above. **The backup pass and the menu
read `fleet.server` only.**

**`enabled` is the machine's health-monitoring switch, and it defaults to
`false`** — `SPEC-01` §2.6, which the menu flips (`SPEC-01` §2.7, §5.6). A block
with `enabled = false`, or none, is a machine the monitor **does not capture**:
it is treated exactly as a machine removed from the file (§3.5) — its worker
stops, its conditions clear (§6.1), and what it already stored is kept. Its
settings stay in the file for when it is switched on again. **The register check
above applies only to machines that are on**: a switched-off block naming a
machine the register does not list is not a refusal, because it asks for
nothing. The checks on the file's own shape — two blocks for one machine, a name
that is not a directory name — apply to every block, on or off.

### 2.3 The signals, and their defaults

| signal | asks | default `every-seconds` | retention | default |
| :--- | :--- | ---: | :--- | ---: |
| `status` | `get status` | 60 | `keep-days` | 14 |
| `log` | `get log all all <cursor>` | 60 | `keep-days` | 14 |
| `thermal` | `get thermal` | 60 | `keep-days` | 14 |
| `screenshot` | `cargo rivet screenshot` (§4.3) | 300 | **`keep-frames`** | 3 |
| `memory` | `get memory` | 300 | `keep-days` | 14 |
| `boot` | `get boot` | 300 | `keep-days` | 14 |
| `storage` | `get storage` | 3600 | `keep-days` | 14 |
| `slot` | `get slot` | 3600 | `keep-days` | 14 |
| `process` | `get process` | 60 | `keep-days` | 14 |

**A signal is captured when its table is present**, and an empty table takes both
defaults. **`every-seconds` below 30 is refused**, naming the floor and why:
every capture writes records into the board's own journal (`ADR-002` §5).
`keep-days` below 1 is refused, and so is `keep-frames` below 1.

**The screenshot's retention is a COUNT, and the two keys are not
interchangeable.** `keep-frames` keeps the newest N **stored frames** for a
machine, across day directories (§5.4); `keep-days` keeps **dated files**, which
for `screenshot` means its index alone. A frame is 6.2 MB and a status line is
about 3 KB, so a day of frames is gigabytes and a day of anything else is not
(`ADR-002` §2's History). **`keep-days` on a `screenshot` table is refused,
naming `keep-frames`**, and `keep-frames` on any other signal is refused naming
`keep-days`: a retention key that was silently ignored is exactly `SPEC-01` §2's
failure — a setting the operator believes is in force and is not.

**`frozen-minutes` must EXCEED twice the screenshot's `every-seconds`, and a
configuration where it does not is refused**, naming both numbers. §6.4 is
measured across separate fetches of one frame, so a window no longer than two
cadences would call a screen frozen on the strength of a single repeated frame.
The defaults satisfy it with room to spare: 15 minutes is 900 s against a floor
of 600. `frozen-minutes = 0` switches the condition off and cannot break a rule
about it.

### 2.4 `services` narrows `status` and `log`, and nothing else

* On `status`: only the named services are kept in the stored `services` array.
  **The machine-level fields — `health`, `mode`, `display_*` — are always kept.**
* On `log`: only records whose `svc` is named are stored, **plus every
  `kernel` record, always.** The kernel belongs to no application and is what
  explains a hang.
* A `services` key on any other signal is refused.
* **A name the machine does not run is loud** (`capture/<machine>/<signal>/service`),
  judged against the most recent `status` answer — `SPEC-01` §2.3's rule, for
  the same reason: a rename turns a filter into one that matches nothing. It is
  judged on each `answered` status capture, so a machine that does not capture
  `status` leaves it unverified.

This finding and §2.2's `capture/<name>/register` are §6.6 conditions and go
through `SPEC-01` §5.5's layer like every other (§6.7). A register that could
not be read is not one of them: the backup pass already announces it as
`register`, and a second announcement of one fact is what §5.5 exists to
prevent, so the monitor only prints it.

### 2.5 Condition settings override per machine

The five condition keys in `[monitor]` — `reach-misses`, `log-stalled-minutes`,
`card-stalled-minutes`, `frozen-minutes` and `power` — are defaults. The same
keys in a `[[monitor.machine]]` entry override them for that machine. `0` (or
`false` for `power`) switches a condition off.

---

## 3. Scheduling

### 3.1 Its own timer, never behind a backup

The monitor does not run inside `SPEC-01` §3's pass. **A backup fetch in progress
never delays a capture**, and a capture never delays a backup.

### 3.2 One worker per machine; signals serial within it

Each monitored machine has its own worker, so a machine that is off cannot delay
the capture of one that is on. Within a machine, captures run one at a time, the
most overdue first.

### 3.3 Every capture has a time limit, and exceeding it is a miss

**`timeout-seconds` (default 20) is enforced on the child process**, which is
killed when it expires. This is the case the feature exists for: a wedged board
may accept a connection and never answer, and a capture with no limit would stop
that machine's worker at the moment it matters most. A killed capture is a
**miss** (§6.2). `timeout-seconds = 0` is refused: it would kill every capture
before it could answer.

### 3.4 Cadence is measured from the last ATTEMPT

Unlike `SPEC-01` §3.1, which measures from the last success: a monitor's job is to
keep asking. A signal is due when `every-seconds` have passed since it was last
attempted, whatever the outcome.

### 3.5 Configuration changes apply on the next tick

As `SPEC-01` already behaves. A removed machine or signal stops being captured;
§6.1 says what happens to its conditions. Its stored data is left alone and ages
out under the retention it was configured with.

The configuration is re-read every **5 seconds**. A file that will not load, or a
monitor refusal (§2), captures **nothing** until it is fixed — `SPEC-01`'s rule
for backups — and the stretch that leaves is written as `not-captured` when
capture resumes (§5.3). A monitor refusal stops no backup, and a backup refusal
(`backups.folder` missing, say) stops no capture: what they share is only what a
capture needs to reach a machine at all — `tool.path`, `fleet.server` and
`fleet.identity`.

### 3.6 `rivet-monitor monitor` runs the monitor alone, in the foreground

```
rivet-monitor [--config PATH] monitor
```

Runs exactly what the tray runs for the monitor — the same supervisor, the same
workers, the same files — and **no backup pass at all**. Every attempt prints one
line: its UTC capture time, the machine, the signal, the outcome, the elapsed
milliseconds, and for `missed` and `failed` the reason. `not-captured` markers,
refusals, retention sweeps and §2's loud findings print as they happen.

**Ctrl+C stops it**: each worker finishes the capture it is in, so no file ends in
half a line. A second Ctrl+C ends the process at once.

It exists so the monitor can be run and asserted against without the tray, as
`run-once` exists for the backup pass — and like `run-once` it **acts**, so it
settles §6's announcements into the monitor's record (§6.7) and prints each one
as `[started] <key> — …` or `[fixed] <key> — …`. It never touches `state.toml`.

The tray runs the same monitor on its own thread whenever the file has a
`[monitor]` table.

---

## 4. Capturing

Every capture calls the `cargo-rivet` at `tool.path`, with the identity and pin
`SPEC-01` §4 already uses, and with the no-window flag `tool.rs` already sets.
**No capture talks to a board directly** (`ADR-001` §4).

### 4.1 The spot readings: `status`, `thermal`, `memory`, `boot`, `storage`, `slot`, `process`

`cargo rivet run --host … --identity … --pin … -- get <subject>`. The answer must
parse as JSON carrying the `subject` asked for; anything else is a
`capture/` failure (§6.6), never a stored reading.

**`process` was added 2026-09-16 and is the one signal here with a named job.**
The others describe a machine; this one is aimed at a specific failure Rivet's
`rivet-01` has now shown three times, in which the supervisor stops and
everything it started keeps running. The evidence that makes it the right signal
is in that project's
`docs/design-candidates/Supervisor-Blocking-Audit-2026-09-16.md` §10.4: after the
supervisor stopped, `get status` — which asks the supervisor — timed out every
minute forever, while `get thermal`, `get boot` and `get memory` answered in tens
of milliseconds for **eleven more minutes**, because a spawned `rivet` answers
them from `/proc` and `/sys` with no supervisor involved. **`get process` is on
that second side of the line.**

Its row for `rivet-sysd` carries `user_millis`, `system_millis` and — since the
same day — `wchan`, the kernel function the process is parked in. Two captures a
minute apart are therefore **frozen counters and a named wait**, which is the
whole diagnosis, taken off the machine, with nobody awake. That is why its
cadence is a minute and not an hour: the window between the supervisor stopping
and the control plane following it down was eleven minutes and thirty-seven
seconds.

**The tool is proved per subject before it is relied on**, the positive-assertion
way `SPEC-01` §2.4 proves `backup`: `cargo rivet run --show -- get <subject>`
must print the `rivet_get` call carrying `{"subject":"<subject>"}`. It sends
nothing and needs no machine. A subject that fails the proof records a `failed`
attempt at its cadence, carrying why; the proof is retried every minute.

**`missed` or `failed`.** `cargo-rivet` exits `1` for every error, so the exit
status cannot separate them, and the line is drawn on its words:

| what happened | outcome |
| :--- | :--- |
| killed at `timeout-seconds` | `missed` — `no answer within N s` |
| a trust refusal: `invalid peer certificate`, a TLS `received fatal alert`, this end's own TLS identity or seed | `failed` — something answered and this side's credential or pin disagrees with it |
| a transport sentence: `cannot reach`, `cannot resolve`, `resolved to no address`, `cannot send to`, `cannot end the request`, `could not be read while`, `closed the connection before`, `answered nothing for call` | `missed` |
| the tool would not start, the register cannot place the machine, or exit 0 with an answer that is not the subject | `failed` |
| **any other non-zero exit** | `failed`, carrying the tool's words |

Trust phrases are checked before transport phrases, because a trust refusal
arrives inside one. **Unrecognised output is `failed`**: it is this build not
understanding its tool, the fail-closed posture of `SPEC-01` §3.6. The cost is
that a reworded transport sentence in a later `cargo-rivet` would turn misses
into failures, and the unit test of these phrases is what is there to notice.

### 4.2 The log

`cargo rivet run … --full -- get log all all <cursor>`. `--full` because
`cargo rivet run` otherwise caps its output at 64 KiB, and a backfill after a
night with Rivet Monitor not running is megabytes.

* **The cursor** is the answer's own `cursor` field. It lives **beside the data
  it covers**, in `<monitor.folder>/<machine>/log/cursor` — one line, the cursor
  verbatim — and nowhere else.
  * **It is written only after the records it covers are durably written**: the
    answer's lines go down in one write and are flushed to the disk, and only
    then is the cursor replaced.
  * **It is replaced atomically**: written to `cursor.tmp` in the same
    directory, flushed, then renamed over `cursor`, so the file always holds one
    whole cursor, the old or the new.
  * A write of the lines that fails leaves the cursor where it was (§5.5); a
    crash between the lines and the cursor re-fetches rather than loses, and the
    records written before it are then stored twice.
  * Retention (§5.4) touches only dated `.jsonl` files, so it never removes the
    cursor.
* **A missing cursor file means the first capture.** That capture asks
  `get log latest all` — the whole current boot — and takes its cursor from
  that. A cursor file that exists and cannot be read, or is empty, is **not** a
  first capture: the attempt is `failed`, saying so, because reading it as none
  would store the whole current boot a second time. Deleting it is how the log
  is started afresh.
* **The `all` boot selector is what makes a reboot safe**: records from a later
  boot come after a cursor in an earlier one (`rivet-core::operation::log`'s own
  test).
* **An answer whose `since` is `boot-gone`** (the board's retention dropped the
  cursor's boot, so everything held came back) is stored, and a `retention-gap`
  marker (§5.3) ahead of its records records that some were lost in between.
* An answer with no records and no cursor (absent or `null`) is a success that
  stored nothing but its attempt line. It keeps the existing cursor.
* **Fail-closed where the cursor depends on it.** An answer that returned records
  and names no cursor, whose `returned` disagrees with its `records`, or — when a
  cursor was sent — whose `since` is neither `at` nor `boot-gone`, is `failed`:
  nothing is stored and the cursor is not moved, so the next capture asks again
  from the same place.
* **The tool is proved for the log's own shape**: `cargo rivet run --show --full
  -- get log all all <cursor>` must print the `rivet_get` call carrying
  `"subject":"log"` and a `since`. A build that translates `get log` but not a
  cursor would otherwise pass §4.1's proof.
* **A limit that is Rivet's, and is not worked around here** (`ADR-001` §4).
  A cursor is `<boot>@<mono>`, `mono` has millisecond resolution, and the board
  selects records whose `mono` is strictly greater. Records sharing a `mono` are
  common — 438 of the 1,194 kept from `rivet-01`'s boot on 2026-09-15, mostly
  kernel bursts — so a record appended *after* an answer, in the same
  millisecond as that answer's last record, would never be returned. Checked
  against the board's own journal over 14 captures: it did not happen. The fix
  belongs in `rivet-core`'s cursor, not in a second copy of its placement here.
* **`timeout-seconds` applies unchanged.** Measured against `rivet-01` on
  2026-09-15 with the debug build: `get log latest all` (3,524 records, 1.4 MB)
  in 0.84 s, and the whole journal (`get log all all`, 20,413 records, seven
  boots) in 1.7 s — the largest answer a `boot-gone` backfill can bring. A
  default of 20 s is more than ten times that, so the log has no limit of its
  own.

### 4.3 The screenshot

Built on `cargo rivet screenshot --holder --json` (`Rivet R-190`).

```
cargo rivet screenshot --host … --identity … --pin … --holder --json --out <monitor.folder>/<machine>/screenshot/staging.png
```

`--holder` asks the machine which application **holds the display** and fetches
that application's saved frame; where the frame lives is Rivet's to know, and no
board path is composed here (`ADR-002` §4). Nothing is pressed. `--json` prints
one object carrying the three things this program needs:

* the name of the holder (`holder`);
* the frame's own timestamp, by the board's clock (`frame_modified_ms`);
* a distinct outcome, not an error, when **no application holds the display**
  (`outcome` is `fetched` or `no-application-holds-the-display`).

**The tool is proved before it is relied on**, the positive-assertion way
`SPEC-01` §3.5.1 proves `--service`: `cargo rivet screenshot --help` must print
`screenshot`'s own usage, with a line starting `--holder` and a line starting
`--json`. A build without them has `screenshot` and would fetch the launcher's
frame — hours old while an application holds the display — and describe it in
prose. A tool that fails the proof makes **screenshot** attempts `failed` at
their cadence, carrying why; every other signal is captured as usual. The proof
is retried every minute.

**Reading the answer, fail-closed.** Killed at `timeout-seconds` is `missed`,
and a non-zero exit is §4.1's table. An exit 0 must print one JSON object
carrying all five keys the tool promises (`outcome`, `holder`, `board_path`,
`frame_modified_ms`, `out`) and agreeing with itself: `fetched` names a holder,
a board path and the file written; the no-holder outcome names none of them. A
holder that could not be a file name (`/`, `\`, `:`, `..`, …) is refused, since
it becomes one (§5.1). Anything else is `failed`, and nothing is kept.

**Then, on the answer:**

* **No holder** → nothing fetched, nothing stored, and the index (§5.2) records
  `no application holds the display`. This is not a failure.
* **The frame's timestamp is the one last stored for this holder** → the staged
  PNG is deleted and the index records `unchanged`.
* **Otherwise** → the PNG is renamed to
  `<machine>/screenshot/<YYYY-MM-DD>/<HHMMSS>-<holder>.png` and the index
  records `stored`. That includes a different holder whose frame carries the
  same timestamp (it is a different picture), and a frame whose
  `frame_modified_ms` is null — nothing shows it unchanged, and a flight recorder
  errs toward keeping the evidence. An existing file of that name is never
  overwritten; the attempt then records `not stored`, saying why.

**The timestamp is compared as a value from the board, never against the PC's
clock** (a Pi has no RTC; the two are not comparable). It is compared for
equality only.

**Where "the last stored for this holder" lives: the screenshot index itself,
and only where the PNG is still there.** When a worker starts it reads every
dated `screenshot/*.jsonl` file oldest first and keeps, per holder, the
`frame_modified_ms` of the latest `stored` line **whose named file is on disk**;
after that it remembers a new one only once its `stored` line has been written,
so memory never says more than the index, and it **re-reads the index after
every retention sweep** (§5.4), which is what keeps that true when the sweep
counts rather than dates. A `stored` line whose frame has been retired says
nothing, so nothing is ever called `unchanged` against a PNG that is gone. It
survives a restart, and it needs no second file:

* **Not `state.toml`**, which the backup pass rewrites whole (§4.2's History).
* **Not `monitor-state.toml`**, whose one writer records which conditions were
  *announced* (§6.7). A frame timestamp is capture data, not an announcement, and
  putting it there would make every stored frame a write of the announcement
  record under its lock.
* **Not a file of its own like the log's cursor.** The cursor must outlive the
  records it points past; this fact must not outlive the frames it describes.
  Reading it back off the index, against the files on disk, is what ties the two
  together: a holder whose last stored frame has been retired is stored afresh
  rather than called `unchanged` against a PNG that no longer exists. A separate
  file would keep answering `unchanged` after the evidence was gone.

**The order is rename, then line.** A crash between the two leaves a frame with
no `stored` line, and the next attempt stores it again: a duplicate, never a
loss. The reverse order could leave a `stored` line with no frame, and every
later attempt against that frame would say `unchanged`.

**The staging file.** It is inside `monitor.folder`, so keeping a frame is a
rename on one volume. It is removed **before every fetch** if one is there — a
run killed mid-fetch leaves one — and the removal is printed; a leftover that
will not go makes the attempt `failed`. After a `missed` or `failed` attempt, and
on `unchanged` or no holder, it is deleted, so no PNG is left behind.

**Every answered attempt feeds §6.4** — holder (or none), `frame_modified_ms`
and `captured_at` — whatever the verdict, and whether or not its line could be
written: the machine's answer is a true observation either way.

**`timeout-seconds` applies unchanged, and there is no pre-check.** Measured
against `rivet-01` on 2026-09-15 with the debug build, 22 attempts at a 30 s
cadence: 533–720 ms, median 652 ms — `unchanged` (median 654 ms) no faster than
`stored` (633 ms), because the frame crosses the network either way — against a
median of 65 ms for `get status` in the same run. That is a thirtieth of the
default limit, so a cheaper "has it changed?" question first would save no
meaningful time and would cost a second connection's journal records.

**What it costs on disk: about 19 MB a machine.** Each stored frame is the
tool's uncompressed PNG — 6,222,418 bytes for `rivet-01`'s 1920×1080 display,
measured — and §5.4 keeps the newest `keep-frames` of them, three by default. So
the frames cost 3 × 6.2 MB whatever the cadence, and the cadence decides only
how much of the recent past those three cover: at the default 300 s, the last 15
minutes. The index beside them is one line an attempt, about 300 bytes, which at
that cadence is 86 KB a day and under 1.2 MB at `keep-days = 14`.

**The cadence is not the retention question, and both were ruled on
2026-09-15** (`R-001`). Aura saves a new frame every 60.0 s (ten consecutive stored frames
were 59,996–60,036 ms apart by the board's clock), so on a machine running it
every fetch stores one: a minute's cadence kept for three days would have been
about 36 GB. `ADR-002` §2's History carries the measurement that corrected the
estimate and the ruling that followed it.

---

## 5. Storage

### 5.1 Layout

```
<monitor.folder>/<machine>/<signal>/<YYYY-MM-DD>.jsonl
<monitor.folder>/<machine>/screenshot/<YYYY-MM-DD>/<HHMMSS>-<holder>.png
```

**The date is the PC's local date at capture**, never a board timestamp, and a
stored screenshot's `HHMMSS` is the PC's local time of day from the same reading.
Every line records the PC's UTC capture time as well, so a file boundary never
hides when a thing happened.

### 5.2 One line per attempt

Every attempt writes exactly one line to its signal's file:

```json
{"captured_at":"2026-09-15T08:49:03Z","elapsed_ms":412,"outcome":"answered","answer":{…}}
{"captured_at":"2026-09-15T08:50:03Z","elapsed_ms":20000,"outcome":"missed","reason":"no answer within 20 s"}
```

* `answered` carries the machine's answer **verbatim**, after §2.4's filter:
  the very text it printed when no filter applies, and the parsed answer
  re-serialised **in the machine's own key order** when one does.
* `missed` carries the reason in words (§6.2 counts these).
* `failed` is a capture failure on this side (§6.6) and carries the reason.
* For `log`, `answer` is replaced by the records, one line **per record**, each
  wrapped with the same `captured_at` — a day's log file stays greppable. The
  record is the machine's own JSONL text, byte for byte, after §2.4's filter.
  **The attempt's own line comes after its records**, carrying the answer's
  counts and cursor in place of them — `returned` is before the filter, `stored`
  after it:

  ```json
  {"captured_at":"2026-09-15T18:07:56Z","outcome":"record","record":{"ts":"…","boot":"b1","mono":28784.33,"svc":"aura-renderer",…}}
  {"captured_at":"2026-09-15T18:07:56Z","elapsed_ms":652,"outcome":"answered","returned":7,"stored":1,"held":2048,"unreadable":0,"since":"at","cursor":"b1@28784.33"}
  ```

  **A record string that is not a JSON object is kept, never dropped**, as
  `{"captured_at":"…","outcome":"unreadable-record","text":"<the string exactly>"}`,
  whatever the filter says — it cannot be read, so it cannot be one the filter
  was asked to drop — and the attempt line counts it as `unreadable_records`.
  The rest of the answer is stored as usual. A record with no `svc` is kept for
  the same reason.
* For `screenshot`, the `.jsonl` is the index. An answered attempt's line
  carries, in place of `answer`, the verdict as `result` — `stored`,
  `unchanged`, `no application holds the display`, or `not stored` with a
  `trouble` saying why — the `holder` and `frame_modified_ms` (each `null` when
  nothing holds the display), the tool's `board_path` when there is a holder, and
  for `stored` the `file` relative to the screenshot directory. Only a `stored`
  line names a PNG on disk, and only `stored` lines are read back (§4.3).
  `missed` and `failed` lines are the usual ones:

  ```json
  {"captured_at":"2026-09-15T19:16:37Z","elapsed_ms":552,"outcome":"answered","result":"stored","holder":"aura-renderer","frame_modified_ms":1789499782796,"board_path":"/state/app/aura-renderer/data/screen.qoi","file":"2026-09-15/191637-aura-renderer.png"}
  {"captured_at":"2026-09-15T19:17:07Z","elapsed_ms":572,"outcome":"answered","result":"unchanged","holder":"aura-renderer","frame_modified_ms":1789499782796,"board_path":"/state/app/aura-renderer/data/screen.qoi"}
  ```

### 5.3 Gaps are written, never inferred

Two markers, each one line in the signal's file:

* **`not-captured`**, written when a capture runs and the previous attempt for
  that signal is older than **twice** `every-seconds` — carrying `from` and `to`.
  It covers Rivet Monitor not running, a refused configuration, and a signal
  re-enabled after a while off. It is written immediately before
  the attempt's own line, into that attempt's file:

  ```json
  {"captured_at":"2026-09-15T09:49:03Z","outcome":"not-captured","from":"2026-09-15T08:49:03Z","to":"2026-09-15T09:49:03Z"}
  ```

  **"The previous attempt" is read off the disk** — the last `answered`,
  `missed` or `failed` line in the signal's newest dated file — so a restart
  knows it. **A signal with no previous attempt anywhere gets no marker**: its
  file begins at its first attempt, and there is no stretch of it to explain.
* **`retention-gap`**, §4.2's case: written ahead of the records of an answer
  whose `since` is `boot-gone`, naming the cursor that could no longer be placed:

  ```json
  {"captured_at":"…","outcome":"retention-gap","after":"<the cursor held>","why":"…"}
  ```

  A `boot-gone` answer with no records writes no marker (its cursor is not
  moved, so every later capture would write another); its attempt line still
  says `"since":"boot-gone"`.

An empty stretch in a file without a marker is therefore a defect in this program,
not a fact about the machine.

### 5.4 Retention

Once an hour, and once at startup. **Two retentions, because a frame is not a
line:**

* **Dated files go by age.** For each machine and signal, files dated more than
  `keep-days` before today are deleted — the screenshot's index among them.
* **Stored frames go by count.** For each machine, the newest `keep-frames`
  PNGs under `screenshot/` are kept and the rest are deleted, **across day
  directories**: the names §5.1 gives are `YYYY-MM-DD/HHMMSS-<holder>.png`, both
  parts fixed-width and both the PC's local clock, so ordering the names orders
  the frames by when they were stored. A day directory a sweep empties is
  removed.

A signal no longer configured keeps the `keep-days` (or `keep-frames`) it last
had, or the default when it never had one. Directories under `monitor.folder`
that this program did not create are never touched.

**How "did not create" is known.** The first write into `<folder>/<machine>/`
leaves a `.rivet-monitor` file there. Retention walks **only** machine
directories carrying it, **only** the nine signal names inside them, and
**only** files named exactly `YYYY-MM-DD.jsonl` inside those — plus, inside
`screenshot` alone, directories named exactly `YYYY-MM-DD`. From such a
directory it removes only files named `HHMMSS-<holder>.png`, and then the
directory **only if it removed something from it and that left it empty**: a
file somebody else put there keeps its directory, and a directory this sweep did
not touch is left alone. It goes no deeper than that, and never touches the
staging file. "More than `keep-days` before today" is by the PC's local
calendar: with `keep-days = 2`, today's and the two previous days' files are
kept. "The retention it last had" is remembered for as long as Rivet Monitor
runs; after a restart a signal that is no longer configured takes the default.

**A sweep is hourly, so `keep-frames` is a floor the sweep restores, not a cap
enforced per capture.** Between two sweeps a machine can hold more than
`keep-frames`: at the ruled 300 s cadence that is at most twelve more in an
hour, about 75 MB. The alternative — sweeping after every stored frame — would
put a directory walk in the capture path for a bound nothing needs to the
minute.

A retention sweep runs only while the monitor's configuration is accepted, so a
refused file cannot delete anything.

### 5.5 A write that fails is loud

A folder that is missing, not writable or full is `capture/<machine>/<signal>`
(§6.6). **Nothing is dropped silently**, and the log cursor does not advance
(§4.2).

---

## 6. Conditions

**Six of them** — `reach/`, `log-stalled/`, `card-stalled/` (§6.9), `frozen/`,
`power/` and `capture/`. All go through `SPEC-01` §5.5 — **announced once when
they appear and once when they clear**, persisted across restarts. A standing
one turns the icon red and is named on the menu's problems list (`SPEC-01`
§5.4.1, §5.8), and `reach/` is one of the critical few that raise a Windows
notification (`SPEC-01` §5.5.4).

### 6.1 The rules §5.5 already has still apply

* **A condition whose check did not run has not cleared.** A worker that could
  not reach the register, or a machine that did not answer, clears nothing but
  `reach/` can speak to.
* **A condition whose subject was deleted clears.** Removing a machine's entry
  clears everything under `*/<machine>`; removing a signal clears the conditions
  that exist only because of it (§6.3's `log-stalled/`, §6.4's `frozen/`,
  §6.5's `power/`, and that signal's `capture/`, and `…/<signal>/service` when
  its `services` filter goes); removing the whole `[monitor]` table clears every
  one. Switching a condition off with `0` or `false` clears it too.
  * **Judged on the accepted file alone**, on every supervisor tick whose
    configuration loads and is not refused — whether or not the register can be
    read, because a machine removed from the file is removed whatever the fleet
    server says. A file that will not load, or is refused (§3.5), is
    *unverified* and clears nothing.
  * A deletion clearance says why — *"reach/rivet-01 is no longer watched, so it
    is cleared: rivet-01 is no longer a [[monitor.machine]]"* — rather than the
    recovery sentence, which would claim the machine had recovered.
  * **A worker still holding the plan from before the change cannot raise it
    again.** Its reading is checked against the subjects the supervisor last
    accepted, and a key with no subject is dropped.

### 6.2 `reach/<machine>`

**Raised** after `reach-misses` (default 3) consecutive `missed` attempts across
**all** of that machine's signals, in the order they were attempted. **Cleared**
by any `answered` attempt. A `failed` attempt (§6.6) neither counts nor resets:
it says nothing about the machine.

### 6.3 `log-stalled/<machine>`

Requires `log`. **Raised** when no new record has arrived for `log-stalled-minutes`
(default 10) **while the machine is answering** — at least one `answered` attempt
on any signal inside that window. **Judged on records before §2.4's filter**, so a
narrow filter on a quiet service cannot fake a stall. **Cleared** when a new record
arrives.

A machine that stops answering raises `reach/`, not this. The two together are
the distinction `ADR-002` §1 needed: *alive but its journal stopped* versus
*gone*.

**How it is judged.** Only on an `answered` log capture, and on its `returned`
count (§5.2): `returned > 0` is a new record, looked at and fine; `returned = 0`
raises once the last capture that returned a record — or the first log capture
this run observed — is `log-stalled-minutes` or more before it. That capture is
itself the answered attempt the window requires. A `missed` or `failed` log
capture judges nothing.

**The monitor's own records do not stop it firing.** Every capture's
control-plane connection writes `rivet-mcpd` records into the board's journal,
so a journal that is accepting records never answers `returned = 0` —
`rivet-01` returned 7–25 records a minute under a one-minute `status` and `log`
cadence. That is a canary, not a mask: those records take the same path as every
other, so a journal writer that has stopped appending stops appending them too,
and a quiet board with nothing else to say is not mistaken for a stalled one.

**What it cannot see.** `rivet-sysd` hands each record to the kernel with
`write(2)` and never `fsync`s (`Rivet cmd/rivet-sysd/src/journal.rs`), and
`get log` reads the journal files back through the kernel. So a card whose
writes stop reaching the media while the board runs on — the best reading of
`ADR-002` §1's incident, not a proven one — still answers every record from the
page cache, and this condition stays silent. It detects a journal that stopped
**accepting** records, not one that stopped **persisting** them.

**§6.9 is the persistence twin, and it is what covers that.** It watches the one
record a machine emits only *after* a write has actually reached the card. The
two can stand at once, and a machine showing both has stopped in every way a
journal can stop.

### 6.4 `frozen/<machine>`

Requires `screenshot`. **Raised** when an application holds the display, has
been **seen drawing** — its frame advanced at least once while it held the
display — and its frame timestamp has then not advanced for `frozen-minutes`
(default 15, §2.3). **Cleared** when it advances, when the holder changes, or when
nothing holds the display. No holder never raises it (§4.3).

**A holder never seen drawing is judged not frozen**, and that judgement clears
a standing `frozen/`. Only a program that was drawing can be said to have
stopped; one whose frame never advances may simply not write frames. That is
Aura on a machine with a compositor (Rivet `ADR-058` §5.3): the compositor takes
the real screenshot, the renderer writes no `screen.qoi` of its own, and the one
it wrote before the compositor existed sits unchanged for ever. Read as a
freeze, it raised `frozen/` against a screen that was animating throughout.

The cost is stated: a holder already frozen when this program starts is never
seen drawing, so it is not caught until it draws again. **Watching the
compositor's own capture is the fix for that**, and it is not built: the capture
is taken on request by `apply input capture`, which writes a new file to the
machine's card each time, and this program asks only read-only questions.

**How it is judged**, per §4.3 observation of holder, frame timestamp and
`captured_at`. The timestamp is compared for equality as the board's own value;
how long it has been unchanged is measured on `captured_at`, from the first
observation of that holder and timestamp. A holder with no timestamp judges
nothing. **The first observation after a restart judges nothing either**: what
the frame was before it is not known, so a `frozen/` standing from before the
restart is not cleared by it — only a change, or no holder, clears.

**Fed by every answered screenshot attempt** (§4.3) whose answer could be read.
A `missed` or `failed` screenshot observes nothing and judges nothing.

### 6.5 `power/<machine>`

Requires `thermal`. **Raised** when a reading's `throttle.under_voltage_now` **or**
`throttle.under_voltage_since_boot` is true. The since-boot flag catches a dip
between two readings, and the machine's own `means` text names it the authority.
**Cleared** by a reading where both are false, which in practice means a new boot.
A reading whose `throttle_unmeasured` says the flags could not be read neither
raises nor clears — and nor does a reading without both flags as booleans.

### 6.6 `capture/<machine>/<signal>`

Something on **this** side prevented a capture: the tool missing or failing its
preflight, an answer that is not the subject asked for, a write that failed, a
`services` name the machine does not run (`…/service`), or a machine name the
register does not hold (`…/register`). **Cleared** when that same capture next
succeeds.

* **`…/<signal>`** is raised by a `failed` attempt, or by any attempt whose line
  — or the `not-captured` marker written ahead of it — could not be written;
  cleared by an `answered` attempt of that signal whose lines were written. A
  `missed` attempt whose line was written judges nothing: it says nothing about
  this side.
* **`…/<signal>/service`** is judged on each `answered` `status` capture, per
  `services` filter.
* **`…/register`** is judged by the supervisor on each tick that has a register
  to capture with (§2.2), including the last one read when a re-read fails.

### 6.7 The record, and its one writer

**The monitor's announced set is `monitor-state.toml`, beside the configuration
file and `state.toml`** — the same shape as `state.toml`'s `standing` table, and
never in `state.toml` itself.

The backup pass rewrites `state.toml` whole: load, change, rename over. The
monitor runs on other threads (§3.1, §3.2). Two writers of one whole-file record
lose updates — whichever renames last discards the other's change, and a lost
entry is a condition announced twice or a recovery never said. §4.2's cursor
left `state.toml` for the same reason.

* **The two key sets are disjoint.** The backup pass raises `config`,
  `refusal/`, `register`, `offers/`, `staging/`, `fetch/`, `place/`, `ledger/`,
  `retention/`, `override/`, `late/` and `health/`; the monitor raises only
  §6's six. Neither pass's enumerated prefixes can reach the other's
  conditions, so the two records never need to agree.
* **Inside the monitor there is one writer.** Every worker and the supervisor
  settle through one shared record whose lock spans the read, the reconcile and
  the rename. The test that proves it interleaves a thread doing what the backup
  pass does to `state.toml` with four workers and a supervisor, and checks that
  every change was announced exactly once and that both files hold what was
  announced.
* **A record that exists and cannot be read is not read as empty.** Nothing is
  announced and nothing is written until it can be read, and the problem is
  printed once. Reading it as empty would announce every standing condition again
  and then overwrite what really happened.
* **What persists is only which conditions were announced.** A worker's run of
  misses, the start of a quiet journal and the last frame seen are in memory.
  (The last frame *stored* per holder is §4.3's and lives in the screenshot
  index; §6.4 does not read it.) A
  restart can therefore delay a raise by up to one threshold; it can never clear
  a standing condition, because each is cleared only by its own evidence — an
  answer, a record, a changed frame, a both-false reading, a success.

A single owner of both records was the alternative. It buys nothing: the backup
pass is already the only writer of `state.toml` in a process, and a channel
between the two would couple the timers §3.1 keeps apart.

**Two processes on one configuration** — the tray and `rivet-monitor monitor`
at once — would be two writers of `monitor-state.toml`, as they already are of
every capture file. Nothing guards against that; do not run both on one file.

### 6.8 What "acted" means for a monitor, which has no passes

`SPEC-01` §5.5.4: only a pass that ACTED settles the record. The monitor's act is
**one capture attempt that was actually made** — a line written, or a write
that failed — and it settles **only the conditions that attempt checked**. The
supervisor acts on each tick whose configuration was accepted: §6.1's deletions,
and `capture/<machine>/register` when it has a register. A paused worker, a
refused or unloadable file, and a register never read settle nothing. `check`
runs no monitor, so it settles nothing here.

### 6.9 `card-stalled/<machine>`

**Requires `log`.** Without it nothing reports the fact this condition is about,
so it is not judged and must not be raised — as §6.4 depends on `screenshot`.

**Raised** when the machine has not got a write through to its card for
`card-stalled-minutes` (default **70**) while it is still answering captures.
**Cleared** when one gets through.

**Why this is the condition `ADR-002` §1 asked for.** On 2026-09-14 `rivet-01`
stopped saving to its card and went on running; the screen froze about 21 hours
later. §6.3 could not have seen it — the journal was still *accepting* records
and `get log` reads them back through the page cache — and `reach/` could not
either, because the machine answered everything it was asked. The one thing that
changed was that writes stopped reaching the media.

**The evidence, and the coupling it creates.** `rivet-sysd` writes the running
clock to the card once an hour, **waits for that write**, and emits an `info`
record whose message begins *"the running clock is written to the card"* **only
when the write succeeded** (`Rivet cmd/rivet-sysd/src/clock.rs`, `Clock::tick`).
So the record is proof that a write reached the card, and it is the only such
proof the control plane carries.

* **This is the one place Rivet Monitor matches the TEXT of a Rivet message.**
  Everywhere else it reads named fields. A reword on Rivet's side would switch
  this condition off silently — no error, no refusal, just a condition that
  never raises again.
* **So the match lives in one named place** — `CLOCK_WRITTEN` in
  `src/conditions.rs`, a prefix, together with the `svc` it must carry — **and a
  unit test pins the exact string** against the record `clock.rs` emits. That
  test is what fails when Rivet rewords it.
* The `svc` is `rivet-sysd`, which `rivet-core::journal` forbids an application
  from claiming, so the match cannot be forged by something a `.rvt` installed.
* The `warn` record for a write that committed but could not be flushed is
  **not** a match. It is a different message, and a card that keeps producing it
  is a card worth raising about.
* Removing the coupling is Rivet's to do and is proposed rather than assumed:
  `get boot` or `get storage` reporting the clock floor's **last durable write
  time** as a field would let this condition read a number instead of a
  sentence. `ADR-001` §4 — the fix belongs there, not in a second copy of
  `clock.rs`'s behaviour here.

**Judged on the records BEFORE §2.4's filter**, like §6.3's `returned`: a
`services` filter that did not name `rivet-sysd` would otherwise delete the
evidence and turn the condition off without saying so.

**How the window is measured: in the board's own monotonic clock, anchored on
the PC's.** Every record carries `mono`, seconds since that boot, so the distance
between the newest clock record and the newest record of any kind is an elapsed
duration measured entirely on the board. Subtracting it from the capture's own
time places the write on this PC's clock without ever comparing the two clocks —
the same discipline §4.3 keeps for a frame's timestamp. `mono` restarts at every
boot, so only the newest boot in an answer is measured.

**The start-up window, and why it is answered this way.** The first log capture
of a run asks `get log latest all` — the whole current boot (§4.2) — so it
carries that boot's hourly clock records and the condition can judge honestly
from it. Three anchors, in order of what the answer supports:

1. **A clock record in this answer.** The anchor is when it was written, and
   this is the only thing that CLEARS the condition. A backfill whose newest
   clock record is already hours old therefore raises on the first capture,
   which is the 2026-09-14 case.
2. **No clock record, in an answer that covers the boot from its start** — the
   first capture of a run, or one whose records cross into a boot this run
   watched begin. The anchor is the boot itself: nothing has reached the card in
   this whole boot. A machine that has just booted is inside the window, and
   `rivet-sysd` writes nothing until its first hour, so this does not raise on a
   fresh boot.
3. **Neither** — Rivet Monitor started with a cursor part-way through a boot, so
   the backfill may hold no clock record and the boot's earlier ones are behind
   the cursor. The anchor is then the first log capture this run observed,
   exactly as §6.3 does it. The condition can still raise once the run has
   watched for a whole window; a restart can delay a raise by up to one window
   (§6.7 says a restart may), and it can never cause one.

**Nothing is marked as looked-at unless a clock record was actually seen, or the
window has run out.** §6.1's rule: a condition whose check did not run has not
cleared, and a restart that knows nothing yet must not clear one standing from
before it.

**The default is 70 minutes**: one write interval plus room for a late one. `0`
switches it off (§2.5), and it overrides per machine like every other threshold.

---

## 7. Acceptance

Nothing here may be written as working until each line below has run, and the
build's report says by what method.

**By unit test:** every refusal in §2 (missing folder, folder inside the backups
folder, a floor breach, an unknown key, `services` on the wrong signal, two entries
for one machine); defaults and overrides; due-ness from the last attempt; a killed
child counted as a miss; the cursor persisted only after the write; `not-captured`
markers at twice the cadence; retention by date that never touches a foreign
directory; each condition raised, not raised and cleared per §6, including a
filter that cannot fake a stall and a `failed` attempt that neither counts nor
resets `reach/`.

**Against `rivet-01`, read-only, with a scratch configuration and a scratch
folder:**

* every signal in §2.3 captured on its cadence for at least an hour, with the
  files opened independently;
* the log's first capture covering the whole current boot, then only new records,
  with no record duplicated or missing across a Rivet Monitor restart;
* a `reach/` raised by pointing one machine at a dead port, and cleared when put
  back;
* a screenshot stored, then `unchanged` for a static frame, then stored again;
* the no-window flag holding: **no console window on the desktop** during the
  hour, observed by the maintainer.

* **§6.9 on the healthy board:** the backfill's clock records found and named,
  and `card-stalled/` not raised at `card-stalled-minutes = 70`; then raised and
  cleared once each with recorded log answers.
* **§5.4's `keep-frames`:** more than `keep-frames` frames stored, and a sweep
  leaving exactly the newest N, named.

**NOT expected to be proven by the build, and to be said so:** `frozen/`,
`log-stalled/`, `power/` and `card-stalled/` raised by a real failure. They are
tested with recorded answers; the real proof is the next incident. So is
overnight capture across an update restart (`ADR-002` §5).

---

## 8. Out of scope

Machines outside the registers it reads; backups from more than one fleet
server (the monitor reads several, §2.2, and the backup pass one); `cpu` and
`network`; pressing capture; anything that writes to a machine.

---

## History

### 2026-10-10 — §6 and §8 brought to the rulings of 2026-09-23

The superseded sentences, verbatim:

> None changes the icon (`ADR-002` §5).

> Machines outside the register; several fleet servers; `cpu`, `process` and
> `network`; pressing capture; the Windows toast; any icon or menu change; anything
> that writes to a machine.

**Why it changed.** The maintainer ruled on 2026-09-23 that a monitor condition
turns the icon red and is named on the problems list (`SPEC-01` §5.4.1, §5.8),
and the Windows notification was built for the critical few, `reach/` among
them (`SPEC-01` §5.5.4). `process` had been added on 2026-09-16 (§4.1). The same
pass corrected §6.4's `frozen-minutes` default, which still read 5 after the
entry below raised it to 15, and replaced the sleeping PC in §4.2 and §5.3: the
program is meant for a PC that stays on, and the gaps those sections cover are
Rivet Monitor's own.

### 2026-09-15 — §2.3 and §5.4: the screenshot's retention is a COUNT, and its cadence is five minutes

The superseded rows and sentences, verbatim:

> | `screenshot` | `cargo rivet screenshot` (§4.3) | 60 | 3 |
>
> `keep-days` below 1 is refused.
>
> Once an hour, and once at startup: for each machine and signal, **files and
> screenshot day-directories dated more than `keep-days` before today are
> deleted.**
>
> **What it costs on disk.** Each stored frame is the tool's uncompressed PNG:
> 6,222,418 bytes for `rivet-01`'s 1920×1080 display. Aura saves a frame every
> 60.0 s […] so a machine running it stores 1,440 frames a day, about 9 GB, and
> `keep-days = 3` holds up to four calendar days — about 36 GB. That is the
> tool's format and this program's configuration, both as specified; whether it
> is what the operator wants is [the build journal]'s open question.

**Why it changed.** Building §4.3 measured the frame and left the question open; the
maintainer answered it on 2026-09-15 (`R-001`): a screenshot every five minutes,
and only the newest three kept. That is two
rulings, and they replace a retention that could not work. **A day of frames is
not comparable to a day of status lines** — 6.2 MB against about 3 KB — so
dating them was the wrong unit: `keep-days` on a `screenshot` table is now
refused by name, `keep-frames` (default 3) keeps the newest N across day
directories, and the default cadence is 300 s. About 19 MB a machine, against
36 GB.

It also changed `frozen-minutes`, which had been 5: at a 300 s cadence that is
one cadence, so §6.4 would have called a screen frozen on a single repeated
frame. The default is 15 minutes and **a configuration where it does not exceed
twice the cadence is refused**, naming both numbers.

And it changed how §4.3's *"the last stored for this holder"* is read back. The
old reasoning was that the index files and the day directories *"share one
`keep-days`, so when a holder's last stored frame is retired its `stored` line
goes with it"* — true while both were dated, and false the moment frames were
counted. The invariant it protected is the one that matters and is kept in the
body: **never answer `unchanged` against a PNG that is gone.** It now holds
because the read-back requires the named file to be on disk, and because a
worker re-reads the index after every sweep.

### 2026-09-15 — §5.4: retention removes old screenshot day directories

The superseded sentence, verbatim:

> Retention walks **only** machine directories carrying it, **only** the eight
> signal names inside them, and **only** files named exactly `YYYY-MM-DD.jsonl`
> inside those; it never recurses and never removes a directory.

**Why it changed.** The same section's first paragraph deletes *"screenshot
day-directories dated more than `keep-days` before today"*, and §5.1 stores
every frame in one — so the two paragraphs contradicted each other, and read
literally the second would have kept every stored PNG for ever, at about 6 MB
each. Building §4.3 settled it: under `screenshot` only, a dated directory is
entered one level, only the PNG names §5.1 gives are removed, and the directory
goes only when that leaves it empty. What the old sentence protected — nothing
this program did not name is ever touched — is kept, and stays in the body.

### 2026-09-15 — §3.6: `rivet-monitor monitor` settles the monitor's announcements

The superseded sentence, verbatim:

> It exists so the monitor can be run and asserted against without the tray, as
> `run-once` exists for the backup pass. It settles no announcement.

**Why it changed.** It was written while §6 was unbuilt, when there was nothing
for it to settle, and building §6 made it wrong in two ways. `SPEC-01` §5.5.4's
rule is that a pass that ACTED settles and a diagnostic does not; `run-once`
settles, `check` does not. `rivet-monitor monitor` runs the very workers the tray
runs, making real captures — it is `run-once`'s kind, not `check`'s. And a
command that ran §6's checks without settling them could not be used to prove
§6 at all, which is the whole reason it exists. Settling cannot consume an
announcement the tray was about to make on a *different* configuration, because
the record lives beside the file (§6.7); two processes on one file are warned
against in §6.7.

### 2026-09-15 — §2.4: the service finding and `…/register` reach the announcement record

The superseded paragraph, verbatim:

> Until §6 is built, this finding and §2.2's `capture/<name>/register` are
> reported — printed by `rivet-monitor monitor`, written to the tray's error
> stream — once when they appear and once when their check finds them gone. They
> do not yet reach `SPEC-01` §5.5's announcement record.

**Why it changed.** §6 was built, and it names both as §6.6 conditions. They are
now settled into §6.7's record, so a restart no longer announces them afresh.
Building it also found that the worker's own clearing of `…/register` for a
removed machine keyed on the second path segment, which read
`monitor/register` — the register-unreadable finding — as a machine named
`register` and cleared it in the same tick that raised it, so while a re-read
kept failing each poll printed `[started]` and then a false `[fixed]`. That finding is not a §6 condition (the
backup pass announces it as `register`), and it is now only printed, by the
supervisor, once per appearance.

### 2026-09-15 — §4.2: the log cursor moved out of `state.toml`, into a file beside the log

The superseded bullet, verbatim:

> * **The cursor** is the answer's own `cursor` field, persisted per machine in
>   `state.toml` **only after** the records it covers are written to disk. A
>   crash between the two re-fetches rather than loses.

**Why it changed.** Building steps 2 and 3 found that `state.toml` is
rewritten whole by the backup pass (`src/ledger.rs`'s `save` serialises the
whole ledger and renames it into place). The monitor runs on its own
thread by design (§3.1), so a worker writing a cursor into that file would race
the pass: whichever wrote last would silently discard the other's update — a
lost cursor re-fetches a stretch, and a lost backup record is worse. The cursor
now lives at `<monitor.folder>/<machine>/log/cursor`, beside the records it
covers, where the one worker for that machine is its only writer; it is replaced
atomically, and still only after its records are durably on disk. The rule the
old bullet carried — cursor after records, so a crash re-fetches rather than
loses — is unchanged and stays in the body.

### 2026-09-15 — §2.2: an unreadable register no longer stops capture once one has been read

The superseded bullet, verbatim:

> * the register could not be read → **unverified**, not refused. Nothing is
>   captured and nothing is cleared (§6.1);

**Why it changed.** Building the worker against the real fleet made the
consequence concrete: the register `rivet-fleetd` serves comes from `rivet-01`,
whose register role is `fleet-server`, and `rivet-01` is the machine this monitor
exists to watch. Read literally, the bullet meant that the moment `rivet-01`
wedged, its register became unreadable, capture of `rivet-01` stopped, and the
`missed` lines that are `ADR-002` §1's whole evidence were never written — the
flight recorder switched itself off at the start of the crash. §2.2's body now
keeps capturing against the last register read and reports the failure; with no
register ever read since start, nothing is captured, which is all the old bullet
can still safely mean.

### 2026-10-11 — §2.2 and §8: the monitor reads every Fleet the Armada lists

§8's first line, verbatim:

> Machines outside the register; several fleet servers; `cpu` and `network`;
> pressing capture; anything that writes to a machine.

**Why it changed.** The monitor has read every Fleet the Armada Registry lists
since 2026-09-23 (`src/armada.rs`, `Rivet ADR-059` §6.3), and this document
still called several fleet servers out of scope and named no Armada at all — the
pre-public review of 2026-10-11 found the gap. §2.2 now says how the registers
are read and joined, and §8 that only the backup pass is held to one fleet
server.

The same review found §1 saying the monitor was *"configuration-only"* and that
*"nothing in the menu changes"*; the per-machine *Health monitoring* switch of
2026-09-23 (`SPEC-01` §2.6) had made both untrue, and §1 now says what the menu
does.
