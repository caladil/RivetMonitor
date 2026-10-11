//! **Where a capture goes, and how long it stays** — `SPEC-02` §5.
//!
//! ```text
//! <monitor.folder>/<machine>/<signal>/<YYYY-MM-DD>.jsonl
//! ```
//!
//! **One line per attempt**, whatever its outcome, so a file is a flight
//! record rather than a list of successes: *"it did not answer at 10:48"* is
//! evidence, and so is *"the same at 10:48 and 10:49"*. And **a gap is written,
//! never inferred** (§5.3) — an empty stretch without a `not-captured` marker is
//! a defect in this program, not a fact about the machine.
//!
//! # What retention may touch, and what it may not
//!
//! §5.4: *directories under `monitor.folder` that this program did not create
//! are never touched.* So a machine directory is claimed with a marker file
//! the first time this program writes into it, retention walks only claimed
//! machine directories, only the nine signal names inside them, and only files
//! named exactly `YYYY-MM-DD.jsonl` inside those — plus, under `screenshot`
//! alone, directories named exactly `YYYY-MM-DD`, inside which it removes only
//! the `HHMMSS-<holder>.png` files it names, and then the directory only if
//! that left it empty. It goes no deeper. Anything else in the folder — notes,
//! a copy somebody made, a directory of another tool's — is invisible to it.
//!
//! **The dated files go by age and the stored frames go by count.** Every
//! signal's `.jsonl` files, the screenshot's index included, keep `keep-days`;
//! the PNGs keep `keep-frames` — the newest N for a machine, whatever day
//! directories they are spread across. One frame is 6.2 MB and a day of them
//! is gigabytes, which is what `ADR-002` §2's History settled.

use std::collections::BTreeMap;
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};

use crate::capture::Outcome;
use crate::clock;
use crate::config::{SIGNALS, Signal};

/// The file that says *"Rivet Monitor made this machine directory"*.
pub const CLAIM: &str = ".rivet-monitor";

/// `<folder>/<machine>/<signal>`.
#[must_use]
pub fn signal_dir(folder: &Path, machine: &str, signal: Signal) -> PathBuf {
    folder.join(machine).join(signal.name())
}

/// `<folder>/<machine>/<signal>/<YYYY-MM-DD>.jsonl`, for a LOCAL day number.
#[must_use]
pub fn day_file(folder: &Path, machine: &str, signal: Signal, local_day: i64) -> PathBuf {
    signal_dir(folder, machine, signal).join(format!("{}.jsonl", clock::date_name(local_day)))
}

/// **§2.4 on `status`**: only the named services are kept in `services`, and
/// every machine-level field — `health`, `mode`, `display_*`, all of it — is
/// kept whatever the filter says.
#[must_use]
pub fn filter_status(answer: &serde_json::Value, services: &[String]) -> serde_json::Value {
    let mut kept = answer.clone();
    if let Some(listed) = kept
        .get_mut("services")
        .and_then(serde_json::Value::as_array_mut)
    {
        listed.retain(|service| {
            service
                .get("name")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|name| services.iter().any(|wanted| wanted == name))
        });
    }
    kept
}

/// **One attempt, as its line** — §5.2.
///
/// `answered` carries the machine's answer **verbatim** — the very text it
/// printed — unless `services` narrows it, in which case it is re-serialised
/// in the machine's own key order (`serde_json`'s `preserve_order`).
#[must_use]
pub fn attempt_line(
    captured_at: u64,
    elapsed_ms: u64,
    outcome: &Outcome,
    services: Option<&[String]>,
) -> String {
    let head = format!(
        "{{\"captured_at\":\"{}\",\"elapsed_ms\":{elapsed_ms},\"outcome\":\"{}\"",
        clock::utc_stamp(captured_at),
        outcome.word()
    );
    match outcome {
        Outcome::Answered { raw, value } => {
            let answer = match services {
                Some(services) => filter_status(value, services).to_string(),
                // A line is one line. The machine prints compact JSON; should a
                // build ever pretty-print, the newlines are only whitespace and
                // the parsed value says the same thing on one line.
                None if raw.contains('\n') => value.to_string(),
                None => raw.clone(),
            };
            format!("{head},\"answer\":{answer}}}")
        }
        Outcome::Missed(reason) | Outcome::Failed(reason) => {
            format!(
                "{head},\"reason\":{}}}",
                serde_json::Value::String(reason.clone())
            )
        }
    }
}

/// **§5.3's `not-captured` marker**, when one is owed: the previous attempt is
/// older than TWICE `every-seconds`.
///
/// A signal with no previous attempt anywhere on disk owes none — there is no
/// stretch of that file for a marker to explain.
#[must_use]
pub fn gap(previous: Option<u64>, now: u64, every_seconds: u64) -> Option<(u64, u64)> {
    let previous = previous?;
    (now.saturating_sub(previous) > every_seconds.saturating_mul(2)).then_some((previous, now))
}

/// The marker's line. `captured_at` is the moment it was written, which is
/// `to`, so a file still sorts by its first field.
#[must_use]
pub fn marker_line(from: u64, to: u64) -> String {
    format!(
        "{{\"captured_at\":\"{to}\",\"outcome\":\"not-captured\",\"from\":\"{from}\",\"to\":\"{to}\"}}",
        from = clock::utc_stamp(from),
        to = clock::utc_stamp(to),
    )
}

/// **Append one line, or say exactly why not** — §5.5. The machine directory
/// is claimed on the way (see the module note).
///
/// # Errors
///
/// A folder that is missing, not writable or full.
pub fn append(folder: &Path, machine: &str, file: &Path, line: &str) -> Result<(), String> {
    write_lines(folder, machine, file, std::slice::from_ref(&line.to_string()), false)
}

// ---------------------------------------------------------------------------
// The log — §4.2 and §5.2
// ---------------------------------------------------------------------------

/// `<folder>/<machine>/log/cursor` — **where the log cursor lives**, beside the
/// records it covers (§4.2).
///
/// Not in `state.toml`: the backup pass rewrites that file whole, and a
/// second thread writing a cursor into it would lose updates to whichever
/// wrote last. A file of its own, written by one worker, has no second writer.
#[must_use]
pub fn cursor_file(folder: &Path, machine: &str) -> PathBuf {
    signal_dir(folder, machine, Signal::Log).join("cursor")
}

