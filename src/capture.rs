//! **One capture, and what its outcome was** — `SPEC-02` §4.1 (the spot
//! readings), §4.2 (the log, read by [`read_log`]) and §5.2.
//!
//! `cargo rivet run --host … --identity … --pin … -- get <subject>`, through
//! `crate::tool`'s bounded call (so `CREATE_NO_WINDOW` and the kill at
//! `timeout-seconds` both hold). No capture talks to a board directly.
//!
//! # `missed` and `failed` are different facts, and the line between them
//!
//! A **miss** is the machine not answering: nothing came back in time, or the
//! connection could not be made or was cut. It is a statement about the
//! MACHINE, and `SPEC-02` §6.2 counts misses toward `reach/`.
//!
//! A **failure** is this side's problem, and says nothing about the machine:
//! the tool would not start, this end's own identity would not load, the
//! machine answered something that is not the subject asked for, or refused
//! the call.
//!
//! **`cargo-rivet` exits 1 for every error**, so the exit status cannot draw
//! that line. Its wording can, because `rivet-remote`'s transport composes a
//! small, fixed set of sentences for a connection that failed — and the line is
//! drawn on those, in [`classify`]:
//!
//! * **killed at the limit** → `missed`, whatever it printed;
//! * a TRUST refusal — `invalid peer certificate`, a TLS `alert`, this end's own
//!   TLS identity — → `failed`. Something answered, and this side's credential
//!   or pin disagrees with it; the machine being *gone* is not what that says;
//! * a TRANSPORT sentence — `cannot reach`, `cannot resolve`, `cannot send to`,
//!   `cannot end the request`, `could not be read while`, `closed the connection
//!   before`, `answered nothing for call` → `missed`;
//! * **anything else** → `failed`, carrying the tool's words. Unrecognised
//!   output is this build not understanding its tool, which is this side's
//!   problem — the same fail-closed posture as `SPEC-01` §3.6's plan parser. The
//!   price is that a reworded transport sentence in a future `cargo-rivet`
//!   turns misses into failures; the test of the phrases is what notices.

use std::path::Path;
use std::time::Duration;

use crate::tool::{self, Finished, Target};

/// §5.2's three outcomes of an attempt.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The machine's answer, as it printed it and as parsed.
    Answered {
        raw: String,
        value: serde_json::Value,
    },
    /// No answer: the reason, in words.
    Missed(String),
    /// This side's problem: the reason, in words.
    Failed(String),
}

impl Outcome {
    /// The word §5.2 writes.
    #[must_use]
    pub const fn word(&self) -> &'static str {
        match self {
            Self::Answered { .. } => "answered",
            Self::Missed(_) => "missed",
            Self::Failed(_) => "failed",
        }
    }
}

/// One attempt, timed.
#[derive(Debug, Clone, PartialEq)]
pub struct Attempt {
    pub outcome: Outcome,
    pub elapsed_ms: u64,
}

/// **Ask one machine one subject**, with the limit enforced.
#[must_use]
pub fn spot(
    tool_path: &Path,
    target: &Target,
    identity: &Path,
    subject: &str,
    limit: Duration,
) -> Attempt {
    let args = tool::get_args(target, identity, subject);
    attempt(tool::command(tool_path, &args), subject, limit)
}

/// **One log capture** — `SPEC-02` §4.2: `get log latest all` with no cursor,
/// `get log all all <cursor>` with one. Judged like a spot reading (the answer
/// must be JSON carrying `"subject":"log"`); [`read_log`] then reads it as a log.
#[must_use]
pub fn log(
    tool_path: &Path,
    target: &Target,
    identity: &Path,
    cursor: Option<&str>,
    limit: Duration,
) -> Attempt {
    let args = tool::log_args(target, identity, cursor);
    attempt(tool::command(tool_path, &args), "log", limit)
}

