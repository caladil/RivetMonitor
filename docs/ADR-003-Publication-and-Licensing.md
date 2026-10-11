# ADR-003: Publication and Licensing

* **Status:** Accepted
* **Date:** 2026-10-10
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Ruled:** by the maintainer — 2026-10-10 (`R-103`): a private development
  repository, a separate repository the published tree is copied into and that
  he makes public, and Rivet's licensing model; 2026-10-11 (`R-100`–`R-102`):
  the whole corpus scrubbed, his words kept out of it, the first publish
  repository retired for a fresh one, the GitHub no-reply identity, and
  personal information read in its widest sense
* **Revised:** 2026-10-11 — §2, §2.1, §2.2, §4 and §5; the History says what
  each replaced and why

---

## 1. Context

Rivet Monitor is developed in a private repository whose record describes one
operator's real fleet. It is published the way Rivet is
([Rivet `ADR-010` §6](https://github.com/caladil/Rivet/blob/v0.1.137/docs/ADR-010-Licensing-and-Distribution-Obligations.md)):
**a separate repository built from the tree, never from the log.**

The first publish, on 2026-10-10, went into a private repository and was
reviewed before anyone made it public. The review found personal information
that a re-check of known values could not, and a path-deletion defect — in the
tree, and so in its history too.

---

## 2. Decision: the public repository is copied from the tree, and replaced rather than rewritten

| repository | visibility | holds |
| :--- | :--- | :--- |
| `caladil/RivetMonitor-Dev` | private | the working repository: the code, the whole record, `.rulings/`, and the history |
| `caladil/RivetMonitor` | public, once the maintainer has checked it and made it so | the published tree, one commit per publish; created fresh on 2026-10-11 |
| `caladil/RivetMonitor-Retired` | private, never public | the first publish repository, replaced rather than rewritten (below); deleted on the maintainer's word once `caladil/RivetMonitor` is public (`R-101`) |

**The history is re-created, not shared**, matching Rivet and Ignition. The
development history is never carried, the repositories share no commit SHAs,
and nothing published may cite a development commit.

**A published history that carries what §2.2 forbids is replaced, never
rewritten.** After a force-push GitHub still serves a superseded commit to
anyone holding its SHA; a fresh repository never had it.

**A published commit is authored and committed as the maintainer's GitHub
no-reply address**, `12374483+caladil@users.noreply.github.com`, **and stamped
in UTC.** A personal address and a workstation's time zone are metadata a
reader learns without opening a file.

**The published repository is made public by the maintainer**, after its
contents have been checked, and by nobody else.

### 2.1 What is published is an ALLOWLIST

| path | published | why not, where not |
| :--- | :--- | :--- |
| `src/` | yes | |
| `Cargo.toml`, `Cargo.lock` | yes | |
| `README.md`, `LICENSE-MIT`, `LICENSE-APACHE`, `.gitignore` | yes | |
| `docs/ADR-*.md`, `docs/SPEC-*.md` | yes | |
| `docs/design-candidates/` | yes, when there is one | a proposal is part of the design record |
| `.rulings/` | **no** | the maintainer's own words, which the record cites by id (`.rulings/` rule 1) |
| `STATE.md` | **no** | the live state of the maintainer's fleet and backups |
| `docs/PLAN.md` | **no** | the working journal: what was run, against which machine, and what it found |
| `AGENTS.md` | **no** | the maintainer's standing practice for agents in the private repository |
| `tools/` | **no** | the development repository's own guards, about files that are not published |

**A file is published only when this table names it**, so a file added to the
development repository is private until somebody decides otherwise and adds it
here. A denylist would publish every new file by default, which is the wrong
way round for a repository whose record describes a real fleet.

**The published files must read completely without the unpublished ones.** They
cite each other and Rivet's public record, never `STATE.md`,
`AGENTS.md` or `PLAN.md`. A ruling is cited by its id, `R-NNN`, as provenance
only: the sentence citing it says what was decided, and still says it with the
citation deleted. A rule from the unpublished files that a public reader needs
belongs in an ADR — as §2.2 is here.

### 2.2 Zero-scrub is a precondition of every publish

What is in the tree on the day is what is published, so no published file may
hold:

* the operator's real network addresses, fleet names or username;
* **personal information about him, in the widest sense** (`R-102`) — not only
  values, but names, descriptions, file types, counts or sizes from which a
  reader could infer something about him;
* identifiers read off his hardware — a sensor's name derived from its radio
  address, a MAC, a serial — and the real readings, sizes and counts of his
  databases: evidence keeps its claim and drops the value. Applications in
  examples and fixtures are invented, never his own;
* details of his home, his routine or his workstation that say something about
  him rather than about the software;
* **his verbatim words** — they live in `.rulings/` (`R-100`);
* credentials, certificates, pins or private keys;
* absolute workstation paths.

**The placeholders** are `192.168.50.0/24` for addresses — a private range, so a
fixture that depends on an address being private still does, and the range
Rivet's own fixtures use — `home-fleet` for a fleet, `someone` for a user, and
invented application and sensor names (`notes-rivet`, `GVH5075_2222`).
`rivet-01` and `dev-01` are Rivet's conventional machine names and are kept, as
Rivet keeps them.

**Deliberately kept:** the maintainer's name where it credits authorship
(`LICENSE-MIT`, `Cargo.toml`, the README); `Caladil` in the notification
sender's identity (`SPEC-01` §5.5.4), which is the publisher's public handle as
the repository's own address is; and the dates of rulings and of the fleet
incidents the design answers, which are the record's provenance.

**Two checks, and neither is the whole of it.** Both run in the development
repository and are not published: a guard (`tools/docs/rulings.py`) fails if a
recorded quotation appears in a published file, and a re-check of known values
uses the real values as its patterns, so it is kept in that repository's
journal rather than written here. Neither can find a value nobody knew to look
for, which is why §4's last step is a reader.

---

## 3. Decision: `MIT OR Apache-2.0`, matching Rivet

**Ruled 2026-10-10 (`R-103`): Rivet's licensing model.** The repository carries
`LICENSE-MIT` and `LICENSE-APACHE`, and `Cargo.toml` says `MIT OR Apache-2.0`.

The reason is Rivet's
([Rivet `ADR-010` §6.4](https://github.com/caladil/Rivet/blob/v0.1.137/docs/ADR-010-Licensing-and-Distribution-Obligations.md)):
**Apache-2.0 carries an explicit
patent grant that MIT does not**, and some organisations treat MIT-only code as
needing legal review for exactly that reason. The published tree exists so the
code can be reused, and the difference is between adoption and a conversation
with somebody's counsel.

The Apache text is Rivet's file, byte for byte. A licence is text that must not
be written from memory.

---

## 4. Publishing

1. In the development repository: its rulings guard passes, and the zero-scrub
   re-check in its journal returns nothing.
2. The allowlist of §2.1 is archived from a development commit and replaces the
   whole tracked tree of a clone of `caladil/RivetMonitor` — every tracked file
   is removed first, so a file deleted in development is deleted in the publish
   too.
3. **The clone is built and tested from clean** (`cargo test`), because a tree
   that only builds beside the private files is not the tree being published.
4. One commit, named for the version, carrying no development SHA, authored and
   committed as the no-reply address and stamped in UTC (§2), is pushed. Which
   development commit it came from is recorded in the development repository's
   journal.
5. **Before the repository is first made public, the clone is reviewed by
   readers with no stake in its text** — for private data, as a stranger trying
   to use it, and for security — and scanned for secrets. The first publish
   showed that a scrub checked only by its author and by known values misses
   what nobody knew to look for.

---

## 5. Consequences

* **Rivet's repository is [`caladil/Rivet`](https://github.com/caladil/Rivet)**,
  re-cut fresh on 2026-10-11 and tagged `v0.1.137`, which this tree's install
  command and every link into Rivet's documents pin. Everything this tree needs
  from Rivet can then be had by anybody: its citations of Rivet open, and
  `cargo rivet backup` — the one tool this program cannot work without
  (`ADR-001` §4) — installs from it with one command (`SPEC-01` §2.4). Nothing
  published asks a reader for a checkout of Rivet.
* **This repository goes public after Rivet does.** Until Rivet is public, its
  install command and its links open only for someone with access to Rivet, and
  a public Rivet Monitor would send a stranger to a door that does not open.
* **A citation of Rivet is written `Rivet ADR-NNN`, `Rivet SPEC-NN` or
  `Rivet R-NNN`**, and is read in
  [Rivet's `docs/` at `v0.1.137`](https://github.com/caladil/Rivet/tree/v0.1.137/docs),
  the release `SPEC-01` §2.4 pins — a tag rather than the default branch,
  because Rivet rewrites a body when its decision changes, and every Rivet
  section cited here was checked against that release.
* **A ruling number is provenance only**, this repository's (`R-NNN`) and
  Rivet's (`Rivet R-NNN`) alike: neither rulings file is published (Rivet
  `ADR-010` §6.5, and §2.1 here), so a published sentence must make sense
  without looking one up, and a message a user sees names what to install
  rather than a ruling.

---

## History

### 2026-10-11 — a fresh repository, the words in `.rulings/`, and personal information read widely

The review of the first publish (§1) found personal information in its tree and
so in its history, and the maintainer ruled the response (`R-100`–`R-102`). The
superseded text, verbatim.

§2's table:

> | `caladil/RivetMonitor-Dev` | private | the working repository: the code, the whole record, and its history |
> | `caladil/RivetMonitor` | private until the maintainer makes it public | the published tree, one commit per publish |

§4's first and last steps:

> 1. The zero-scrub re-check in the private journal returns nothing.

> 4. One commit, named for the version, carrying no development SHA, is pushed.
>    Which development commit it came from is recorded in the private journal.

**Why it changed.** The repository named `caladil/RivetMonitor` then held two
commits carrying what §2.2 now names, so it was renamed
`caladil/RivetMonitor-Retired` and a fresh one took the name: a rewritten
history still serves its old commits by SHA. §2.2 had listed kinds of value to
keep out; the ruling reads personal information in its widest sense, so a name
or a description that lets a reader infer something about the operator is kept
out as surely as a value is. The maintainer's quotations moved to `.rulings/`,
as Rivet's did, so publishing safely means not copying one directory rather
than finding every quote. The no-reply identity and the UTC stamp keep the
commit metadata from saying what the files no longer do. And a re-check of
known values had passed a tree that a reader then found wanting, so a reader
is now a step of the publish.

### 2026-10-10 — Rivet became public, and "Rivet comes first" was met

§5's first consequence was written while Rivet was still private:

> * **Rivet comes first.** Until Rivet is public, the published tree cites a
>   record a reader cannot open, and `cargo rivet backup` — the one tool this
>   program cannot work without (`ADR-001` §4) — cannot be installed by anybody
>   else.

Rivet was published as `caladil/Rivet` at `v0.1.136` and made public the same
day. The refusals that had told a user to build `cargo-rivet` from a Rivet
checkout now give the public install command, proved by running it, and the
Rivet sections this tree cites were checked against that release.