/// **The cursor held for a machine**: `None` when there is no cursor file,
/// which means the next capture is the first (§4.2).
///
/// # Errors
///
/// A cursor file that exists and cannot be read, or is empty. **Not read as
/// `None`**: that would ask for the whole current boot again and store every
/// record of it a second time. The capture is `failed` instead, saying so.
pub fn read_cursor(folder: &Path, machine: &str) -> Result<Option<String>, String> {
    let path = cursor_file(folder, machine);
    match std::fs::read_to_string(&path) {
        Ok(text) if !text.trim().is_empty() => Ok(Some(text.trim().to_string())),
        Ok(_) => Err(format!(
            "{} is empty. It is written whole or not at all, so something else emptied it; \
             reading it as no cursor would store the whole current boot again. Delete it to start \
             the log afresh (SPEC-02 §4.2).",
            path.display()
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

/// **Replace the cursor atomically**: a temporary file in the same directory,
/// flushed to disk, then renamed over the old one — so the file is always one
/// whole cursor, the old or the new.
fn write_cursor(folder: &Path, machine: &str, cursor: &str) -> Result<(), String> {
    let path = cursor_file(folder, machine);
    let Some(dir) = path.parent() else {
        return Err(format!("{} has no directory", path.display()));
    };
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let temp = dir.join("cursor.tmp");
    {
        let mut handle = std::fs::File::create(&temp)
            .map_err(|e| format!("cannot create {}: {e}", temp.display()))?;
        handle
            .write_all(format!("{cursor}\n").as_bytes())
            .and_then(|()| handle.sync_all())
            .map_err(|e| format!("cannot write {}: {e}", temp.display()))?;
    }
    std::fs::rename(&temp, &path)
        .map_err(|e| format!("cannot move {} over {}: {e}", temp.display(), path.display()))
}

/// **§2.4 on `log`**: a record is kept when its `svc` is named, and **every
/// `kernel` record is kept whatever the filter says** — the kernel belongs to
/// no application and is what explains a hang.
///
/// A record with no `svc` to judge is kept: the filter narrows by service, and
/// a record it cannot place is not one it was asked to drop.
#[must_use]
pub fn keeps_record(record: &serde_json::Value, services: Option<&[String]>) -> bool {
    let Some(services) = services else {
        return true;
    };
    match record.get("svc").and_then(serde_json::Value::as_str) {
        None | Some("kernel") => true,
        Some(svc) => services.iter().any(|wanted| wanted == svc),
    }
}

/// What [`log_lines`] made of one answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLines {
    /// In the order they are written: the `retention-gap` marker when one is
    /// owed, one line per kept record, then the attempt's own line.
    pub lines: Vec<String>,
    /// Record lines among them — AFTER §2.4's filter, unreadable ones included.
    pub stored: usize,
    /// Record strings that were not a JSON object, kept as text.
    pub unreadable_records: usize,
}

/// **One log answer, as its lines** — §5.2.
///
/// * **One line per record**, wrapped with the capture's `captured_at`:
///   `{"captured_at":"…","outcome":"record","record":{…}}`, the record being
///   the machine's own JSONL text, byte for byte.
/// * **A record string that is not a JSON object is kept, not dropped**:
///   `{"captured_at":"…","outcome":"unreadable-record","text":"…"}`, carrying
///   the string exactly. The filter cannot read its `svc`, so it cannot be one
///   the filter was asked to drop, and a log that quietly lost a line it could
///   not read is the failure a log must not have. The rest of the answer is
///   stored as usual.
/// * **`boot-gone` with records** writes a `retention-gap` marker first (§5.3),
///   naming the cursor that could no longer be placed.
/// * **The attempt's own line comes last**: `outcome` `answered`, with the
///   answer's counts and cursor in place of the records — so §3.4 and §5.3
///   still find one attempt line per attempt, at the end of the file.
#[must_use]
pub fn log_lines(
    captured_at: u64,
    elapsed_ms: u64,
    answer: &crate::capture::LogAnswer,
    previous_cursor: Option<&str>,
    services: Option<&[String]>,
) -> LogLines {
    let stamp = clock::utc_stamp(captured_at);
    let mut lines = Vec::with_capacity(answer.records.len() + 2);
    if answer.boot_gone && !answer.records.is_empty() {
        lines.push(format!(
            "{{\"captured_at\":\"{stamp}\",\"outcome\":\"retention-gap\",\"after\":{},\"why\":\"the machine's journal no longer holds the boot this cursor is in, so it answered everything it holds; whatever it logged between the cursor and its oldest record is lost\"}}",
            serde_json::Value::String(previous_cursor.unwrap_or_default().to_string())
        ));
    }
    let mut stored = 0;
    let mut unreadable_records = 0;
    for text in &answer.records {
        match serde_json::from_str::<serde_json::Value>(text) {
            Ok(record) if record.is_object() => {
                if !keeps_record(&record, services) {
                    continue;
                }
                // A line is one line: the machine writes JSONL, and should a
                // record ever carry a raw newline the parsed value says the
                // same thing on one.
                let body = if text.contains(['\n', '\r']) {
                    record.to_string()
                } else {
                    text.trim().to_string()
                };
                lines.push(format!(
                    "{{\"captured_at\":\"{stamp}\",\"outcome\":\"record\",\"record\":{body}}}"
                ));
            }
            _ => {
                unreadable_records += 1;
                lines.push(format!(
                    "{{\"captured_at\":\"{stamp}\",\"outcome\":\"unreadable-record\",\"text\":{}}}",
                    serde_json::Value::String(text.clone())
                ));
            }
        }
        stored += 1;
    }

    let mut attempt = format!(
        "{{\"captured_at\":\"{stamp}\",\"elapsed_ms\":{elapsed_ms},\"outcome\":\"answered\",\"returned\":{},\"stored\":{stored}",
        answer.records.len()
    );
    if unreadable_records > 0 {
        attempt.push_str(&format!(",\"unreadable_records\":{unreadable_records}"));
    }
    if let Some(held) = answer.held {
        attempt.push_str(&format!(",\"held\":{held}"));
    }
    if let Some(unreadable) = answer.unreadable {
        attempt.push_str(&format!(",\"unreadable\":{unreadable}"));
    }
    if let Some(since) = &answer.since {
        attempt.push_str(&format!(",\"since\":{}", serde_json::Value::String(since.clone())));
    }
    if let Some(cursor) = &answer.cursor {
        attempt.push_str(&format!(",\"cursor\":{}", serde_json::Value::String(cursor.clone())));
    }
    attempt.push('}');
    lines.push(attempt);
    LogLines {
        lines,
        stored,
        unreadable_records,
    }
}

/// **Write a log answer's lines, THEN move the cursor** — §4.2.
///
/// The lines go down in one write and are flushed to disk before the cursor is
/// touched. A failure writing them returns before the cursor is written, so it
/// does not advance and the next capture asks for the same records again. A
/// crash between the two does the same: **a re-fetch rather than a loss**. The
/// price is that records written before such a crash are stored twice.
///
/// `cursor` of `None` — an empty answer — leaves the cursor held untouched.
///
/// # Errors
///
/// Lines that could not be written (the cursor is untouched), or a cursor that
/// could not be moved after they were (said in those words).
pub fn commit_log(
    folder: &Path,
    machine: &str,
    file: &Path,
    lines: &[String],
    cursor: Option<&str>,
) -> Result<(), String> {
    write_lines(folder, machine, file, lines, true)?;
    if let Some(cursor) = cursor {
        write_cursor(folder, machine, cursor).map_err(|why| {
            format!(
                "the records are stored and the cursor could not be moved past them, so the next \
                 capture fetches them again: {why}"
            )
        })?;
    }
    Ok(())
}

/// Create `dir` and claim the machine directory it is under (see the module
/// note), so retention knows this program made it.
fn claim(folder: &Path, machine: &str, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let claim = folder.join(machine).join(CLAIM);
    if !claim.exists() {
        std::fs::write(
            &claim,
            "Rivet Monitor writes this machine's captures here (SPEC-02 §5). Its retention \
             touches only the signal directories, the dated .jsonl files inside them, and the \
             dated screenshot directories and the PNGs it named inside those.\n",
        )
        .map_err(|e| format!("cannot write {}: {e}", claim.display()))?;
    }
    Ok(())
}

/// Append `lines`, each followed by a newline, in one write — flushed to the
/// disk itself when `durable`.
fn write_lines(
    folder: &Path,
    machine: &str,
    file: &Path,
    lines: &[String],
    durable: bool,
) -> Result<(), String> {
    if let Some(parent) = file.parent() {
        claim(folder, machine, parent)?;
    }
    let mut handle = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .map_err(|e| format!("cannot open {}: {e}", file.display()))?;
    let mut bytes = Vec::with_capacity(lines.iter().map(|l| l.len() + 1).sum());
    for line in lines {
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
    }
    handle
        .write_all(&bytes)
        .and_then(|()| handle.flush())
        .and_then(|()| if durable { handle.sync_data() } else { Ok(()) })
        .map_err(|e| format!("cannot write {}: {e}", file.display()))
}

// ---------------------------------------------------------------------------
// The screenshot — §4.3 and §5.2
// ---------------------------------------------------------------------------

/// `<folder>/<machine>/screenshot/staging.png` — **where `cargo rivet
/// screenshot` writes the frame before this program decides whether to keep
/// it.** Inside `monitor.folder`, so keeping it is a rename on one volume,
/// never a copy that a full disk could leave half-done.
#[must_use]
pub fn screenshot_staging(folder: &Path, machine: &str) -> PathBuf {
    signal_dir(folder, machine, Signal::Screenshot).join("staging.png")
}

/// **Ready the staging file for a fetch**: the directory exists and is
/// claimed, and a staged frame left by an interrupted attempt is removed —
/// so what the tool writes next is the only thing there. `true` when a
/// leftover was removed.
///
/// # Errors
///
/// A directory that cannot be made, or a leftover that cannot be removed. The
/// attempt is then `failed`: fetching over a file that would not go is
/// fetching into something this program cannot vouch for.
pub fn prepare_staging(folder: &Path, machine: &str) -> Result<bool, String> {
    let staging = screenshot_staging(folder, machine);
    if let Some(dir) = staging.parent() {
        claim(folder, machine, dir)?;
    }
    match std::fs::remove_file(&staging) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!(
            "a staged frame left by an earlier attempt, {}, could not be removed: {e}",
            staging.display()
        )),
    }
}

/// Remove the staging file if there is one.
///
/// # Errors
///
/// One that exists and will not go.
pub fn discard_staging(folder: &Path, machine: &str) -> Result<(), String> {
    let staging = screenshot_staging(folder, machine);
    match std::fs::remove_file(&staging) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("the staged frame {} could not be removed: {e}", staging.display())),
    }
}