/// A `get log` answer, read — `SPEC-02` §4.2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogAnswer {
    /// Each record as the machine wrote it: the JSON text **inside** each
    /// string of `records`, oldest first. Not yet parsed — a string that is not
    /// a JSON object is kept, not dropped (`crate::store::log_lines`).
    pub records: Vec<String>,
    /// The answer's own `cursor`: the position after the last record returned.
    /// `None` when nothing was returned, which means **keep the one held**.
    pub cursor: Option<String>,
    /// `since` said `boot-gone`: the board's retention dropped the cursor's
    /// boot, so everything it holds came back and some records between are
    /// lost for good.
    pub boot_gone: bool,
    /// What `since` said, verbatim, when a cursor was sent.
    pub since: Option<String>,
    /// `held` and `unreadable`, carried into the attempt line.
    pub held: Option<u64>,
    pub unreadable: Option<u64>,
}

/// **Read a `get log` answer as a log**, or say why it cannot be.
///
/// **Fail-closed where the cursor is concerned**, because the cursor is what
/// decides whether the next capture repeats or skips: an answer that returned
/// records and names no cursor, a `returned` that disagrees with `records`,
/// or — when a cursor was sent — a `since` that is neither `at` nor
/// `boot-gone` is a `failed` attempt, nothing is stored, and the cursor held
/// stays where it was. The next capture asks again from the same place.
///
/// # Errors
///
/// Any of those, in words.
pub fn read_log(value: &serde_json::Value, sent_cursor: bool) -> Result<LogAnswer, String> {
    let listed = value
        .get("records")
        .and_then(serde_json::Value::as_array)
        .ok_or("the `get log` answer carries no `records` array, so there is nothing to store and no way to tell an idle machine from an answer this build cannot read")?;
    let records: Vec<String> = listed
        .iter()
        .map(|record| match record {
            serde_json::Value::String(text) => text.clone(),
            // Not the shape Rivet sends. Kept as its JSON text, so the store
            // treats it like any other record it can or cannot read.
            other => other.to_string(),
        })
        .collect();
    if let Some(returned) = value.get("returned").and_then(serde_json::Value::as_u64)
        && usize::try_from(returned).ok() != Some(records.len())
    {
        return Err(format!(
            "the `get log` answer says `returned` {returned} and carries {} record(s). An answer \
             that disagrees with itself is not stored, and the cursor is not moved.",
            records.len()
        ));
    }
    let cursor = match value.get("cursor") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(text)) if !text.is_empty() => Some(text.clone()),
        Some(other) => {
            return Err(format!(
                "the `get log` answer's `cursor` is {other}, which is not a cursor. Nothing is \
                 stored and the cursor held is not moved."
            ));
        }
    };
    if !records.is_empty() && cursor.is_none() {
        return Err(format!(
            "the `get log` answer returned {} record(s) and no `cursor`. Storing them without one \
             would fetch them all again next time, so nothing is stored and the cursor held is \
             not moved.",
            records.len()
        ));
    }
    let since = value
        .get("since")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let boot_gone = match (sent_cursor, since.as_deref()) {
        (false, _) | (true, Some("at")) => false,
        (true, Some("boot-gone")) => true,
        (true, said) => {
            return Err(format!(
                "a cursor was sent and the answer's `since` is {}, which is neither `at` nor \
                 `boot-gone`. This build cannot tell whether the cursor was placed, so it cannot \
                 tell new records from repeated ones; nothing is stored and the cursor held is \
                 not moved.",
                said.map_or_else(|| "absent".to_string(), |s| format!("{s:?}"))
            ));
        }
    };
    Ok(LogAnswer {
        records,
        cursor,
        boot_gone,
        since,
        held: value.get("held").and_then(serde_json::Value::as_u64),
        unreadable: value.get("unreadable").and_then(serde_json::Value::as_u64),
    })
}

