//! **`cargo rivet backup`, and proving the binary can run it.**
//!
//! `ADR-001` §4: Rivet is a dependency, not a place to fix things. Nothing
//! here fetches, validates or names a manifest key — it composes a command
//! line, runs it, and reads what came back.
//!
//! # The preflight exists because of a real trap
//!
//! The `cargo-rivet` on a workstation's PATH is often an old build — Rivet
//! 0.1.66's has nine verbs and `backup` is not one — while the build that has
//! it is elsewhere. A program
//! that invoked the PATH binary would get *"`backup` is not a `cargo rivet`
//! verb"* on every run and could very reasonably report "no backups
//! configured" — a silent failure of exactly the class this whole design is
//! written against. So the path is configuration, and [`preflight`] runs the
//! binary and makes it prove the verb before anything is scheduled.
//!
//! The check is a POSITIVE assertion — the binary must print `backup`'s own
//! usage — rather than the absence of an error string, because an absence is
//! what a binary that changed its wording also produces.
//!
//! # `--service` is proved the same way, and for the same reason
//!
//! `Rivet ADR-051` §8.5 gave `cargo rivet backup` a repeatable
//! `--service NAME` filter, which is what lets an
//! `exempt` application mean *never fetched* instead of *never kept*
//! (`SPEC-01` §3.5's History). But a `cargo-rivet` built with `backup` and
//! before that filter would still pass the verb check above — it HAS
//! `backup`, it just cannot filter it — and this program would have no way
//! to notice short of fetching a whole machine and finding an exempt
//! application's bytes on disk after the fact. This build of `backup`
//! refuses a flag it does not recognise rather than ignoring it, so in
//! practice an old binary handed `--service` today would error loudly
//! rather than mis-fetch quietly — but depending on that is depending on
//! unspecified behaviour of a binary this program does not own, which is
//! exactly the fragility §2.4 was written against. So [`preflight`] proves
//! `--service` the same way it proves the verb itself: positively, before a
//! pass schedules a single fetch that relies on it.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// A `cargo-rivet` that has been run and can `backup`.
#[derive(Debug, Clone)]
pub struct Tool {
    pub path: PathBuf,
    /// The first line of `--version`, for the log and the report.
    pub version: String,
}

/// **How anybody gets a `cargo-rivet` this build can use** — from Rivet's
/// public repository, at the release every proof in this file was run against
/// (`SPEC-01` §2.4). Every refusal that ends in "install one" says this.
///
/// **`--force`**, because whoever reads a refusal already has a `cargo-rivet`
/// — the one just refused — and `cargo install` will not replace a binary
/// another source installed without it.
pub const INSTALL: &str =
    "cargo install --git https://github.com/caladil/Rivet --tag v0.1.137 --locked --force cargo-rivet";

/// A proof whose `cargo-rivet` would not start, said the way every refusal
/// about the tool is — with how to install one.
fn would_not_start(why: String) -> String {
    format!(
        "{why}\n\x20 why  tool.path must name a `cargo-rivet` executable; this one would not run \
         at all.\n\x20 fix  `{INSTALL}` installs one."
    )
}

/// Run `<path> rivet <args…>` and hand back what it said.
///
/// **stdout and stderr are joined.** `cargo-rivet` writes its refusals to
/// stderr and its reports to stdout, and a caller that read one would miss
/// half of what it needs to say.
///
/// **`CREATE_NO_WINDOW`, always.** `cargo-rivet.exe` is a console program, and
/// the tray has let go of its own console (`tray::detach_own_console`), so
/// without this flag Windows gives every call a brand-new console window —
/// a black window flashing on the desktop on every pass.
fn run(path: &Path, args: &[&str]) -> Result<(bool, String), String> {
    let mut command = Command::new(path);
    command.arg("rivet").args(args);
    no_window(&mut command);
    let out = command
        .output()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    let err = String::from_utf8_lossy(&out.stderr);
    if !err.trim().is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&err);
    }
    Ok((out.status.success(), text))
}

/// **`CREATE_NO_WINDOW`, on every child this program starts** — see [`run`].
/// One place, so a new call path cannot forget it.
fn no_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// `<path> rivet <args…>`, not yet started.
#[must_use]
pub fn command(path: &Path, args: &[String]) -> Command {
    let mut command = Command::new(path);
    command.arg("rivet").args(args);
    command
}

/// What a bounded child did.
#[derive(Debug, Clone)]
pub struct Finished {
    /// Exited on its own with status 0. False when it was killed.
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    /// **It was still running at the limit and was killed** — `SPEC-02` §3.3.
    pub timed_out: bool,
    pub elapsed: Duration,
}

/// **Run a child with a hard limit, and KILL it when the limit passes** —
/// `SPEC-02` §3.3.
///
/// This is the case the monitor exists for: a wedged board may accept a
/// connection and never answer, and a capture with no limit would stop that
/// machine's worker at the moment it matters most. `std::process::Command`
/// has no timeout, so the child is polled and `TerminateProcess`ed.
///
/// stdout and stderr are drained on their own threads while it runs, so a
/// child that writes more than a pipe holds cannot deadlock against this. And
/// after a kill those threads are waited on for **two seconds and no longer**:
/// a grandchild that inherited a pipe would otherwise hold this call open for
/// as long as it lived, which is the hang the limit exists to prevent.
///
/// `CREATE_NO_WINDOW` is applied here, whatever `command` is.
///
/// # Errors
///
/// Only a child that would not start at all — the tool missing, most often.
pub fn run_bounded(mut command: Command, limit: Duration) -> Result<Finished, String> {
    use std::io::Read as _;
    use std::process::Stdio;

    no_window(&mut command);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let program = command.get_program().to_string_lossy().into_owned();
    let started = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|e| format!("{program} would not start: {e}"))?;

    let drain = |pipe: Option<Box<dyn std::io::Read + Send>>| {
        let (sender, receiver) = std::sync::mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            let _ = sender.send(bytes);
        });
        receiver
    };
    let stdout = drain(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
    );
    let stderr = drain(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
    );

    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{program} could not be waited on: {e}"));
            }
        }
        if started.elapsed() >= limit {
            timed_out = true;
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let elapsed = started.elapsed();

    let grace = Duration::from_secs(2);
    let text = |receiver: std::sync::mpsc::Receiver<Vec<u8>>| {
        String::from_utf8_lossy(&receiver.recv_timeout(grace).unwrap_or_default()).into_owned()
    };
    Ok(Finished {
        success: status.is_some_and(|s| s.success()),
        stdout: text(stdout),
        stderr: text(stderr),
        timed_out,
        elapsed,
    })
}