/// **What §4.3 does with one answered screenshot.**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing holds the display: nothing fetched, nothing stored.
    NoHolder,
    /// The frame's timestamp is the one already stored for this holder.
    Unchanged,
    /// Keep it.
    Store,
}

impl Verdict {
    /// The index's word — §5.2.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::NoHolder => "no application holds the display",
            Self::Unchanged => "unchanged",
            Self::Store => "stored",
        }
    }
}

/// **Decide, on the board's own values only.**
///
/// `stored` is the last frame timestamp stored **per holder** ([`last_stored_frames`]).
/// Equal for this holder is `unchanged`; anything else — a new timestamp, a
/// holder never stored, a holder whose last stored frame had a different
/// timestamp than another holder's — is stored. **A frame with no timestamp is
/// stored**: nothing shows it unchanged, and a flight recorder errs toward
/// keeping the evidence. The PC's clock is never consulted.
#[must_use]
pub fn judge_frame(shot: &crate::capture::Shot, stored: &BTreeMap<String, i64>) -> Verdict {
    let Some(holder) = &shot.holder else {
        return Verdict::NoHolder;
    };
    match shot.frame_modified_ms {
        Some(ms) if stored.get(holder) == Some(&ms) => Verdict::Unchanged,
        _ => Verdict::Store,
    }
}

/// `<YYYY-MM-DD>/<HHMMSS>-<holder>.png`, relative to the screenshot directory.
#[must_use]
pub fn screenshot_name(local_day: i64, hhmmss: &str, holder: &str) -> String {
    format!("{}/{hhmmss}-{holder}.png", clock::date_name(local_day))
}

/// **Carry out a verdict on the staged frame.** `Store` renames it to
/// `<folder>/<machine>/screenshot/<YYYY-MM-DD>/<HHMMSS>-<holder>.png` and
/// returns that name relative to the screenshot directory; `Unchanged` and
/// `NoHolder` delete it, so no PNG is left behind.
///
/// The rename comes BEFORE the index line (the caller's), so a crash between
/// the two leaves a frame with no `stored` line and the next attempt stores it
/// again — a duplicate, never a loss. The reverse order could leave a `stored`
/// line with no frame, and every later attempt would call it `unchanged`.
///
/// # Errors
///
/// A `Store` whose staged file is missing or empty (the tool said it fetched
/// and wrote nothing), a destination that already exists (never overwritten),
/// or a directory or rename that failed; a staged file that would not be
/// deleted.
pub fn place_screenshot(
    folder: &Path,
    machine: &str,
    verdict: Verdict,
    shot: &crate::capture::Shot,
    local_day: i64,
    hhmmss: &str,
) -> Result<Option<String>, String> {
    let (Verdict::Store, Some(holder)) = (verdict, &shot.holder) else {
        return discard_staging(folder, machine).map(|()| None);
    };
    let staging = screenshot_staging(folder, machine);
    match std::fs::metadata(&staging) {
        Ok(meta) if meta.is_file() && meta.len() > 0 => {}
        Ok(_) => {
            let _ = discard_staging(folder, machine);
            return Err(format!(
                "cargo rivet screenshot said it fetched {holder}'s frame and {} is empty",
                staging.display()
            ));
        }
        Err(e) => {
            return Err(format!(
                "cargo rivet screenshot said it fetched {holder}'s frame and wrote nothing to {}: {e}",
                staging.display()
            ));
        }
    }
    let name = screenshot_name(local_day, hhmmss, holder);
    let dir = signal_dir(folder, machine, Signal::Screenshot);
    let destination = dir.join(&name);
    if let Some(day) = destination.parent() {
        claim(folder, machine, day)?;
    }
    if destination.exists() {
        let _ = discard_staging(folder, machine);
        return Err(format!(
            "{} already exists and a stored frame is never overwritten (was the PC's clock set back?)",
            destination.display()
        ));
    }
    std::fs::rename(&staging, &destination).map_err(|e| {
        let _ = discard_staging(folder, machine);
        format!("cannot move {} to {}: {e}", staging.display(), destination.display())
    })?;
    Ok(Some(name))
}

/// **One answered screenshot, as its index line** — §5.2.
///
/// `result` is the verdict's word — or `not stored` when a `Store` could not
/// be carried out, with `trouble` saying why. Only a `stored` line is a frame
/// on disk, and only a `stored` line is read back by [`last_stored_frames`].
#[must_use]
pub fn screenshot_line(
    captured_at: u64,
    elapsed_ms: u64,
    shot: &crate::capture::Shot,
    verdict: Verdict,
    file: Option<&str>,
    trouble: Option<&str>,
) -> String {
    let string = |value: Option<&str>| {
        value.map_or_else(|| "null".to_string(), |text| serde_json::Value::String(text.to_string()).to_string())
    };
    let result = match (verdict, trouble) {
        (Verdict::Store, Some(_)) => "not stored",
        _ => verdict.word(),
    };
    let mut line = format!(
        "{{\"captured_at\":\"{}\",\"elapsed_ms\":{elapsed_ms},\"outcome\":\"answered\",\"result\":{},\"holder\":{},\"frame_modified_ms\":{}",
        clock::utc_stamp(captured_at),
        string(Some(result)),
        string(shot.holder.as_deref()),
        shot.frame_modified_ms.map_or_else(|| "null".to_string(), |ms| ms.to_string()),
    );
    if shot.holder.is_some() {
        line.push_str(&format!(",\"board_path\":{}", string(shot.board_path.as_deref())));
    }
    if let Some(file) = file {
        line.push_str(&format!(",\"file\":{}", string(Some(file))));
    }
    if let Some(trouble) = trouble {
        line.push_str(&format!(",\"trouble\":{}", string(Some(trouble))));
    }
    line.push('}');
    line
}