/// **One screenshot capture** — `SPEC-02` §4.3: `cargo rivet screenshot
/// --holder --json --out <staging>`, with the limit enforced.
///
/// Killed at the limit is `missed`, and a non-zero exit is drawn by
/// [`classify`] exactly as a spot reading's is — the verb reaches the machine
/// through the same transport and says the same sentences. An exit 0 must
/// print ONE JSON object; [`read_screenshot`] then reads it.
#[must_use]
pub fn screenshot(
    tool_path: &Path,
    target: &Target,
    identity: &Path,
    staging: &Path,
    limit: Duration,
) -> Attempt {
    let args = tool::screenshot_args(target, identity, staging);
    let started = std::time::Instant::now();
    match tool::run_bounded(tool::command(tool_path, &args), limit) {
        Ok(finished) => Attempt {
            elapsed_ms: millis(finished.elapsed),
            outcome: judge_screenshot(&finished, limit),
        },
        Err(why) => Attempt {
            elapsed_ms: millis(started.elapsed()),
            outcome: Outcome::Failed(why),
        },
    }
}

/// What a finished `screenshot --json` child means — before its object is
/// read as a screenshot.
#[must_use]
pub fn judge_screenshot(finished: &Finished, limit: Duration) -> Outcome {
    if finished.timed_out {
        return Outcome::Missed(format!("no answer within {} s", limit.as_secs()));
    }
    if !finished.success {
        let said = format!("{}\n{}", finished.stderr.trim(), finished.stdout.trim());
        return classify(said.trim());
    }
    let raw = finished.stdout.trim();
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value) if value.is_object() => Outcome::Answered {
            raw: raw.to_string(),
            value,
        },
        _ => Outcome::Failed(format!(
            "`cargo rivet screenshot --json` exited 0 and did not print one JSON object, so this \
             build cannot tell what it fetched. It began: {}",
            raw.chars().take(200).collect::<String>()
        )),
    }
}

/// `--json`'s `outcome` when a frame was fetched and written.
pub const FETCHED: &str = "fetched";
/// `--json`'s `outcome` when no application holds the display.
pub const NO_HOLDER: &str = "no-application-holds-the-display";

/// **One screenshot answer, read** — `SPEC-02` §4.3's three facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shot {
    /// The application holding the display; `None` when nothing holds it.
    pub holder: Option<String>,
    /// The frame's path on the machine, as the tool reported it. Carried into
    /// the index; never composed here (`ADR-002` §4).
    pub board_path: Option<String>,
    /// The frame's own timestamp, **by the board's clock**. Compared only for
    /// equality with another board value, never with the PC's clock.
    pub frame_modified_ms: Option<i64>,
}

/// **Read `screenshot --json`'s object**, or say why it cannot be.
///
/// **Fail-closed**, because what this decides is whether a frame is kept: the
/// object must carry all five keys the tool promises (`outcome`, `holder`,
/// `board_path`, `frame_modified_ms`, `out`), and must agree with itself —
/// `fetched` names a holder, a board path and the file written; the no-holder
/// outcome names no holder and no frame. A holder that could not be a file
/// name is refused too, since it becomes one (`SPEC-02` §5.1).
///
/// # Errors
///
/// Any of those, in words.
pub fn read_screenshot(value: &serde_json::Value) -> Result<Shot, String> {
    let refuse = |why: String| {
        Err(format!(
            "the `screenshot --json` answer {why}, so nothing is stored from it: {value}"
        ))
    };
    for key in ["outcome", "holder", "board_path", "frame_modified_ms", "out"] {
        if value.get(key).is_none() {
            return refuse(format!("carries no `{key}`"));
        }
    }
    let text = |key: &str| -> Result<Option<String>, String> {
        match &value[key] {
            serde_json::Value::Null => Ok(None),
            serde_json::Value::String(text) if !text.is_empty() => Ok(Some(text.clone())),
            other => Err(format!("has `{key}` {other}, which is neither a string nor null")),
        }
    };
    let (holder, board_path, out) = match (text("holder"), text("board_path"), text("out")) {
        (Ok(h), Ok(b), Ok(o)) => (h, b, o),
        (Err(why), _, _) | (_, Err(why), _) | (_, _, Err(why)) => return refuse(why),
    };
    let frame_modified_ms = match &value["frame_modified_ms"] {
        serde_json::Value::Null => None,
        number => match number.as_i64() {
            Some(ms) => Some(ms),
            None => {
                return refuse(format!(
                    "has `frame_modified_ms` {number}, which is not a whole number of milliseconds"
                ));
            }
        },
    };
    match value["outcome"].as_str() {
        Some(FETCHED) => {
            let Some(name) = holder else {
                return refuse("says `fetched` and names no holder, and `--holder` was asked".to_string());
            };
            if name == "."
                || name == ".."
                || name.chars().any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
            {
                return refuse(format!("names the holder {name:?}, which cannot be a file name"));
            }
            if board_path.is_none() || out.is_none() {
                return refuse("says `fetched` and names no board path or no file written".to_string());
            }
            Ok(Shot {
                holder: Some(name),
                board_path,
                frame_modified_ms,
            })
        }
        Some(NO_HOLDER) => {
            if holder.is_some() || board_path.is_some() || out.is_some() || frame_modified_ms.is_some() {
                return refuse("says nothing holds the display and still names a holder or a frame".to_string());
            }
            Ok(Shot {
                holder: None,
                board_path: None,
                frame_modified_ms: None,
            })
        }
        _ => refuse(format!("has the outcome {}, which this build does not know", value["outcome"])),
    }
}