/// `run --host … --identity … --pin … -- get <subject>` — the arguments of a
/// spot reading (`SPEC-02` §4.1), and of [`health`]'s `get status`.
#[must_use]
pub fn get_args(target: &Target, identity: &Path, subject: &str) -> Vec<String> {
    vec![
        "run".to_string(),
        "--host".to_string(),
        format!("{}:{}", target.address, target.control_port),
        "--identity".to_string(),
        identity.display().to_string(),
        "--pin".to_string(),
        target.pin.clone(),
        "--".to_string(),
        "get".to_string(),
        subject.to_string(),
    ]
}

/// **The words after `--` for one log capture** — `SPEC-02` §4.2.
///
/// No cursor is the first capture ever for a machine, and asks for the whole
/// current boot: `get log latest all`. A cursor asks for everything after it,
/// across every boot the journal holds: `get log all all <cursor>`. The `all`
/// boot selector is what makes a reboot between two captures safe — records
/// from a later boot come after a cursor in an earlier one.
#[must_use]
pub fn log_words(cursor: Option<&str>) -> Vec<String> {
    let mut words = vec!["get".to_string(), "log".to_string()];
    match cursor {
        None => words.extend(["latest".to_string(), "all".to_string()]),
        Some(cursor) => words.extend(["all".to_string(), "all".to_string(), cursor.to_string()]),
    }
    words
}

/// `run --host … --identity … --pin … --full -- get log …` — one log capture.
///
/// **`--full`**, because `cargo rivet run` otherwise cuts its output at
/// 64 KiB, and a whole boot is megabytes (1.4 MB for 3,524 records on
/// `rivet-01`, 2026-09-15). A cut answer is not JSON and would be refused, so
/// without the flag every first capture would fail.
#[must_use]
pub fn log_args(target: &Target, identity: &Path, cursor: Option<&str>) -> Vec<String> {
    let mut args = get_args(target, identity, "log");
    // `get_args` ends `-- get log`; the flag goes before the `--`.
    let words_at = args.len() - 3;
    args.truncate(words_at);
    args.push("--full".to_string());
    args.push("--".to_string());
    args.extend(log_words(cursor));
    args
}

/// **Prove this binary can take a log capture as `SPEC-02` §4.2 sends it** —
/// `--full`, and a cursor as the fourth word.
///
/// A `cargo-rivet` that translates `get log` but predates the cursor would
/// pass [`prove_get`] and then refuse every capture after the first, or —
/// worse — send a cursor its machine ignores and answer the whole journal
/// every minute. So the proof is of the exact shape: `run --show --full -- get
/// log all all <cursor>` must print the `rivet_get` call carrying
/// `"subject":"log"` and a `since`.
///
/// # Errors
///
/// A binary that will not start, or an answer that is not that translation.
pub fn prove_log(path: &Path) -> Result<(), String> {
    let mut args = vec!["run".to_string(), "--show".to_string(), "--full".to_string(), "--".to_string()];
    args.extend(log_words(Some(PROOF_CURSOR)));
    let finished = run_bounded(command(path, &args), Duration::from_secs(20)).map_err(would_not_start)?;
    judge_show_log(path, &finished)
}

/// A cursor-shaped word for the proof. `--show` sends nothing, so it names no
/// real position.
const PROOF_CURSOR: &str = "proof@1";

/// [`prove_log`]'s judgement, apart from running anything.
fn judge_show_log(path: &Path, finished: &Finished) -> Result<(), String> {
    let since = format!("\"since\":\"{PROOF_CURSOR}\"");
    if finished.success
        && finished.stdout.contains("rivet_get")
        && finished.stdout.contains("\"subject\":\"log\"")
        && finished.stdout.contains(&since)
    {
        return Ok(());
    }
    let said = format!("{}{}", finished.stdout, finished.stderr);
    Err(format!(
        "{} cannot capture `log`: `run --show --full -- get log all all {PROOF_CURSOR}` did not \
         print the `rivet_get` call with \"subject\":\"log\" and {since}.\n\x20 it said  {}\n\x20 \
         why  the log is captured from a stored cursor (SPEC-02 §4.2), and a build that cannot \
         send one would fetch the whole journal every time or refuse every capture after the \
         first.\n\x20 fix  set tool.path to a cargo-rivet that can. `{INSTALL}` installs one.",
        path.display(),
        said.trim()
    ))
}

/// **Prove this binary can `run` and translates `get <subject>`**, before a
/// single capture relies on it — the positive-assertion shape `SPEC-01` §2.4
/// uses for `backup`.
///
/// `run --show` prints the tool call it would send and sends nothing: no
/// identity, no pin, no machine. So a `cargo-rivet` with no `run` verb, or one
/// whose catalogue has no such subject, is found here rather than as a
/// `failed` line per capture that nobody reads until they need the data.
///
/// # Errors
///
/// A binary that will not start, or an answer that is not the translation.
pub fn prove_get(path: &Path, subject: &str) -> Result<(), String> {
    let args = [
        "run".to_string(),
        "--show".to_string(),
        "--".to_string(),
        "get".to_string(),
        subject.to_string(),
    ];
    let finished = run_bounded(command(path, &args), Duration::from_secs(20)).map_err(would_not_start)?;
    judge_show(path, subject, &finished)
}

/// [`prove_get`]'s judgement, apart from running anything.
fn judge_show(path: &Path, subject: &str, finished: &Finished) -> Result<(), String> {
    let wanted = format!("{{\"subject\":\"{subject}\"}}");
    if finished.success
        && finished.stdout.contains("rivet_get")
        && finished.stdout.contains(&wanted)
    {
        return Ok(());
    }
    let said = format!("{}{}", finished.stdout, finished.stderr);
    Err(format!(
        "{} cannot capture `{subject}`: `run --show -- get {subject}` did not print the \
         `rivet_get` call with {wanted}.\n\x20 it said  {}\n\x20 why  every monitor capture is \
         `cargo rivet run … -- get <subject>` (SPEC-02 §4.1), and a build that cannot translate \
         it would fail every capture.\n\x20 fix  set tool.path to a cargo-rivet that can. \
         `{INSTALL}` installs one.",
        path.display(),
        said.trim()
    ))
}