/// **The last frame timestamp STORED for each holder**, read off the index —
/// what §4.3's `unchanged` compares against, and how it survives a restart.
///
/// **The index is the record**, as it is for §3.4's last attempt: a second
/// file holding the same fact would be a second thing to disagree. And it ages
/// out WITH the frames: the index files and the day directories share one
/// `keep-days`, so a holder whose last stored frame has been retired is
/// stored afresh rather than called `unchanged` against a PNG that no longer
/// exists.
///
/// Every dated index file, oldest first, every line; a later `stored` line for
/// a holder replaces an earlier one. A line that does not parse is skipped.
#[must_use]
pub fn last_stored_frames(folder: &Path, machine: &str) -> BTreeMap<String, i64> {
    let mut stored = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(signal_dir(folder, machine, Signal::Screenshot)) else {
        return stored;
    };
    let mut dated: Vec<(i64, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            let day = clock::parse_date(name.strip_suffix(".jsonl")?)?;
            entry.path().is_file().then(|| (day, entry.path()))
        })
        .collect();
    dated.sort();
    for (_, path) in dated {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if value.get("result").and_then(serde_json::Value::as_str) != Some("stored") {
                continue;
            }
            // **The PNG has to still be there.** Retention keeps the newest
            // `keep-frames` frames for a machine (§5.4), so a holder that has
            // not held the display for a while can have its last frame retired
            // while its `stored` line is still in a dated index. Answering
            // `unchanged` against a frame that is gone would store nothing and
            // keep nothing — §4.3's invariant, under counted retention.
            let (Some(holder), Some(ms), Some(file)) = (
                value.get("holder").and_then(serde_json::Value::as_str),
                value.get("frame_modified_ms").and_then(serde_json::Value::as_i64),
                value.get("file").and_then(serde_json::Value::as_str),
            ) else {
                continue;
            };
            if signal_dir(folder, machine, Signal::Screenshot).join(file).is_file() {
                stored.insert(holder.to_string(), ms);
            }
        }
    }
    stored
}

/// A name [`screenshot_name`] makes: `HHMMSS-<holder>.png`.
fn is_stored_frame_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    name.len() > "000000-.png".len()
        && name.ends_with(".png")
        && bytes[..6].iter().all(u8::is_ascii_digit)
        && bytes[6] == b'-'
}

/// **§5.4 for the stored frames: keep the newest `keep`, across every day
/// directory, and remove a day directory this left empty.**
///
/// A count rather than an age, because a frame is 6.2 MB and a day of them is
/// gigabytes (`ADR-002` §2's History). The order is the one §5.1's names give:
/// the day directory, then `HHMMSS` inside it, both fixed-width and both the
/// PC's local clock, so sorting the paths sorts the frames by when they were
/// stored.
///
/// **Only what this program named.** A day directory keeps every file it holds
/// that is not an `HHMMSS-<holder>.png`, and keeps its directory with it.
fn retire_frames(dir: &Path, keep: u64, removed: &mut Vec<PathBuf>, trouble: &mut Vec<String>) {
    let Ok(days) = std::fs::read_dir(dir) else {
        return;
    };
    let mut frames: Vec<(String, String, PathBuf)> = Vec::new();
    let mut day_dirs: Vec<PathBuf> = Vec::new();
    for day in days.flatten() {
        let day_path = day.path();
        let Some(day_name) = day.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !day_path.is_dir() || clock::parse_date(&day_name).is_none() {
            continue;
        }
        day_dirs.push(day_path.clone());
        let Ok(entries) = std::fs::read_dir(&day_path) else {
            trouble.push(format!("cannot read {}", day_path.display()));
            continue;
        };
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if is_stored_frame_name(&name) && entry.path().is_file() {
                frames.push((day_name.clone(), name, entry.path()));
            }
        }
    }
    frames.sort();
    let over = frames.len().saturating_sub(usize::try_from(keep).unwrap_or(usize::MAX));
    let mut emptied: Vec<PathBuf> = Vec::new();
    for (_, _, path) in frames.into_iter().take(over) {
        match std::fs::remove_file(&path) {
            Ok(()) => {
                if let Some(day) = path.parent() {
                    emptied.push(day.to_path_buf());
                }
                removed.push(path);
            }
            Err(e) => trouble.push(format!("cannot remove {}: {e}", path.display())),
        }
    }
    // Only a directory this sweep removed something from, and only when that
    // left nothing at all in it.
    emptied.sort();
    emptied.dedup();
    for day in emptied {
        if !day_dirs.contains(&day) {
            continue;
        }
        if std::fs::read_dir(&day).is_ok_and(|mut left| left.next().is_none()) {
            match std::fs::remove_dir(&day) {
                Ok(()) => removed.push(day),
                Err(e) => trouble.push(format!("cannot remove {}: {e}", day.display())),
            }
        }
    }
}

/// **When this signal was last ATTEMPTED**, read off the disk — §3.4 and §5.3.
///
/// The newest dated file's last attempt line (a marker is not an attempt).
/// Read from the file rather than kept in `state.toml`, because the file is the
/// record and a second copy of it is a second thing to disagree.
#[must_use]
pub fn last_attempt(folder: &Path, machine: &str, signal: Signal) -> Option<u64> {
    let mut dated: Vec<(i64, PathBuf)> = std::fs::read_dir(signal_dir(folder, machine, signal))
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            let day = clock::parse_date(name.strip_suffix(".jsonl")?)?;
            entry.path().is_file().then(|| (day, entry.path()))
        })
        .collect();
    dated.sort();
    dated
        .iter()
        .rev()
        .find_map(|(_, path)| last_attempt_in(path))
}

/// The last attempt line's `captured_at` in one file, reading only its tail.
fn last_attempt_in(path: &Path) -> Option<u64> {
    let mut file = std::fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let mut window = 64 * 1024_u64;
    loop {
        let start = length.saturating_sub(window);
        file.seek(SeekFrom::Start(start)).ok()?;
        let mut tail = Vec::new();
        file.read_to_end(&mut tail).ok()?;
        let text = String::from_utf8_lossy(&tail);
        // When the window starts mid-file its first line is partial; skip it.
        let lines: Vec<&str> = text.lines().collect();
        let whole = if start == 0 { &lines[..] } else { lines.get(1..).unwrap_or(&[]) };
        for line in whole.iter().rev() {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let outcome = value.get("outcome").and_then(serde_json::Value::as_str);
            if matches!(outcome, Some("answered" | "missed" | "failed")) {
                return value
                    .get("captured_at")
                    .and_then(serde_json::Value::as_str)
                    .and_then(clock::parse_stamp);
            }
        }
        if start == 0 {
            return None;
        }
        window = window.saturating_mul(4);
    }
}

