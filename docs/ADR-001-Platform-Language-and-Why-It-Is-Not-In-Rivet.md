# ADR-001: Platform, Language, and Why This Is Not In Rivet

* **Status:** Accepted
* **Date:** 2026-09-08
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Ruled:** by the maintainer, 2026-09-08, in conversation while designing the
  backup contract in Rivet

---

## 1. Context

Rivet gained a way for an application to publish a backup surface in its signed
manifest, and `cargo rivet backup` — Rivet's own tool — is what reads that
surface, fetches, and validates. **Something still has to decide when, how
often, how many to keep, and where to put them.**

That decision is an operator's, and several of this operator's applications had
each grown their own answer — TempMon and others each shipped a Windows taskbar
application that watched one server. Rivet Monitor replaces them with one that
watches a **fleet**.

---

## 2. Decision: Windows 11, Rust, Ignition

| | choice | why |
| :--- | :--- | :--- |
| platform | **Windows 11 only** | it is the machine that is on when the operator is at it, and the notification area is where he already looks |
| language | **Rust** | every peer project is Rust; this one calls Rivet's own tool, `cargo-rivet`, rather than linking Rivet's crates or reimplementing what they do (§4) |
| libraries | **`caladil/Ignition`** | configuration, structured logging and the CLI surface, already carried by every sibling. Its self-update is deliberately not taken: this program is built and placed by hand, and a binary that could replace itself would be a second answer to *which build is running* |

**It is not cross-platform and does not pretend it might be.** A tray icon is a
platform's own idiom; the honest way to serve another platform is a second
program, not an abstraction over notification areas.

---

## 3. Decision: this lives OUTSIDE the Rivet repository

**Ruled 2026-09-08.** The maintainer asked directly whether keeping it out was a
disservice to Rivet, given the gap it leaves — and the answer turned on
separating two things that had been conflated.

### 3.1 The gap is real, and it is closed in Rivet rather than here

A contract nothing consumes is worse than no contract, **because it looks
built**. Rivet had one when this was decided: `Rivet ADR-045` had shipped a
health vocabulary and an SDK `report` call that the supervisor read nowhere.
That design was reversed the same day — the supervisor now asks each
application (`Rivet ADR-045` §3.1), `report` is only a hint to ask sooner, and
the polled answer is the health Rivet Monitor reads (`SPEC-01` §5.4.1). A
backup contract with no reader would have been the same mistake made
deliberately — the day somebody renamed a manifest key, nothing would fail.

**So the consumer belongs in Rivet, and it is `cargo rivet backup`**: headless,
cross-platform, testable in CI against the in-tree producers, and the reference
implementation the contract needs. A second Rivet user gets working backups with
no Windows and no graphical program.

### 3.2 What is left for here is genuinely an operator's

Scheduling, retention, per-application overrides, where the files land,
start-with-Windows, the icon and its menu. **None of that is a property of the
platform**, and all of it is a property of one person's arrangement.

### 3.3 And `examples/` would have been the wrong home

Rivet's `examples/` share one invariant, stated in Aura's own manifest: *"its own
workspace, like every other application in `examples/`, because `cargo rivet
package` builds it for a board."* A Windows graphical application never builds
in that Linux container, never runs in `tools/test-all.sh`, and would need
special-casing in every sweep — permanent friction earning nothing.

---

## 4. Consequences

* **Rivet Monitor holds two credentials and no other** (`SPEC-01` §4): the fleet
  authority's operator certificate, which reads the fleet server's register,
  and a control-plane identity, which reaches each machine for what it offers,
  its health and its backups.
* **It calls `cargo rivet backup` rather than reimplementing it.** Fetching,
  validating and the feed cursor are Rivet's; a bug fixed there is fixed here
  without a release.
* **It is bound to a Rivet version.** A manifest key added in Rivet is invisible
  here until the tool it calls understands it — which is correct, because the
  tool is the one that must be right.
* **Nothing here may become the authority for what an application publishes.**
  The moment this program knows something about an application that the
  application's own manifest does not say, that knowledge is in the wrong
  repository (`SPEC-01` §6).

---

## 5. Rejected

**Inside Rivet, as `examples/rivet-monitor`** — §3.3.

**A cross-platform tray abstraction** — the second platform is a second program.

**Reimplementing fetch and validation here** — it would rot against Rivet's
manifest keys, and it is the exact staleness the maintainer named when he asked
whether validation logic belonged in this program at all.

---

## History

### 2026-10-11 — §2 and §4 brought to what the program is

The superseded rows and bullet, verbatim:

> | language | **Rust** | every peer project is Rust, and this one links Rivet's own crates rather than reimplementing them |
> | libraries | **`caladil/Ignition`** | configuration, structured logging, self-update and the CLI surface, already carried by every sibling |

> * **Rivet Monitor holds the fleet operator certificate** and that is the whole
>   of its authority (`SPEC-01` §4). It reaches the fleet server for the register
>   and each machine for its backups.

**Why it changed.** The pre-public review of 2026-10-11 found all three untrue of
the code. The program never linked a Rivet crate: it calls `cargo-rivet`, which
is §4's own point. The maintainer ruled that it takes Ignition's configuration
and logging and not its self-update, and `main.rs` turns update off. And backups
moved onto Rivet's control plane (`SPEC-01` §4's History, 2026-10-11), so the
control-plane identity is the credential every machine is reached through, and
the certificate only reads the register.