/// `screenshot --host … --identity … --pin … --holder --json --out <staging>` —
/// one screenshot capture (`SPEC-02` §4.3).
///
/// **`--holder`**, because which application's frame to fetch is the
/// machine's to say and where that frame lives is Rivet's to know (`ADR-002`
/// §4): this program never composes a board path. **`--json`**, because the
/// holder, the frame's own timestamp and "nobody holds the display" are read
/// from one object rather than from prose. **Never `apply input capture`**:
/// the verb fetches the frame that is already there and presses nothing.
#[must_use]
pub fn screenshot_args(target: &Target, identity: &Path, out: &Path) -> Vec<String> {
    vec![
        "screenshot".to_string(),
        "--host".to_string(),
        format!("{}:{}", target.address, target.control_port),
        "--identity".to_string(),
        identity.display().to_string(),
        "--pin".to_string(),
        target.pin.clone(),
        "--holder".to_string(),
        "--json".to_string(),
        "--out".to_string(),
        out.display().to_string(),
    ]
}

/// **Prove this binary's `screenshot` can find the display holder's frame and
/// say so in JSON**, before a single capture relies on it — `SPEC-02` §4.3,
/// the positive-assertion way `SPEC-01` §3.5.1 proves `backup --service`.
///
/// `screenshot --help` prints the verb's usage and contacts nothing. A build
/// from before `--holder` has `screenshot` and neither flag: it would fetch the
/// LAUNCHER's frame by default (hours old while an application holds the
/// display) and print prose, so every capture would be stored as a picture of
/// the wrong thing. That is found here instead.
///
/// # Errors
///
/// A binary that will not start, or a usage that does not name both flags.
pub fn prove_screenshot(path: &Path) -> Result<(), String> {
    let args = ["screenshot".to_string(), "--help".to_string()];
    let finished = run_bounded(command(path, &args), Duration::from_secs(20)).map_err(would_not_start)?;
    judge_screenshot_usage(path, &format!("{}\n{}", finished.stdout, finished.stderr))
}

/// [`prove_screenshot`]'s judgement, apart from running anything.
///
/// **Three positive assertions**: the text is `screenshot`'s own usage, and a
/// line of it STARTS with `--holder` and another with `--json` — the options
/// list, not a sentence that merely mentions a flag, which is how Rivet's own
/// test of this help reads it.
fn judge_screenshot_usage(path: &Path, said: &str) -> Result<(), String> {
    let starts_a_line = |flag: &str| said.lines().any(|line| line.trim_start().starts_with(flag));
    let missing: Vec<&str> = ["--holder", "--json"]
        .into_iter()
        .filter(|flag| !starts_a_line(flag))
        .collect();
    if said.contains("usage: cargo rivet screenshot") && missing.is_empty() {
        return Ok(());
    }
    let what = if said.contains("usage: cargo rivet screenshot") {
        format!("its `screenshot` usage names no {} option", missing.join(" or "))
    } else {
        "`screenshot --help` did not print `screenshot`'s usage".to_string()
    };
    let first = said.trim().lines().next().unwrap_or("(nothing)");
    Err(format!(
        "{} cannot capture `screenshot`: {what}.\n\x20 it said  {first}\n\x20 why  a screenshot is \
         the frame of the application HOLDING THE DISPLAY, found by the tool (`--holder`) and \
         reported as one JSON object (`--json`) naming the holder and the frame's own timestamp \
         (SPEC-02 §4.3). A build without them fetches the launcher's frame, which may be hours \
         old, and says so in prose.\n\x20 fix  set tool.path to a cargo-rivet whose `screenshot \
         --help` lists both. `{INSTALL}` installs one.",
        path.display()
    ))
}

/// Run `<path> rivet <args…>`, with an owned argument list.
fn run_owned(path: &Path, args: &[String]) -> Result<(bool, String), String> {
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    run(path, &borrowed)
}

/// **Prove this binary can `backup`, and can filter it, before anything is
/// scheduled.**
///
/// # Errors
///
/// A path that will not execute, a build with no `backup` verb, a build
/// whose `backup` usage this one does not recognise, or a build that can
/// `backup` but does not document `--service`.
pub fn preflight(path: &Path) -> Result<Tool, String> {
    let (_, version_text) = run(path, &["--version"]).map_err(would_not_start)?;
    let version = version_text
        .lines()
        .next()
        .unwrap_or("(no version line)")
        .trim()
        .to_string();

    let (_, usage) = run(path, &["backup"]).map_err(would_not_start)?;
    decide(path, &version, &usage)
}

/// The whole of [`preflight`]'s judgement, apart from actually running the
/// binary — split out so it is one thing to test against captured `backup`
/// usage text, the same way [`parse_plan`] is tested without a real
/// `cargo-rivet` to run.
///
/// **Two POSITIVE assertions, in order, and each closes a DIFFERENT trap:**
///
/// 1. `backup`'s own usage has to appear at all (`SPEC-01` §2.4) — the
///    `cargo-rivet` on a PATH is very often an older build with no `backup`
///    verb, and this is the check that has always been here for it.
/// 2. That usage has to name `--service` (`SPEC-01` §2.4, extended
///    2026-09-09) — a build with `backup` but from before `--service`
///    (`Rivet ADR-051` §8.5) would pass check 1 and then fetch a machine whole on every
///    pass, with an exempt application's bytes crossing the network and
///    being discarded only after they arrived, rather than never crossing
///    it at all.
///
/// The second check cannot be folded into the first: a binary refusing
/// `backup` outright and a binary that runs `backup` but ignores `exempt`
/// are different failures needing different fixes, and a caller told only
/// "cannot `backup`" about the second would go looking for the wrong thing.
fn decide(path: &Path, version: &str, usage: &str) -> Result<Tool, String> {
    if !usage.contains("usage: cargo rivet backup") {
        let verbs = usage
            .lines()
            .find(|l| l.contains("is not a `cargo rivet` verb"))
            .map_or_else(String::new, |l| format!("\n\x20 it said  {}", l.trim()));
        return Err(format!(
            "{} cannot `backup`.{verbs}\n\
             \x20 version  {version}\n\
             \x20 why  Rivet Monitor does not fetch or validate anything itself — it schedules \
             `cargo rivet backup` (Rivet ADR-051), and a build without that verb can do nothing for \
             it. The `cargo-rivet` on a PATH is very often an older `cargo install` and not the \
             build that has it.\n\
             \x20 fix  set tool.path to a cargo-rivet that has `backup`. `{INSTALL}` installs \
             one.",
            path.display()
        ));
    }

    if !usage.contains("--service") {
        return Err(format!(
            "{} can `backup` but cannot filter it: its usage names no `--service` flag.\n\
             \x20 version  {version}\n\
             \x20 why  Rivet Monitor schedules `--service NAME` for every application that is \
             actually due (SPEC-01 §3.5), which is what makes `exempt` mean never fetched \
             rather than never kept. A build without the flag would fetch a machine whole on \
             every pass with no way for this program to tell — the exact silent failure \
             SPEC-01 §2.4 exists to prevent, one flag downstream of where it was closed the \
             first time.\n\
             \x20 fix  set tool.path to a cargo-rivet whose `backup` usage lists `--service` \
             (Rivet ADR-051 §8.5). `{INSTALL}` installs one.",
            path.display()
        ));
    }

    Ok(Tool {
        path: path.to_path_buf(),
        version: version.to_string(),
    })
}