/// **§5.4: delete what is dated more than `keep-days` before today — and keep
/// only the newest `keep-frames` stored screenshots.**
///
/// `keep` answers `keep-days` for a machine and signal, `frames` answers
/// `keep-frames` for a machine, and `today` is the PC's local day number.
/// Returns what was removed and what could not be.
///
/// **Two retentions, because a frame is not a line.** Every signal's dated
/// `.jsonl` files — the screenshot's index among them — go by age; the stored
/// PNGs go by count, since one is 6.2 MB and `keep-days` on them was tens of
/// gigabytes (`ADR-002` §2's History).
pub fn retain(
    folder: &Path,
    keep: &dyn Fn(&str, Signal) -> u64,
    frames: &dyn Fn(&str) -> u64,
    today: i64,
) -> (Vec<PathBuf>, Vec<String>) {
    let mut removed = Vec::new();
    let mut trouble = Vec::new();
    let Ok(machines) = std::fs::read_dir(folder) else {
        return (removed, trouble);
    };
    for machine in machines.flatten() {
        let machine_dir = machine.path();
        // **Only a directory this program claimed.**
        if !machine_dir.is_dir() || !machine_dir.join(CLAIM).is_file() {
            continue;
        }
        let Some(name) = machine.file_name().to_str().map(str::to_string) else {
            continue;
        };
        for signal in SIGNALS {
            let signal_path = machine_dir.join(signal.name());
            // §5.1's second layout line: the stored frames, kept by count.
            if signal == Signal::Screenshot {
                retire_frames(&signal_path, frames(&name), &mut removed, &mut trouble);
            }
            let Ok(files) = std::fs::read_dir(&signal_path) else {
                continue;
            };
            let keep_days = i64::try_from(keep(&name, signal)).unwrap_or(i64::MAX);
            for file in files.flatten() {
                let path = file.path();
                let Some(day) = file
                    .file_name()
                    .to_str()
                    .and_then(|n| n.strip_suffix(".jsonl"))
                    .and_then(clock::parse_date)
                else {
                    continue;
                };
                if !path.is_file() || today - day <= keep_days {
                    continue;
                }
                match std::fs::remove_file(&path) {
                    Ok(()) => removed.push(path),
                    Err(e) => trouble.push(format!("cannot remove {}: {e}", path.display())),
                }
            }
        }
    }
    removed.sort();
    (removed, trouble)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("rivet-monitor-tests")
            .join("store")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    const STATUS: &str = r#"{"subject":"status","phase":"converging","health":"healthy","mode":"kiosk","display_master":"rivet-launcher","display_leased":"aura-renderer","services":[{"name":"rivet-netd","health":"unknown"},{"name":"aura-renderer","health":"unknown"},{"name":"tempmon-rivet","health":"healthy"}]}"#;

    /// **§2.4 on `status`**: the named services stay, the others go, and every
    /// machine-level field is kept.
    #[test]
    fn the_services_filter_keeps_machine_level_fields() {
        let value: serde_json::Value = serde_json::from_str(STATUS).expect("json");
        let outcome = Outcome::Answered {
            raw: STATUS.to_string(),
            value,
        };
        let line = attempt_line(0, 12, &outcome, Some(&["aura-renderer".to_string()]));
        let read: serde_json::Value = serde_json::from_str(&line).expect("one JSON line");
        let answer = &read["answer"];
        for field in ["subject", "phase", "health", "mode", "display_master", "display_leased"] {
            assert_eq!(answer[field], serde_json::from_str::<serde_json::Value>(STATUS).unwrap()[field], "{field}");
        }
        let names: Vec<&str> = answer["services"]
            .as_array()
            .expect("services")
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["aura-renderer"]);
        // And the machine's own key order survives re-serialisation.
        assert!(line.contains(r#""answer":{"subject":"status","phase":"converging""#), "{line}");
    }

    /// Unfiltered, the answer is the machine's text, byte for byte.
    #[test]
    fn an_unfiltered_answer_is_stored_verbatim() {
        let outcome = Outcome::Answered {
            raw: STATUS.to_string(),
            value: serde_json::from_str(STATUS).unwrap(),
        };
        let line = attempt_line(1_789_462_143, 412, &outcome, None);
        assert_eq!(
            line,
            format!(
                r#"{{"captured_at":"2026-09-15T08:49:03Z","elapsed_ms":412,"outcome":"answered","answer":{STATUS}}}"#
            )
        );
        let missed = attempt_line(1_789_462_143, 20_000, &Outcome::Missed("no answer within 20 s".into()), None);
        assert_eq!(
            missed,
            r#"{"captured_at":"2026-09-15T08:49:03Z","elapsed_ms":20000,"outcome":"missed","reason":"no answer within 20 s"}"#
        );
    }

    /// **§5.3: a marker at twice the cadence**, and not at a hair under it.
    #[test]
    fn not_captured_is_owed_past_twice_the_cadence() {
        assert_eq!(gap(Some(1_000), 1_120, 60), None, "exactly twice is not past it");
        assert_eq!(gap(Some(1_000), 1_121, 60), Some((1_000, 1_121)));
        assert_eq!(gap(None, 1_000_000, 60), None, "no previous attempt, no stretch to explain");
        let line = marker_line(1_789_462_143, 1_789_465_743);
        let read: serde_json::Value = serde_json::from_str(&line).expect("json");
        assert_eq!(read["outcome"], "not-captured");
        assert_eq!(read["from"], "2026-09-15T08:49:03Z");
        assert_eq!(read["to"], "2026-09-15T09:49:03Z");
    }

    /// **Due-ness and the marker both need the last ATTEMPT from disk**, and a
    /// marker is not one.
    #[test]
    fn the_last_attempt_is_read_back_off_the_newest_file() {
        let folder = scratch("last-attempt");
        let older = day_file(&folder, "rivet-01", Signal::Thermal, clock::days_from_civil(2026, 9, 14));
        let newer = day_file(&folder, "rivet-01", Signal::Thermal, clock::days_from_civil(2026, 9, 15));
        append(&folder, "rivet-01", &older, &attempt_line(100, 1, &Outcome::Missed("x".into()), None)).unwrap();
        append(&folder, "rivet-01", &newer, &attempt_line(1_789_462_143, 1, &Outcome::Failed("y".into()), None)).unwrap();
        append(&folder, "rivet-01", &newer, &marker_line(1_789_462_143, 1_789_465_743)).unwrap();
        assert_eq!(last_attempt(&folder, "rivet-01", Signal::Thermal), Some(1_789_462_143));
        assert_eq!(last_attempt(&folder, "rivet-01", Signal::Status), None);
    }

    use crate::capture::LogAnswer;

    const KERNEL: &str = r#"{"ts":"2026-09-14T11:34:25.001Z","boot":"b1","mono":0.512,"sev":"info","svc":"kernel","corr":"k","actor":"kernel","msg":"Booting Linux","priority":6}"#;
    const RENDERER: &str = r#"{"ts":"2026-09-14T11:34:30.000Z","boot":"b1","mono":5.25,"sev":"warn","svc":"aura-renderer","corr":"a","actor":"uid:1000","msg":"a frame"}"#;
    const MCPD: &str = r#"{"ts":"2026-09-14T11:34:31.000Z","boot":"b1","mono":6.0,"sev":"info","svc":"rivet-mcpd","corr":"m","actor":"supervisor","msg":"TLS 1.3"}"#;

    fn answer(records: &[&str], cursor: Option<&str>) -> LogAnswer {
        LogAnswer {
            records: records.iter().map(|r| (*r).to_string()).collect(),
            cursor: cursor.map(str::to_string),
            boot_gone: false,
            since: None,
            held: Some(20_413),
            unreadable: Some(0),
        }
    }

    fn outcomes(lines: &[String]) -> Vec<String> {
        lines
            .iter()
            .map(|l| {
                let v: serde_json::Value = serde_json::from_str(l).expect("every line is JSON");
                v["outcome"].as_str().unwrap().to_string()
            })
            .collect()
    }

    /// **§5.2: one line per record**, each wrapped with the capture's own
    /// `captured_at`, the record byte for byte — and the attempt's line last,
    /// carrying the counts and the cursor instead of the records.
    #[test]
    fn a_log_answer_is_stored_one_line_per_record_with_captured_at() {
        let got = log_lines(1_789_462_143, 840, &answer(&[KERNEL, RENDERER, MCPD], Some("b1@6")), None, None);
        assert_eq!(outcomes(&got.lines), ["record", "record", "record", "answered"]);
        for (line, record) in got.lines.iter().zip([KERNEL, RENDERER, MCPD]) {
            assert_eq!(
                *line,
                format!(r#"{{"captured_at":"2026-09-15T08:49:03Z","outcome":"record","record":{record}}}"#)
            );
        }
        let attempt: serde_json::Value = serde_json::from_str(&got.lines[3]).unwrap();
        assert_eq!(attempt["captured_at"], "2026-09-15T08:49:03Z");
        assert_eq!(attempt["returned"], 3);
        assert_eq!(attempt["stored"], 3);
        assert_eq!(attempt["cursor"], "b1@6");
        assert_eq!(attempt["held"], 20_413);
        assert!(attempt.get("records").is_none() && attempt.get("answer").is_none());
        assert_eq!(got.stored, 3);
    }

    /// **§2.4 on `log`**: the named service is kept, **every kernel record is
    /// kept**, and nothing else is.
    #[test]
    fn the_log_filter_keeps_named_services_and_every_kernel_record() {
        let services = ["aura-renderer".to_string()];
        let got = log_lines(0, 1, &answer(&[KERNEL, RENDERER, MCPD], Some("b1@6")), None, Some(&services));
        let kept: Vec<String> = got.lines[..got.lines.len() - 1]
            .iter()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["record"]["svc"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(kept, ["kernel", "aura-renderer"]);
        let attempt: serde_json::Value = serde_json::from_str(got.lines.last().unwrap()).unwrap();
        assert_eq!(attempt["returned"], 3, "returned is BEFORE the filter — §6.3 judges on it");
        assert_eq!(attempt["stored"], 2, "stored is after it");

        let only_kernel = ["tempmon-rivet".to_string()];
        let got = log_lines(0, 1, &answer(&[KERNEL, RENDERER, MCPD], Some("b1@6")), None, Some(&only_kernel));
        assert_eq!(got.stored, 1, "a filter naming neither keeps the kernel regardless");
    }

    /// **A record string that is not valid JSON is kept as text, and the rest
    /// of the answer is stored as usual** — never dropped, never a failure of
    /// the whole capture.
    #[test]
    fn a_record_string_that_is_not_json_is_kept_as_text_and_the_rest_are_stored() {
        let broken = r#"{"boot":"b1","mono":5.5,"svc":"aura-renderer","msg":"cut off"#;
        let services = ["aura-renderer".to_string()];
        let got = log_lines(0, 1, &answer(&[KERNEL, broken, "42", MCPD, RENDERER], Some("b1@6")), None, Some(&services));
        assert_eq!(
            outcomes(&got.lines),
            ["record", "unreadable-record", "unreadable-record", "record", "answered"]
        );
        let kept: serde_json::Value = serde_json::from_str(&got.lines[1]).unwrap();
        assert_eq!(kept["text"], broken, "the string exactly as it came");
        assert_eq!(got.unreadable_records, 2);
        assert_eq!(got.stored, 4);
        let attempt: serde_json::Value = serde_json::from_str(got.lines.last().unwrap()).unwrap();
        assert_eq!(attempt["unreadable_records"], 2);
    }

    /// **§4.2's boot-gone answer is stored, with a `retention-gap` marker
    /// ahead of it** naming the cursor that could not be placed.
    #[test]
    fn a_boot_gone_answer_writes_a_retention_gap_before_its_records() {
        let mut gone = answer(&[KERNEL, RENDERER], Some("b1@5.25"));
        gone.boot_gone = true;
        gone.since = Some("boot-gone".to_string());
        let got = log_lines(0, 1, &gone, Some("b0@9999.5"), None);
        assert_eq!(outcomes(&got.lines), ["retention-gap", "record", "record", "answered"]);
        let marker: serde_json::Value = serde_json::from_str(&got.lines[0]).unwrap();
        assert_eq!(marker["after"], "b0@9999.5");
        let attempt: serde_json::Value = serde_json::from_str(got.lines.last().unwrap()).unwrap();
        assert_eq!(attempt["since"], "boot-gone");

        let plain = log_lines(0, 1, &answer(&[KERNEL], Some("b1@0.512")), Some("b1@0.1"), None);
        assert!(!outcomes(&plain.lines).contains(&"retention-gap".to_string()));
    }

    /// **The cursor moves only after the records are written**, a missing
    /// file is the first capture, and **an empty answer keeps the cursor**.
    #[test]
    fn the_cursor_is_written_after_the_lines_and_an_empty_answer_keeps_it() {
        let folder = scratch("cursor");
        assert_eq!(read_cursor(&folder, "rivet-01"), Ok(None), "no file is the first capture");
        let file = day_file(&folder, "rivet-01", Signal::Log, clock::days_from_civil(2026, 9, 15));

        let first = log_lines(0, 1, &answer(&[KERNEL], Some("b1@0.512")), None, None);
        commit_log(&folder, "rivet-01", &file, &first.lines, Some("b1@0.512")).unwrap();
        assert_eq!(read_cursor(&folder, "rivet-01"), Ok(Some("b1@0.512".to_string())));
        assert!(std::fs::read_to_string(&file).unwrap().contains("Booting Linux"));

        let idle = log_lines(60, 1, &answer(&[], None), Some("b1@0.512"), None);
        commit_log(&folder, "rivet-01", &file, &idle.lines, None).unwrap();
        assert_eq!(
            read_cursor(&folder, "rivet-01"),
            Ok(Some("b1@0.512".to_string())),
            "an empty answer keeps the cursor held"
        );
        assert!(!cursor_file(&folder, "rivet-01").with_file_name("cursor.tmp").exists());
    }

    /// **A write that fails leaves the cursor where it was** — §5.5. The day
    /// file is made a directory, so opening it for append fails.
    #[test]
    fn a_failed_write_does_not_advance_the_cursor() {
        let folder = scratch("cursor-write-fails");
        let today = day_file(&folder, "rivet-01", Signal::Log, clock::days_from_civil(2026, 9, 15));
        let first = log_lines(0, 1, &answer(&[KERNEL], Some("b1@0.512")), None, None);
        commit_log(&folder, "rivet-01", &today, &first.lines, Some("b1@0.512")).unwrap();

        let tomorrow = day_file(&folder, "rivet-01", Signal::Log, clock::days_from_civil(2026, 9, 16));
        std::fs::create_dir_all(&tomorrow).unwrap();
        let next = log_lines(60, 1, &answer(&[RENDERER], Some("b1@5.25")), Some("b1@0.512"), None);
        let failed = commit_log(&folder, "rivet-01", &tomorrow, &next.lines, Some("b1@5.25"));
        assert!(failed.is_err(), "{failed:?}");
        assert_eq!(
            read_cursor(&folder, "rivet-01"),
            Ok(Some("b1@0.512".to_string())),
            "the records were not written, so the cursor must not have moved past them"
        );
    }

    /// **A cursor file that exists and says nothing is not a first capture** —
    /// reading it as one would store the whole boot again.
    #[test]
    fn an_empty_cursor_file_is_refused_not_read_as_none() {
        let folder = scratch("cursor-empty");
        let path = cursor_file(&folder, "rivet-01");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "\n").unwrap();
        assert!(read_cursor(&folder, "rivet-01").is_err());
    }

    /// **The attempt line is last**, so due-ness and the `not-captured` marker
    /// still read the last log attempt off the disk after a backfill.
    #[test]
    fn the_last_log_attempt_is_found_after_its_records() {
        let folder = scratch("log-last-attempt");
        let file = day_file(&folder, "rivet-01", Signal::Log, clock::days_from_civil(2026, 9, 15));
        let many: Vec<&str> = std::iter::repeat_n(RENDERER, 2_000).collect();
        let got = log_lines(1_789_462_143, 1, &answer(&many, Some("b1@5.25")), None, None);
        commit_log(&folder, "rivet-01", &file, &got.lines, Some("b1@5.25")).unwrap();
        assert_eq!(last_attempt(&folder, "rivet-01", Signal::Log), Some(1_789_462_143));
    }

    /// **§5.4: retention by date, and never a directory this program did not
    /// create.**
    #[test]
    fn retention_deletes_by_date_and_leaves_a_foreign_directory_alone() {
        let folder = scratch("retention");
        let today = clock::days_from_civil(2026, 9, 15);
        let line = attempt_line(0, 1, &Outcome::Missed("x".into()), None);

        // Ours: claimed by writing through `append`.
        let old = day_file(&folder, "rivet-01", Signal::Status, today - 3);
        let edge = day_file(&folder, "rivet-01", Signal::Status, today - 2);
        let fresh = day_file(&folder, "rivet-01", Signal::Status, today);
        for path in [&old, &edge, &fresh] {
            append(&folder, "rivet-01", path, &line).unwrap();
        }
        // Not ours, though every name inside it looks like ours.
        let foreign = folder.join("somebody-else").join("status");
        std::fs::create_dir_all(&foreign).unwrap();
        let foreign_old = foreign.join(format!("{}.jsonl", clock::date_name(today - 100)));
        std::fs::write(&foreign_old, "{}\n").unwrap();
        // Inside ours, but not a signal directory, and not a dated file.
        let notes = folder.join("rivet-01").join("notes");
        std::fs::create_dir_all(&notes).unwrap();
        let note = notes.join(format!("{}.jsonl", clock::date_name(today - 100)));
        std::fs::write(&note, "{}\n").unwrap();
        let undated = folder.join("rivet-01").join("status").join("README.jsonl");
        std::fs::write(&undated, "{}\n").unwrap();

        let (removed, trouble) = retain(&folder, &|_, _| 2, &|_| 3, today);

        assert!(trouble.is_empty(), "{trouble:?}");
        assert_eq!(removed, vec![old.clone()]);
        assert!(!old.exists());
        assert!(edge.exists(), "two days old with keep-days = 2 is kept");
        assert!(fresh.exists());
        assert!(foreign_old.exists(), "a directory this program did not create is never touched");
        assert!(note.exists());
        assert!(undated.exists());
    }

    // -- the screenshot ------------------------------------------------------

    use crate::capture::Shot;

    const M: &str = "rivet-01";

    fn shot(holder: Option<&str>, ms: Option<i64>) -> Shot {
        Shot {
            holder: holder.map(str::to_string),
            board_path: holder.map(|h| format!("/state/app/{h}/data/screen.qoi")),
            frame_modified_ms: ms,
        }
    }

    /// Stage a frame the way the tool would, with some bytes in it.
    fn stage(folder: &Path) {
        prepare_staging(folder, M).expect("staging is ready");
        std::fs::write(screenshot_staging(folder, M), b"\x89PNG not really").unwrap();
    }

    /// Everything under `<folder>/rivet-01/screenshot` that is a PNG, relative.
    fn pngs(folder: &Path) -> Vec<String> {
        let dir = signal_dir(folder, M, Signal::Screenshot);
        let mut found = Vec::new();
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                for inner in std::fs::read_dir(&path).unwrap().flatten() {
                    found.push(format!("{}/{}", entry.file_name().to_string_lossy(), inner.file_name().to_string_lossy()));
                }
            } else if path.extension().is_some_and(|e| e == "png") {
                found.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        found.sort();
        found
    }

    /// One attempt through the store, as the worker does it: judge, place,
    /// write the index line, and remember what was stored only once the line
    /// is down. Returns the verdict.
    fn observe(folder: &Path, stored: &mut BTreeMap<String, i64>, seen: &Shot, at: u64, hhmmss: &str) -> Verdict {
        let day = clock::days_from_civil(2026, 9, 15);
        if seen.holder.is_some() {
            stage(folder);
        }
        let verdict = judge_frame(seen, stored);
        let placed = place_screenshot(folder, M, verdict, seen, day, hhmmss).expect("placed");
        let line = screenshot_line(at, 900, seen, verdict, placed.as_deref(), None);
        append(folder, M, &day_file(folder, M, Signal::Screenshot, day), &line).unwrap();
        if let (Verdict::Store, Some(holder), Some(ms)) = (verdict, &seen.holder, seen.frame_modified_ms) {
            stored.insert(holder.clone(), ms);
        }
        verdict
    }

    /// **§4.3's three file effects**: stored is renamed into the day
    /// directory; unchanged and no holder leave no PNG behind — and the index
    /// says which.
    #[test]
    fn stored_unchanged_and_no_holder_each_leave_the_right_files() {
        let folder = scratch("screenshot-effects");
        let mut stored = BTreeMap::new();
        let aura = Some("aura-renderer");

        assert_eq!(observe(&folder, &mut stored, &shot(aura, Some(1_000)), 1_789_462_143, "104903"), Verdict::Store);
        assert_eq!(pngs(&folder), ["2026-09-15/104903-aura-renderer.png"]);
        assert!(!screenshot_staging(&folder, M).exists(), "the staged frame was moved, not copied");

        assert_eq!(observe(&folder, &mut stored, &shot(aura, Some(1_000)), 1_789_462_173, "104933"), Verdict::Unchanged);
        assert_eq!(pngs(&folder), ["2026-09-15/104903-aura-renderer.png"], "unchanged stores nothing");
        assert!(!screenshot_staging(&folder, M).exists(), "and leaves no staged PNG behind");

        assert_eq!(observe(&folder, &mut stored, &shot(None, None), 1_789_462_203, "105003"), Verdict::NoHolder);
        assert_eq!(pngs(&folder).len(), 1);
        assert!(!screenshot_staging(&folder, M).exists());

        assert_eq!(observe(&folder, &mut stored, &shot(aura, Some(61_000)), 1_789_462_233, "105033"), Verdict::Store);
        assert_eq!(pngs(&folder), ["2026-09-15/104903-aura-renderer.png", "2026-09-15/105033-aura-renderer.png"]);

        let index = std::fs::read_to_string(day_file(&folder, M, Signal::Screenshot, clock::days_from_civil(2026, 9, 15))).unwrap();
        let lines: Vec<serde_json::Value> = index.lines().map(|l| serde_json::from_str(l).expect("JSON")).collect();
        let results: Vec<&str> = lines.iter().map(|l| l["result"].as_str().unwrap()).collect();
        assert_eq!(results, ["stored", "unchanged", "no application holds the display", "stored"]);
        assert_eq!(lines[0]["holder"], "aura-renderer");
        assert_eq!(lines[0]["frame_modified_ms"], 1_000);
        assert_eq!(lines[0]["file"], "2026-09-15/104903-aura-renderer.png");
        assert_eq!(lines[0]["outcome"], "answered", "an index line is an attempt line (§3.4, §5.3)");
        assert!(lines[1].get("file").is_none());
        assert_eq!(lines[2]["holder"], serde_json::Value::Null);
        assert_eq!(last_attempt(&folder, M, Signal::Screenshot), Some(1_789_462_233));
    }

    /// **The last stored timestamp per holder survives a restart**: a fresh
    /// read of the index says the same as the memory that wrote it, so the
    /// first attempt after a restart against an unchanged frame is `unchanged`.
    #[test]
    fn the_last_stored_timestamp_per_holder_survives_a_restart() {
        let folder = scratch("screenshot-restart");
        let mut before = BTreeMap::new();
        observe(&folder, &mut before, &shot(Some("aura-renderer"), Some(5_000)), 100, "000140");
        observe(&folder, &mut before, &shot(Some("rivet-launcher"), Some(7_000)), 130, "000210");
        observe(&folder, &mut before, &shot(Some("aura-renderer"), Some(9_000)), 160, "000240");
        observe(&folder, &mut before, &shot(Some("aura-renderer"), Some(9_000)), 190, "000310");

        let mut after = last_stored_frames(&folder, M);
        assert_eq!(after, before);
        assert_eq!(after["aura-renderer"], 9_000, "the LATER stored line for a holder wins");
        assert_eq!(observe(&folder, &mut after, &shot(Some("aura-renderer"), Some(9_000)), 220, "000340"), Verdict::Unchanged);
        assert_eq!(observe(&folder, &mut after, &shot(Some("rivet-launcher"), Some(7_000)), 250, "000410"), Verdict::Unchanged);
        assert!(last_stored_frames(&scratch("screenshot-nothing"), M).is_empty());
    }

    /// **Per holder**: a different holder whose frame carries the same
    /// timestamp is stored — it is a different picture.
    #[test]
    fn a_holder_change_with_an_equal_timestamp_is_stored() {
        let folder = scratch("screenshot-holder-change");
        let mut stored = BTreeMap::new();
        assert_eq!(observe(&folder, &mut stored, &shot(Some("aura-renderer"), Some(42)), 0, "000000"), Verdict::Store);
        assert_eq!(observe(&folder, &mut stored, &shot(Some("rivet-launcher"), Some(42)), 30, "000030"), Verdict::Store);
        assert_eq!(pngs(&folder).len(), 2);
    }

    /// A frame the board gave no time for cannot be shown unchanged, so it is
    /// kept — and is not remembered as a timestamp to compare against.
    #[test]
    fn a_frame_with_no_timestamp_is_stored_every_time() {
        let folder = scratch("screenshot-untimed");
        let mut stored = BTreeMap::new();
        for (at, hhmmss) in [(0, "000000"), (30, "000030")] {
            assert_eq!(observe(&folder, &mut stored, &shot(Some("aura-renderer"), None), at, hhmmss), Verdict::Store);
        }
        assert!(last_stored_frames(&folder, M).is_empty());
    }

    /// **A staged file left by an interrupted run is removed before the next
    /// fetch.**
    #[test]
    fn a_leftover_staged_frame_is_removed_before_the_next_fetch() {
        let folder = scratch("screenshot-leftover");
        let staging = screenshot_staging(&folder, M);
        assert_eq!(prepare_staging(&folder, M), Ok(false), "nothing left over the first time");
        std::fs::write(&staging, b"half a frame from a run that was killed").unwrap();
        assert_eq!(prepare_staging(&folder, M), Ok(true));
        assert!(!staging.exists());
        assert!(folder.join(M).join(CLAIM).is_file(), "preparing claims the machine directory");
    }

    /// A `Store` whose staged file is missing is a failure to store, never a
    /// `stored` line — and a stored frame is never overwritten.
    #[test]
    fn a_store_with_nothing_staged_or_a_taken_name_is_not_stored() {
        let folder = scratch("screenshot-not-stored");
        let day = clock::days_from_civil(2026, 9, 15);
        let seen = shot(Some("aura-renderer"), Some(1));
        prepare_staging(&folder, M).unwrap();
        let refused = place_screenshot(&folder, M, Verdict::Store, &seen, day, "120000").expect_err("nothing staged");
        assert!(refused.contains("wrote nothing"), "{refused}");
        let line: serde_json::Value = serde_json::from_str(&screenshot_line(0, 1, &seen, Verdict::Store, None, Some(&refused))).unwrap();
        assert_eq!(line["result"], "not stored");

        stage(&folder);
        place_screenshot(&folder, M, Verdict::Store, &seen, day, "120000").expect("stored");
        stage(&folder);
        let taken = place_screenshot(&folder, M, Verdict::Store, &seen, day, "120000").expect_err("taken");
        assert!(taken.contains("never overwritten"), "{taken}");
        assert!(!screenshot_staging(&folder, M).exists());
    }

    /// **§5.4 for screenshots: the newest `keep-frames` survive, across day
    /// directories** — and a day directory emptied by that goes, while one
    /// holding something this program did not name stays, and a machine
    /// directory it never claimed is never touched.
    #[test]
    fn retention_keeps_the_newest_frames_across_day_directories() {
        let folder = scratch("screenshot-retention");
        let today = clock::days_from_civil(2026, 9, 15);
        let mut stored = BTreeMap::new();
        for (offset, ms) in [(4, 1), (3, 2), (2, 3), (0, 4)] {
            stage(&folder);
            let seen = shot(Some("aura-renderer"), Some(ms));
            place_screenshot(&folder, M, judge_frame(&seen, &stored), &seen, today - offset, "080000").unwrap();
            stored.insert("aura-renderer".into(), ms);
        }
        let day_dir = |offset: i64| signal_dir(&folder, M, Signal::Screenshot).join(clock::date_name(today - offset));
        // Something somebody else put in an old day directory.
        std::fs::write(day_dir(4).join("notes.txt"), "mine").unwrap();
        // A whole machine directory this program never claimed, dated like ours.
        let foreign = folder.join("somebody-else").join("screenshot").join(clock::date_name(today - 30));
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::write(foreign.join("080000-x.png"), "theirs").unwrap();

        // keep-days is generous; keep-frames is 2, and it is what decides.
        let (removed, trouble) = retain(&folder, &|_, _| 90, &|_| 2, today);

        assert!(trouble.is_empty(), "{trouble:?}");
        assert!(day_dir(2).join("080000-aura-renderer.png").exists(), "the second newest frame stays");
        assert!(day_dir(0).join("080000-aura-renderer.png").exists(), "and the newest");
        assert!(!day_dir(3).join("080000-aura-renderer.png").exists(), "the third newest goes");
        assert!(!day_dir(3).exists(), "and its day directory, which that left empty");
        assert!(removed.contains(&day_dir(3)));
        assert!(!day_dir(4).join("080000-aura-renderer.png").exists(), "so does the oldest");
        assert!(day_dir(4).join("notes.txt").exists(), "a file this program did not name stays");
        assert!(day_dir(4).exists(), "and so does the directory holding it");
        assert!(foreign.join("080000-x.png").exists(), "an unclaimed machine directory is never touched");
        let frames: Vec<String> = pngs(&folder).into_iter().filter(|n| n.ends_with(".png")).collect();
        assert_eq!(
            frames,
            vec![
                format!("{}/080000-aura-renderer.png", clock::date_name(today - 2)),
                format!("{}/080000-aura-renderer.png", clock::date_name(today)),
            ],
            "exactly keep-frames remain, and they are the newest"
        );
    }

    /// **§5.4: `keep-frames` counts frames, and `keep-days` still retires the
    /// screenshot INDEX** — the two are different retentions over one directory.
    #[test]
    fn the_screenshot_index_is_still_retired_by_date() {
        let folder = scratch("screenshot-index-retention");
        let today = clock::days_from_civil(2026, 9, 15);
        let line = attempt_line(0, 1, &Outcome::Missed("x".into()), None);
        let old = day_file(&folder, M, Signal::Screenshot, today - 20);
        let fresh = day_file(&folder, M, Signal::Screenshot, today);
        for path in [&old, &fresh] {
            append(&folder, M, path, &line).unwrap();
        }
        stage(&folder);
        let seen = shot(Some("aura-renderer"), Some(1));
        place_screenshot(&folder, M, Verdict::Store, &seen, today - 20, "080000").unwrap();

        let (_, trouble) = retain(&folder, &|_, _| 14, &|_| 3, today);

        assert!(trouble.is_empty(), "{trouble:?}");
        assert!(!old.exists(), "an index file older than keep-days goes");
        assert!(fresh.exists());
        assert_eq!(
            pngs(&folder).iter().filter(|n| n.ends_with(".png")).count(),
            1,
            "and the one frame stays, because keep-frames is 3 and its age is not what counts"
        );
    }

    /// **§4.3 under counted retention**: a `stored` line whose PNG has been
    /// retired is not read back, so nothing is ever called `unchanged` against
    /// a frame that is gone.
    #[test]
    fn a_stored_line_whose_frame_was_retired_is_not_read_back() {
        let folder = scratch("stored-frame-retired");
        let day = clock::days_from_civil(2026, 9, 15);
        let file = day_file(&folder, M, Signal::Screenshot, day);
        let mut stored = BTreeMap::new();
        for (ms, at) in [(11_i64, "080000"), (22, "080100")] {
            stage(&folder);
            let seen = shot(Some("aura-renderer"), Some(ms));
            let name = place_screenshot(&folder, M, Verdict::Store, &seen, day, at).unwrap();
            append(&folder, M, &file, &screenshot_line(0, 1, &seen, Verdict::Store, name.as_deref(), None)).unwrap();
            stored.insert("aura-renderer".to_string(), ms);
        }
        assert_eq!(last_stored_frames(&folder, M).get("aura-renderer"), Some(&22));

        retain(&folder, &|_, _| 14, &|_| 1, day);
        assert_eq!(
            last_stored_frames(&folder, M).get("aura-renderer"),
            Some(&22),
            "the newest frame survives keep-frames = 1, so it is still what `unchanged` compares against"
        );

        // Now retire it too, as a sweep would if that holder stopped holding
        // the display and other frames pushed it out.
        std::fs::remove_file(signal_dir(&folder, M, Signal::Screenshot).join(screenshot_name(day, "080100", "aura-renderer"))).unwrap();
        assert!(
            last_stored_frames(&folder, M).is_empty(),
            "a stored line with no PNG behind it says nothing"
        );
    }
}