/// [`spot`] with the command already built — so a test can hand it a child
/// that never answers and watch it be killed and counted.
#[must_use]
pub fn attempt(command: std::process::Command, subject: &str, limit: Duration) -> Attempt {
    let started = std::time::Instant::now();
    match tool::run_bounded(command, limit) {
        Ok(finished) => Attempt {
            elapsed_ms: millis(finished.elapsed),
            outcome: judge(&finished, subject, limit),
        },
        Err(why) => Attempt {
            elapsed_ms: millis(started.elapsed()),
            outcome: Outcome::Failed(why),
        },
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// **What a finished child means.**
#[must_use]
pub fn judge(finished: &Finished, subject: &str, limit: Duration) -> Outcome {
    if finished.timed_out {
        return Outcome::Missed(format!("no answer within {} s", limit.as_secs()));
    }
    if !finished.success {
        let said = format!("{}\n{}", finished.stderr.trim(), finished.stdout.trim());
        return classify(said.trim());
    }
    read_answer(&finished.stdout, subject)
}

/// §4.1: **the answer must parse as JSON carrying the `subject` asked for**,
/// or it is a failure and never a stored reading.
fn read_answer(stdout: &str, subject: &str) -> Outcome {
    let raw = stdout.trim();
    let value: serde_json::Value = match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(e) => {
            return Outcome::Failed(format!(
                "the answer to `get {subject}` is not JSON ({e}), so it is not stored as a \
                 reading. It began: {}",
                raw.chars().take(200).collect::<String>()
            ));
        }
    };
    match value.get("subject").and_then(serde_json::Value::as_str) {
        Some(said) if said == subject => Outcome::Answered {
            raw: raw.to_string(),
            value,
        },
        Some(said) => Outcome::Failed(format!(
            "`get {subject}` was answered about `{said}`, which is a different question, so it \
             is not stored as a reading"
        )),
        None => Outcome::Failed(format!(
            "the answer to `get {subject}` carries no `subject`, so it cannot be shown to be the \
             answer asked for and is not stored as a reading"
        )),
    }
}

/// Trust refusals: something answered, and this side would not trust it or
/// could not present itself. Checked BEFORE the transport phrases, because a
/// trust refusal arrives inside one (`cannot send to …: invalid peer
/// certificate`).
const TRUST: &[&str] = &[
    "invalid peer certificate",
    "received fatal alert",
    "own TLS identity or pins",
    "TLS seed",
];

/// `rivet-remote`'s sentences for a connection that did not carry an answer.
const TRANSPORT: &[&str] = &[
    "cannot reach",
    "cannot resolve",
    "resolved to no address",
    "cannot send to",
    "cannot end the request",
    "could not be read while",
    "closed the connection before",
    "answered nothing for call",
];

/// The module note's line, applied to what a non-zero exit printed.
#[must_use]
pub fn classify(said: &str) -> Outcome {
    let text = if said.is_empty() {
        "cargo-rivet exited with an error and printed nothing".to_string()
    } else {
        said.to_string()
    };
    if TRUST.iter().any(|phrase| said.contains(phrase)) {
        return Outcome::Failed(text);
    }
    if TRANSPORT.iter().any(|phrase| said.contains(phrase)) {
        return Outcome::Missed(text);
    }
    Outcome::Failed(text)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn finished(success: bool, stdout: &str, stderr: &str) -> Finished {
        Finished {
            success,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            timed_out: false,
            elapsed: Duration::from_millis(80),
        }
    }

    /// A child that never answers. `ping` on Windows, `sleep` elsewhere —
    /// either is a process with no grandchild, so killing it closes its pipes.
    fn never_answers() -> std::process::Command {
        if cfg!(windows) {
            let mut command = std::process::Command::new("ping");
            command.args(["-n", "30", "127.0.0.1"]);
            command
        } else {
            let mut command = std::process::Command::new("sleep");
            command.arg("30");
            command
        }
    }

    /// **§3.3: a child still running at the limit is KILLED, and the capture
    /// is a miss** — with the limit in the reason, and the worker back well
    /// before the child would have finished on its own.
    #[test]
    fn a_child_killed_at_the_timeout_is_a_miss() {
        let started = std::time::Instant::now();
        let attempt = attempt(never_answers(), "storage", Duration::from_secs(1));
        assert_eq!(
            attempt.outcome,
            Outcome::Missed("no answer within 1 s".to_string())
        );
        assert!(attempt.elapsed_ms >= 1_000, "{attempt:?}");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the worker waited {:?} for a child that was meant to be killed at 1 s",
            started.elapsed()
        );
    }

    /// **§4.1.** The subject asked for, in JSON, is a reading.
    #[test]
    fn json_carrying_the_asked_subject_is_answered() {
        let out = judge(
            &finished(true, "{\"subject\":\"thermal\",\"zones\":[]}\n", ""),
            "thermal",
            Duration::from_secs(20),
        );
        assert_eq!(out.word(), "answered");
    }

    /// **§4.1.** Anything else is a failure, never a stored reading.
    #[test]
    fn an_answer_that_is_not_the_asked_subject_is_failed() {
        for stdout in [
            "not json at all",
            "{\"subject\":\"memory\"}",
            "{\"zones\":[]}",
            "{\"subject\":\"thermal\"}\n\n--- CUT: 65536 of 90000 bytes shown ---",
        ] {
            let out = judge(&finished(true, stdout, ""), "thermal", Duration::from_secs(20));
            assert_eq!(out.word(), "failed", "{stdout}: {out:?}");
        }
    }

    /// **The line between missed and failed**, on the sentences this build of
    /// `cargo-rivet` actually prints. The first is measured, 2026-09-15, against
    /// a closed local port.
    #[test]
    fn transport_sentences_are_misses_and_everything_else_is_a_failure() {
        let missed = [
            "cargo-rivet: cannot reach 127.0.0.1:1: No connection could be made because the target machine actively refused it. (os error 10061)",
            "cargo-rivet: rivet-01 closed the connection before the `rivet_get` call was complete.",
            "cargo-rivet: 192.168.50.10:7777 could not be read while the `rivet_get` call was in flight, after 0 byte(s): timed out",
            "cargo-rivet: 192.168.50.10:7777 answered nothing for call 2 of 1.",
        ];
        for said in missed {
            assert_eq!(classify(said).word(), "missed", "{said}");
        }
        let failed = [
            "cargo-rivet: cannot read this machine's TLS seed at D:/x\\device.seed: The system cannot find the path specified. (os error 3).",
            "cargo-rivet: cannot send to 192.168.50.10:7777: invalid peer certificate: BadSignature",
            "cargo-rivet: 192.168.50.10:7777 could not be read while the `rivet_get` call was in flight, after 0 byte(s): received fatal alert: CertificateRequired",
            "cargo-rivet: the machine refused the call: \"error\":{\"code\":-32602}",
            "cargo-rivet: `get` has no subject `slott`",
            "",
        ];
        for said in failed {
            assert_eq!(classify(said).word(), "failed", "{said}");
        }
    }

    fn log_answer(text: &str) -> serde_json::Value {
        serde_json::from_str(text).expect("a JSON answer")
    }

    /// **§4.2: an answer with no records and no cursor is a success that
    /// stores nothing** — and its cursor is `None`, which means *keep the one
    /// held*, whether Rivet omits the field or sends `null`.
    #[test]
    fn an_empty_log_answer_reads_as_no_new_cursor() {
        for text in [
            r#"{"subject":"log","boot":"all","records":[],"returned":0,"held":9,"since":"at"}"#,
            r#"{"subject":"log","boot":"all","records":[],"returned":0,"held":9,"since":"at","cursor":null}"#,
        ] {
            let read = read_log(&log_answer(text), true).expect(text);
            assert!(read.records.is_empty());
            assert_eq!(read.cursor, None, "{text}");
            assert!(!read.boot_gone);
        }
    }

    /// **`boot-gone` is read as such**, and only when a cursor was sent.
    #[test]
    fn a_boot_gone_answer_is_read_as_a_retention_gap() {
        let text = r#"{"subject":"log","boot":"all","records":["{\"boot\":\"b\",\"mono\":1.0,\"svc\":\"kernel\"}"],"returned":1,"cursor":"b@1","since":"boot-gone"}"#;
        let read = read_log(&log_answer(text), true).expect("readable");
        assert!(read.boot_gone);
        assert_eq!(read.cursor.as_deref(), Some("b@1"));
    }

    /// **Fail-closed on everything the cursor depends on.**
    #[test]
    fn a_log_answer_the_cursor_cannot_be_trusted_from_is_failed() {
        let record = r#""{\"boot\":\"b\",\"mono\":1.0,\"svc\":\"kernel\"}""#;
        let refused = [
            // records and no cursor
            (format!(r#"{{"subject":"log","records":[{record}],"returned":1}}"#), false),
            // returned disagrees with records
            (format!(r#"{{"subject":"log","records":[{record}],"returned":2,"cursor":"b@1"}}"#), false),
            // a cursor was sent and since is absent
            (format!(r#"{{"subject":"log","records":[{record}],"returned":1,"cursor":"b@1"}}"#), true),
            // a cursor was sent and since is a word this build does not know
            (format!(r#"{{"subject":"log","records":[{record}],"returned":1,"cursor":"b@1","since":"reset"}}"#), true),
            // no records array
            (r#"{"subject":"log","returned":0}"#.to_string(), false),
        ];
        for (text, sent) in refused {
            assert!(read_log(&log_answer(&text), sent).is_err(), "{text}");
        }
    }

    /// An answer as a `--holder` build of `cargo rivet screenshot` prints it.
    const FETCHED_ANSWER: &str = r#"{"outcome":"fetched","holder":"aura-renderer","board_path":"/state/app/aura-renderer/data/screen.qoi","frame_modified_ms":1789492160748,"out":"D:\\m\\rivet-01\\screenshot\\staging.png"}"#;
    const NO_HOLDER_ANSWER: &str = r#"{"outcome":"no-application-holds-the-display","holder":null,"board_path":null,"frame_modified_ms":null,"out":null}"#;

    /// **§4.3: each outcome of the JSON, read.**
    #[test]
    fn each_screenshot_outcome_is_read() {
        let fetched = read_screenshot(&log_answer(FETCHED_ANSWER)).expect("fetched");
        assert_eq!(
            fetched,
            Shot {
                holder: Some("aura-renderer".into()),
                board_path: Some("/state/app/aura-renderer/data/screen.qoi".into()),
                frame_modified_ms: Some(1_789_492_160_748),
            }
        );
        let nobody = read_screenshot(&log_answer(NO_HOLDER_ANSWER)).expect("no holder");
        assert_eq!(nobody.holder, None);
        assert_eq!(nobody.frame_modified_ms, None);

        // A frame the board gave no time for is still a frame.
        let untimed = FETCHED_ANSWER.replace("1789492160748", "null");
        assert_eq!(read_screenshot(&log_answer(&untimed)).expect("untimed").frame_modified_ms, None);

        // And through the child's judgement: exit 0 with the object is answered.
        let out = judge_screenshot(&finished(true, &format!("{FETCHED_ANSWER}\n"), ""), Duration::from_secs(20));
        assert_eq!(out.word(), "answered");
    }

    /// **§4.3: an answer that is not JSON, or is incomplete or disagrees with
    /// itself, is `failed`** — never a stored frame.
    #[test]
    fn a_screenshot_answer_that_is_not_json_or_is_incomplete_is_failed() {
        for stdout in ["holder   aura-renderer holds the display", "", "[1,2]", "{\"outcome\":\"fetched\"} trailing"] {
            let out = judge_screenshot(&finished(true, stdout, ""), Duration::from_secs(20));
            assert_eq!(out.word(), "failed", "{stdout:?}: {out:?}");
        }
        let refused = [
            FETCHED_ANSWER.replace(r#","out":"D:\\m\\rivet-01\\screenshot\\staging.png""#, ""),
            FETCHED_ANSWER.replace(r#""holder":"aura-renderer""#, r#""holder":null"#),
            FETCHED_ANSWER.replace(r#""board_path":"/state/app/aura-renderer/data/screen.qoi""#, r#""board_path":null"#),
            FETCHED_ANSWER.replace("aura-renderer\"", "../evil\""),
            FETCHED_ANSWER.replace("1789492160748", "\"1789492160748\""),
            FETCHED_ANSWER.replace("1789492160748", "1.5"),
            FETCHED_ANSWER.replace("\"fetched\"", "\"pressed\""),
            NO_HOLDER_ANSWER.replace(r#""holder":null"#, r#""holder":"aura-renderer""#),
            NO_HOLDER_ANSWER.replace(r#","frame_modified_ms":null"#, ""),
            "{}".to_string(),
        ];
        for text in refused {
            assert!(read_screenshot(&log_answer(&text)).is_err(), "{text}");
        }
    }

    /// **§4.3 uses §4.1's line**: killed is a miss, a transport sentence is a
    /// miss, anything else a failure.
    #[test]
    fn a_screenshot_that_did_not_answer_is_classified_like_every_capture() {
        let mut killed = finished(false, "", "");
        killed.timed_out = true;
        assert_eq!(judge_screenshot(&killed, Duration::from_secs(20)), Outcome::Missed("no answer within 20 s".into()));
        let refused = finished(false, "", "cargo-rivet: cannot reach 192.168.50.10:7777: timed out");
        assert_eq!(judge_screenshot(&refused, Duration::from_secs(20)).word(), "missed");
        let trust = finished(false, "", "cargo-rivet: cannot send to 192.168.50.10:7777: invalid peer certificate: BadSignature");
        assert_eq!(judge_screenshot(&trust, Duration::from_secs(20)).word(), "failed");
    }

    /// A tool that will not start is this side's problem.
    #[test]
    fn a_tool_that_will_not_start_is_failed() {
        let attempt = spot(
            Path::new("D:/no/such/cargo-rivet.exe"),
            &Target {
                name: "rivet-01".into(),
                address: "127.0.0.1".into(),
                control_port: 1,
                pin: "aa".into(),
            },
            Path::new("./id"),
            "status",
            Duration::from_secs(5),
        );
        assert_eq!(attempt.outcome.word(), "failed", "{attempt:?}");
    }
}