/// What one machine's applications offer, as `cargo rivet backup`'s plan
/// reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// The application, as the manifest names it — `tempmon-rivet`.
    pub service: String,
    /// A feed is **reported and not fetched** by the tool this build calls
    /// (`Rivet ADR-051` §8.4), so it is carried here and said out loud rather than
    /// scheduled as if it worked.
    pub feed: bool,
}

/// Where a machine is and how to prove it is the right one.
///
/// **Three things, and none of them is interchangeable** (`SPEC-01` §4): the
/// address to dial, the name TLS verifies, and the machine's own public key
/// that the control plane pins. All three come from the register.
#[derive(Debug, Clone)]
pub struct Target {
    pub name: String,
    pub address: String,
    pub control_port: u16,
    pub pin: String,
}

/// The flags every invocation shares.
fn base_args(target: &Target, identity: &Path, into: &Path) -> Vec<String> {
    vec![
        "backup".to_string(),
        "--host".to_string(),
        format!("{}:{}", target.address, target.control_port),
        "--identity".to_string(),
        identity.display().to_string(),
        "--pin".to_string(),
        target.pin.clone(),
        "--into".to_string(),
        into.display().to_string(),
    ]
}

/// **Ask a machine what its applications offer** — the tool's own plan, which
/// fetches nothing.
///
/// # Errors
///
/// The machine's refusal, a control plane that will not answer, or a plan this
/// build cannot read.
pub fn offers(
    tool: &Tool,
    target: &Target,
    identity: &Path,
    into: &Path,
) -> Result<Vec<Offer>, String> {
    let args = base_args(target, identity, into);
    let (ok, text) = run_owned(&tool.path, &args)?;
    if !ok {
        return Err(format!("{}: {}", target.name, text.trim()));
    }
    parse_plan(&text).map_err(|why| format!("{}: {why}\n{text}", target.name))
}

/// Read `cargo rivet backup`'s plan.
///
/// **The count on the first line is the authority.** The plan's empty case
/// prints an indented prose paragraph that looks exactly like an application
/// line, and a parser that trusted indentation alone would invent an
/// application called `Nothing`. So the count is read first and the parsed
/// lines are checked against it — a plan this build cannot read is an ERROR,
/// never an empty list, because an empty list is indistinguishable from a
/// machine that offers nothing.
fn parse_plan(text: &str) -> Result<Vec<Offer>, String> {
    let head = text
        .lines()
        .find(|l| l.contains("application(s) on this machine"))
        .ok_or(
            "this is not a `cargo rivet backup` plan: no line says how many applications offer \
             something. Refusing to read it rather than reporting nothing to back up.",
        )?;
    let expected: usize = head
        .split_whitespace()
        .next()
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| format!("the plan's first line does not begin with a count: {head:?}"))?;

    let mut out: Vec<Offer> = Vec::new();
    if expected == 0 {
        return Ok(out);
    }
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("  ") {
            if !rest.starts_with(' ') && !rest.is_empty() {
                let word = rest.split_whitespace().next().unwrap_or_default();
                // The plan indents an application's name by two and its detail
                // by four; a name is one word on its own line.
                if rest.trim() == word {
                    // **A machine chooses this name**, and it becomes a folder
                    // and a `--service` argument (`SPEC-01` §4.4). Refused
                    // whole, like every other plan this build cannot use.
                    crate::register::usable_name(word).map_err(|why| {
                        format!("the plan names an application this program will not use: {why}")
                    })?;
                    out.push(Offer {
                        service: word.to_string(),
                        feed: false,
                    });
                    continue;
                }
            }
            if rest.trim_start().starts_with("feed ")
                && let Some(last) = out.last_mut()
            {
                last.feed = true;
            }
        }
    }
    if out.len() != expected {
        return Err(format!(
            "the plan says {expected} application(s) and this build read {} of them, so it does \
             not understand this tool's output. Nothing is scheduled from a plan that was only \
             partly read.",
            out.len()
        ));
    }
    // **Two names Windows would give one folder** (`SPEC-01` §4.4): `Photos` and
    // `photos` share a directory on NTFS, and one's retention would prune the
    // other's backups.
    for (i, offer) in out.iter().enumerate() {
        if let Some(twin) = out[..i]
            .iter()
            .find(|earlier| earlier.service.to_lowercase() == offer.service.to_lowercase())
        {
            return Err(format!(
                "the plan names {:?} and {:?}, which Windows would give one folder, so one's \
                 retention would prune the other's backups (SPEC-01 §4.4). Refusing the plan.",
                twin.service, offer.service
            ));
        }
    }
    Ok(out)
}

/// **What one machine says about its own health** — `Rivet ADR-045` §4.
///
/// The supervisor polls each application on its own cadence and folds the
/// `critical` ones into a machine word. Rivet Monitor **reads** that and never
/// second-guesses it: whether a degraded application matters is fleet policy,
/// expressed in the machine's own document, and it is not this program's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Health {
    /// The machine's own word — `healthy`, `degraded`, `unknown` or `down`.
    pub machine: String,
    /// One entry per service.
    pub services: Vec<ServiceHealth>,
}

/// One service, as `get status` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceHealth {
    pub name: String,
    /// Its health word, **carried verbatim**. `Rivet ADR-045` §5: nothing here
    /// branches on a caption and nothing invents a word the machine did not
    /// say.
    pub health: String,
    /// The application's own sentence about itself, when it gave one.
    pub caption: Option<String>,
    /// Whether the machine's document calls this service critical — which is
    /// what decides whether it reaches the machine's own word.
    pub critical: bool,
}

/// **Ask a machine how it is** — `cargo rivet run -- get status`.
///
/// # Why the same binary and the same credential
///
/// It is the control plane `Rivet ADR-035` already defines and the identity this
/// program already holds. A second transport for health would be a second
/// trust model, and there is exactly one.
///
/// # Errors
///
/// The machine's refusal, a control plane that will not answer, or an answer
/// this build cannot read. **A machine that did not answer is an error and
/// never an empty list**, because an empty list is what a machine with no
/// services also looks like.
pub fn health(tool: &Tool, target: &Target, identity: &Path) -> Result<Health, String> {
    let args = get_args(target, identity, "status");
    let (ok, text) = run_owned(&tool.path, &args)?;
    if !ok {
        return Err(format!("{}: {}", target.name, text.trim()));
    }
    parse_status(&text).map_err(|why| format!("{}: {why}", target.name))
}

/// Read `get status`'s JSON.
///
/// **A build of Rivet older than 2026-09-09 has no health in this answer**, and
/// that is reported as `unknown` rather than as a failure: a fleet with one old
/// machine in it should still be readable.
fn parse_status(text: &str) -> Result<Health, String> {
    let start = text.find('{').ok_or(
        "this is not a `get status` answer: there is no JSON in it. Refusing to read it rather \
         than reporting a machine with no services.",
    )?;
    let value: serde_json::Value = serde_json::from_str(text[start..].trim())
        .map_err(|e| format!("`get status` did not answer JSON this build can read: {e}"))?;

    let word = |v: Option<&serde_json::Value>| {
        v.and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_string()
    };

    let listed = value
        .get("services")
        .and_then(serde_json::Value::as_array)
        .ok_or("`get status` answered without a `services` array")?;

    Ok(Health {
        machine: word(value.get("health")),
        services: listed
            .iter()
            .filter_map(|s| {
                Some(ServiceHealth {
                    name: s.get("name")?.as_str()?.to_string(),
                    health: word(s.get("health")),
                    caption: s
                        .get("caption")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string),
                    critical: s
                        .get("critical")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                })
            })
            .collect(),
    })
}

/// `--service NAME`, once per entry in `services`, in the ORDER given.
///
/// Split out from [`fetch_args`] so it is one thing to test directly.
/// **[`offers`] never calls this.** §2.3's override check needs a machine's
/// WHOLE table to tell an unmatched override from one for an application
/// this particular pass simply is not fetching yet — narrowing discovery by
/// the same filter would make a real application look renamed.
fn service_args(services: &[String]) -> Vec<String> {
    services
        .iter()
        .flat_map(|service| ["--service".to_string(), service.clone()])
        .collect()
}

/// The full argument list one fetch sends: [`base_args`], one `--service`
/// per name in `services`, and `confirm`.
///
/// **No `--name` and no `--authority`.** Since Rivet's backups moved onto the
/// control plane (`Rivet ADR-051`, `Rivet R-234`), `cargo rivet backup` reaches each
/// application through `rivet-mcpd` on the pinned connection and refuses both
/// flags; the operator certificate they fed is retired.
///
/// Kept apart from [`fetch`] so a test can see exactly what would run
/// without spawning `cargo-rivet` — a [`Target`], a couple of paths and a
/// list of names are enough to check the shape, the same way [`parse_plan`]
/// is tested against captured text rather than a live process.
fn fetch_args(
    target: &Target,
    identity: &Path,
    into: &Path,
    services: &[String],
) -> Vec<String> {
    let mut args = base_args(target, identity, into);
    args.extend(service_args(services));
    args.push("confirm".to_string());
    args
}

/// **Fetch only `services`, into `into`.**
///
/// `services` is `schedule::due_here`'s own answer for this machine —
/// everything actually due, with `exempt` and a feed already filtered out
/// before this is ever called. Passing it as `--service` (`Rivet ADR-051`
/// §8.5) is what makes `exempt` mean *never fetched*
/// rather than *never kept*: an application not named here never has its
/// bytes cross the network, rather than crossing it and being discarded
/// once they arrive (`SPEC-01` §3.5).
///
/// The staging directory the caller passes as `into` still exists and still
/// matters — placement, naming and retention stay this program's, never
/// this verb's (`Rivet ADR-051` §8.2) — filtering the fetch just means staging now
/// holds only what was actually asked for, rather than a whole machine.
///
/// # Errors
///
/// The tool's own report, verbatim, when any application failed. The tool does
/// not stop at the first failure (`Rivet ADR-051` §8.3), so the successful ones are
/// still on disk and the caller still walks the staging directory.
pub fn fetch(
    tool: &Tool,
    target: &Target,
    identity: &Path,
    into: &Path,
    services: &[String],
) -> Result<String, String> {
    let args = fetch_args(target, identity, into, services);
    let (ok, text) = run_owned(&tool.path, &args)?;
    if ok {
        Ok(text)
    } else {
        Err(text.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    const PLAN: &str = r"2 application(s) on this machine offer something to back up.

  tempmon-rivet
    fetch  https://rivet-01.local:443/tempmon/api/v1/backup
    as     sqlite
    into   D:\b\tempmon-rivet
  archive-rivet
    feed   https://rivet-01.local:443/archive/api/v1/blobs
    NOT FETCHED - this build implements the artifact half only (ADR-051)

Nothing was fetched. Add `confirm` to fetch and write.
";

    #[test]
    fn a_plan_names_its_applications_and_says_which_are_feeds() {
        let offers = parse_plan(PLAN).expect("a readable plan");
        assert_eq!(
            offers,
            vec![
                Offer {
                    service: "tempmon-rivet".into(),
                    feed: false
                },
                Offer {
                    service: "archive-rivet".into(),
                    feed: true
                },
            ]
        );
    }

    /// **The empty plan's prose paragraph is indented like an application.**
    /// A parser trusting indentation would invent one called `Nothing`.
    #[test]
    fn an_empty_plan_is_empty_and_not_a_machine_offering_nothing() {
        let text = r"0 application(s) on this machine offer something to back up.

  Nothing is missing: an application offers a backup by declaring backup-path
  and backup-format, or backup-feed, in its signed manifest.
";
        assert!(parse_plan(text).expect("readable").is_empty());
    }

    /// **A plan this build cannot read is an ERROR, never an empty list**, and
    /// an empty list is what "nothing to back up" also looks like.
    #[test]
    fn output_that_is_not_a_plan_is_refused_rather_than_read_as_nothing() {
        let refused = parse_plan("cargo-rivet: `backup` is not a `cargo rivet` verb.\n")
            .expect_err("not a plan");
        assert!(refused.contains("Refusing"), "{refused}");
    }

    /// A count that disagrees with the lines means this build no longer
    /// understands the tool, and a partly-read plan schedules nothing.
    #[test]
    fn a_count_that_disagrees_with_the_lines_is_refused() {
        let text = "3 application(s) on this machine offer something to back up.\n\n  a\n";
        let refused = parse_plan(text).expect_err("count disagrees");
        assert!(refused.contains("partly read"), "{refused}");
    }

    /// **§4.4: a machine's application name becomes a folder and an argument**,
    /// so one that could leave the folder or read as an option refuses the plan
    /// — never a `--service --into` on a command line, never a `..` under the
    /// backup folder.
    #[test]
    fn a_plan_naming_an_application_this_program_will_not_use_is_refused() {
        for bad in ["--into", "..", "C:\\x", "nul", ".rivet-monitor-staging", "TEMPMO~1"] {
            let text = PLAN.replace("  tempmon-rivet\n", &format!("  {bad}\n"));
            let refused = parse_plan(&text).expect_err(bad);
            assert!(refused.contains("will not use"), "{bad}: {refused}");
        }
    }

    /// **§4.4: two names Windows would give one folder refuse the plan** — one's
    /// retention would prune the other's backups.
    #[test]
    fn a_plan_naming_two_applications_that_differ_only_in_case_is_refused() {
        let text = PLAN.replace("  archive-rivet\n", "  TempMon-Rivet\n");
        let refused = parse_plan(&text).expect_err("case twins");
        assert!(refused.contains("one folder"), "{refused}");
        // NTFS folds more than ASCII.
        let text = PLAN
            .replace("  tempmon-rivet\n", "  café-rivet\n")
            .replace("  archive-rivet\n", "  CAFÉ-rivet\n");
        let refused = parse_plan(&text).expect_err("non-ASCII case twins");
        assert!(refused.contains("one folder"), "{refused}");
    }

    /// **The refusal a stranger meets first** — `tool.path` naming nothing that
    /// runs — says how to install one, as every refusal about the tool does
    /// (`SPEC-01` §2.4).
    #[test]
    fn a_tool_path_that_will_not_run_says_how_to_install_one() {
        let missing = std::env::temp_dir().join("rivet-monitor-no-such-tool").join("cargo-rivet.exe");
        let refusals = [
            preflight(&missing).map(|_| ()),
            prove_get(&missing, "status"),
            prove_log(&missing),
            prove_screenshot(&missing),
        ];
        for refused in refusals {
            let refused = refused.expect_err("nothing there runs");
            assert!(refused.contains("would not run at all"), "{refused}");
            assert!(refused.contains(INSTALL), "{refused}");
        }
    }

    // -----------------------------------------------------------------------
    // --service — the arguments this program builds
    // -----------------------------------------------------------------------

    fn target() -> Target {
        Target {
            name: "rivet-01.local".to_string(),
            address: "192.168.50.10".to_string(),
            control_port: 7777,
            pin: "ab".to_string(),
        }
    }

    /// **One due application produces exactly one `--service` flag naming
    /// it.** This is the literal shape `schedule::fetch_and_place` hands to
    /// `cargo-rivet` for a machine with one thing due.
    #[test]
    fn fetch_args_names_one_due_application() {
        let due = vec!["tempmon-rivet".to_string()];
        let args = fetch_args(
            &target(),
            Path::new("./id"),
            Path::new("./into"),
            &due,
        );
        assert_eq!(args.iter().filter(|a| *a == "--service").count(), 1);
        assert!(
            args.windows(2)
                .any(|w| w == ["--service".to_string(), "tempmon-rivet".to_string()]),
            "{args:?}"
        );
        assert!(args.contains(&"confirm".to_string()), "{args:?}");
    }

    /// **Several due applications produce one `--service` flag EACH**, in
    /// the order `due_here` found them — the case a machine with more than
    /// one thing due on the same pass hits every week.
    #[test]
    fn fetch_args_names_every_due_application_with_its_own_flag() {
        let due = vec!["tempmon-rivet".to_string(), "notes-rivet".to_string()];
        let args = fetch_args(
            &target(),
            Path::new("./id"),
            Path::new("./into"),
            &due,
        );
        assert_eq!(args.iter().filter(|a| *a == "--service").count(), 2);
        assert!(
            args.windows(2)
                .any(|w| w == ["--service".to_string(), "tempmon-rivet".to_string()]),
            "{args:?}"
        );
        assert!(
            args.windows(2)
                .any(|w| w == ["--service".to_string(), "notes-rivet".to_string()]),
            "{args:?}"
        );
    }

    /// **An application never in `due` never appears as an argument at
    /// all** — not merely unfetched after arriving, but never named to the
    /// tool in the first place. This is the literal proof that `exempt`
    /// closes at the argument list this program builds, not just at where
    /// the fetched bytes are later placed.
    #[test]
    fn fetch_args_never_names_a_service_that_is_not_due() {
        let due = vec!["tempmon-rivet".to_string()];
        let args = fetch_args(
            &target(),
            Path::new("./id"),
            Path::new("./into"),
            &due,
        );
        assert!(!args.contains(&"archive-rivet".to_string()), "{args:?}");
    }

    /// **No due application at all builds no `--service` flag**, which is
    /// the identity case `cargo rivet backup` itself defines (`Rivet ADR-051`
    /// §8.5) — `schedule::run` never reaches this with an empty `due`
    /// (`if due.is_empty() ... continue`), but the argument builder is still
    /// correct on its own if it ever did.
    #[test]
    fn fetch_args_with_nothing_due_names_no_service() {
        let args = fetch_args(
            &target(),
            Path::new("./id"),
            Path::new("./into"),
            &[],
        );
        assert!(!args.contains(&"--service".to_string()), "{args:?}");
    }

    /// **Neither `--name` nor `--authority` is ever passed.** A `cargo-rivet`
    /// built since Rivet's backups moved onto the control plane refuses both,
    /// so passing either fails every capture and every backup.
    #[test]
    fn backup_args_carry_neither_name_nor_authority() {
        let due = vec!["tempmon-rivet".to_string()];
        for args in [
            base_args(&target(), Path::new("./id"), Path::new("./into")),
            fetch_args(&target(), Path::new("./id"), Path::new("./into"), &due),
        ] {
            assert!(!args.contains(&"--name".to_string()), "{args:?}");
            assert!(!args.contains(&"--authority".to_string()), "{args:?}");
        }
    }

    // -----------------------------------------------------------------------
    // preflight's `--service` proof
    // -----------------------------------------------------------------------

    /// A `backup` usage carrying both the header line and `--service` — what
    /// a `cargo-rivet` that has `Rivet ADR-051` §8.5's filter prints.
    const USAGE_WITH_SERVICE: &str = "usage: cargo rivet backup --host HOST:PORT --identity DIR --pin HEX --into DIR [confirm]\n\n\
         backup options:\n\
         \x20 --host HOST:PORT   REQUIRED.\n\
         \x20 --into DIR         REQUIRED.\n\
         \x20 --service NAME     repeatable. Fetch only the named application(s).\n\
         \x20 confirm            fetch and write.\n";

    /// The same shape, but from before `--service` existed — what every
    /// `cargo-rivet` between `Rivet ADR-051`'s first landing and its §8.5 filter
    /// prints. It still has to pass check 1; only check 2 may fail it.
    const USAGE_WITHOUT_SERVICE: &str = "usage: cargo rivet backup --host HOST:PORT --identity DIR --pin HEX --into DIR [confirm]\n\n\
         backup options:\n\
         \x20 --host HOST:PORT   REQUIRED.\n\
         \x20 --into DIR         REQUIRED.\n\
         \x20 confirm            fetch and write.\n";

    /// What an old build, Rivet 0.1.66's, actually says — no `backup` usage at
    /// all.
    const USAGE_NO_BACKUP_VERB: &str =
        "cargo-rivet: `backup` is not a `cargo rivet` verb. This build has 9 verbs: …\n";

    /// **The monitor's proof is positive too**: `run --show` must print the
    /// `rivet_get` call for the subject, and a build that cannot is refused by
    /// subject. The accepted text is what the real binary printed, 2026-09-15.
    #[test]
    fn a_build_that_cannot_translate_get_subject_is_refused_by_subject() {
        let shown = Finished {
            success: true,
            stdout: "tool       rivet_get\narguments  {\"subject\":\"slot\"}\nwords      rivet get slot\n".into(),
            stderr: String::new(),
            timed_out: false,
            elapsed: Duration::from_millis(30),
        };
        assert!(judge_show(Path::new("cargo-rivet"), "slot", &shown).is_ok());

        let no_run = Finished {
            success: false,
            stdout: String::new(),
            stderr: "cargo-rivet: `run` is not a `cargo rivet` verb.\n".into(),
            timed_out: false,
            elapsed: Duration::from_millis(30),
        };
        let refused = judge_show(Path::new("cargo-rivet"), "slot", &no_run).expect_err("no run");
        assert!(refused.contains("cannot capture `slot`"), "{refused}");
        assert!(refused.contains(INSTALL), "{refused}");
        assert!(judge_show(Path::new("cargo-rivet"), "boot", &shown).is_err(), "a different subject");
    }

    /// **§4.2: the first capture ever asks for the whole current boot**, and
    /// `--full` sits before the `--` so the answer is not cut at 64 KiB.
    #[test]
    fn the_first_log_capture_asks_for_latest_all_with_full() {
        let args = log_args(&target(), Path::new("./id"), None);
        let dashes = args.iter().position(|a| a == "--").expect("a --");
        assert_eq!(&args[dashes + 1..], ["get", "log", "latest", "all"]);
        assert_eq!(args[dashes - 1], "--full", "{args:?}");
        assert_eq!(args[0], "run");
        assert!(args.windows(2).any(|w| w == ["--host", "192.168.50.10:7777"]), "{args:?}");
        assert!(args.windows(2).any(|w| w == ["--pin", "ab"]), "{args:?}");
    }

    /// **§4.2: a later capture passes the stored cursor**, verbatim, as the
    /// fourth word, with the `all` boot selector so a reboot is safe.
    #[test]
    fn a_later_log_capture_passes_the_stored_cursor() {
        let cursor = "00000000-0000-4000-8000-000000000001@28131.687";
        let args = log_args(&target(), Path::new("./id"), Some(cursor));
        let dashes = args.iter().position(|a| a == "--").expect("a --");
        assert_eq!(&args[dashes + 1..], ["get", "log", "all", "all", cursor]);
        assert_eq!(args[dashes - 1], "--full", "{args:?}");
    }

    /// **The log's proof is of the exact shape it sends.** The accepted text is
    /// what the real binary printed, 2026-09-15.
    #[test]
    fn a_build_that_cannot_send_a_log_cursor_is_refused() {
        let finished = |stdout: &str| Finished {
            success: true,
            stdout: stdout.into(),
            stderr: String::new(),
            timed_out: false,
            elapsed: Duration::from_millis(30),
        };
        let shown = finished(
            "tool       rivet_get\narguments  {\"subject\":\"log\",\"boot\":\"all\",\"limit\":\"all\",\"since\":\"proof@1\"}\nwords      rivet get log all all proof@1\n",
        );
        assert!(judge_show_log(Path::new("cargo-rivet"), &shown).is_ok());
        let no_since = finished(
            "tool       rivet_get\narguments  {\"subject\":\"log\",\"boot\":\"all\",\"limit\":\"all\"}\n",
        );
        let refused = judge_show_log(Path::new("cargo-rivet"), &no_since).expect_err("no since");
        assert!(refused.contains(INSTALL), "{refused}");
    }

    /// The options part of `cargo rivet screenshot --help` as the first build with `--holder`
    /// printed it, 2026-09-15 — the lines the proof reads, abbreviated.
    const SCREENSHOT_USAGE: &str = "usage: cargo rivet screenshot --host HOST:PORT --identity DIR --pin HEX\n\
         \x20                             [--path PATH | --holder] [--out FILE.png]\n\
         \x20                             [--qoi FILE.qoi] [--json]\n\n\
         screenshot options:\n\
         \x20 --host HOST:PORT   REQUIRED. Where rivet-mcpd is listening.\n\
         \x20 --path PATH        the capture ON THE BOARD. Default:\n\
         \x20 --holder           the frame of the APPLICATION HOLDING THE DISPLAY,\n\
         \x20 --json             print ONE JSON object on stdout, and nothing else\n\
         \x20 --out FILE.png     where the PNG goes. Default ./screen.png, and an\n";

    /// **§4.3's preflight accepts a usage naming both flags as options.**
    #[test]
    fn a_screenshot_usage_naming_holder_and_json_is_accepted() {
        assert_eq!(judge_screenshot_usage(Path::new("cargo-rivet"), SCREENSHOT_USAGE), Ok(()));
    }

    /// **…and refuses one lacking either**, naming which — including a usage
    /// that mentions the flag only inside its synopsis, not as an option.
    #[test]
    fn a_screenshot_usage_lacking_holder_or_json_is_refused_naming_it() {
        let without = |flag: &str| {
            SCREENSHOT_USAGE
                .lines()
                .filter(|line| !line.trim_start().starts_with(flag))
                .collect::<Vec<_>>()
                .join("\n")
        };
        for flag in ["--holder", "--json"] {
            let text = without(flag);
            assert!(text.contains(flag), "the synopsis still mentions {flag}, and must not count");
            let refused = judge_screenshot_usage(Path::new("cargo-rivet"), &text).expect_err(flag);
            assert!(refused.contains(&format!("names no {flag} option")), "{refused}");
            assert!(refused.contains(INSTALL), "{refused}");
        }
        let neither = without("--holder").lines().filter(|l| !l.trim_start().starts_with("--json")).collect::<Vec<_>>().join("\n");
        let refused = judge_screenshot_usage(Path::new("cargo-rivet"), &neither).expect_err("neither");
        assert!(refused.contains("--holder or --json"), "{refused}");

        let old = "cargo-rivet: `screenshot` is not a `cargo rivet` verb. This build has 9 verbs: …\n";
        let refused = judge_screenshot_usage(Path::new("cargo-rivet"), old).expect_err("no verb");
        assert!(refused.contains("did not print `screenshot`'s usage"), "{refused}");

        // Both flags as options of some OTHER verb's usage are not this proof.
        let other = SCREENSHOT_USAGE.replace("usage: cargo rivet screenshot", "usage: cargo rivet watch");
        let refused = judge_screenshot_usage(Path::new("cargo-rivet"), &other).expect_err("another verb");
        assert!(refused.contains("did not print `screenshot`'s usage"), "{refused}");
    }

    /// **§4.3's call**: `--holder`, `--json`, the staging file, and the
    /// register's host and pin — and no board path and nothing that presses.
    #[test]
    fn the_screenshot_call_asks_for_the_holder_in_json_into_staging() {
        let args = screenshot_args(&target(), Path::new("./id"), Path::new("D:/m/rivet-01/screenshot/staging.png"));
        assert_eq!(args[0], "screenshot");
        assert!(args.contains(&"--holder".to_string()) && args.contains(&"--json".to_string()), "{args:?}");
        assert!(args.windows(2).any(|w| w == ["--out", "D:/m/rivet-01/screenshot/staging.png"]), "{args:?}");
        assert!(args.windows(2).any(|w| w == ["--host", "192.168.50.10:7777"]), "{args:?}");
        assert!(args.windows(2).any(|w| w == ["--pin", "ab"]), "{args:?}");
        assert!(!args.iter().any(|a| a == "--path" || a == "apply" || a == "capture"), "{args:?}");
    }

    #[test]
    fn a_build_with_service_is_accepted() {
        let tool = decide(Path::new("cargo-rivet"), "0.1.90", USAGE_WITH_SERVICE)
            .expect("a build with --service");
        assert_eq!(tool.version, "0.1.90");
    }

    /// **The trap this task closes.** A build that CAN `backup` but does not
    /// document `--service` is refused, naming the missing flag rather than
    /// being accepted on the strength of having a `backup` verb at all.
    #[test]
    fn a_build_with_backup_but_no_service_is_refused_naming_the_flag() {
        let refused = decide(Path::new("cargo-rivet"), "0.1.80", USAGE_WITHOUT_SERVICE)
            .expect_err("backup with no --service");
        assert!(refused.contains("--service"), "{refused}");
        assert!(refused.contains("cannot filter"), "{refused}");
        assert!(refused.contains("0.1.80"), "{refused}");
        assert!(refused.contains("lists `--service`"), "{refused}");
        assert!(refused.contains(INSTALL), "{refused}");
    }

    /// The original trap this preflight was built for is still caught, and
    /// caught by the FIRST check rather than the second — a binary with no
    /// `backup` verb at all is missing more than a flag.
    #[test]
    fn a_build_with_no_backup_verb_is_refused_by_the_verb_check() {
        let refused = decide(Path::new("cargo-rivet"), "0.1.66", USAGE_NO_BACKUP_VERB)
            .expect_err("no backup verb");
        assert!(refused.contains("cannot `backup`"), "{refused}");
        assert!(refused.contains("0.1.66"), "{refused}");
        assert!(refused.contains(INSTALL), "{refused}");
        // And not the OTHER message -- the two failures need different fixes.
        assert!(!refused.contains("cannot filter"), "{refused}");
    }

    /// **A refusal's fix is one anybody can run** — Rivet's public
    /// repository, pinned to a release — never a build from a checkout only
    /// Rivet's own maintainer has.
    #[test]
    fn the_install_route_is_rivets_public_repository_at_a_release() {
        assert!(INSTALL.contains("--git https://github.com/caladil/Rivet "), "{INSTALL}");
        assert!(INSTALL.contains("--tag v"), "{INSTALL}");
        assert!(INSTALL.contains("--locked"), "{INSTALL}");
        assert!(!INSTALL.contains("--path"), "{INSTALL}");
    }
}
