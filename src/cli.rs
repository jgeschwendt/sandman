//! The dispatcher: `sandman <verb> [args]`.
//!
//! Hand-rolled parsing, std only. Three exit codes and nothing else — 0 for a
//! verb that ran, 1 for a failure (one line on stderr), 2 for anything the
//! operator has to fix in the command line itself, unbuilt verbs included.
//!
//! This is the only place that resolves the real roots (`crate::paths`); every
//! verb below it takes them as arguments.

use std::env;
use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::str::FromStr;
use std::time::Instant;

use crate::error::Error;
use crate::hook;
use crate::journal;
use crate::memory::MemoryType;
use crate::paths;
use crate::slug::truncate_chars;
use crate::time::Timestamp;
use crate::verbs::{dream, forget, recall, reflect, remember, take};
use crate::version::VERSION;

/// A failure that ran a verb badly.
const EXIT_ERROR: u8 = 1;
/// A failure in the command line itself.
const EXIT_USAGE: u8 = 2;

/// Set by the dream pass on the minds it spawns. A memory-blind run recalls
/// nothing: extraction and judging must never echo existing memories.
const PIPELINE_ENV: &str = "CLAUDE_MEMORY_PIPELINE";
/// Claude Code names the running session here.
const SESSION_ENV: &str = "CLAUDE_SESSION_ID";
/// Set by a caller that drives `claude -p --resume` turns: each turn ends the
/// session it resumes, and the take would move the transcript out from under
/// the next one.
const NO_TAKE_ENV: &str = "SANDMAN_NO_TAKE";

/// The `SessionEnd` reason an operator is behind: `/clear` wipes the
/// conversation, so the transcript it ends can never be appended to again.
const CLEAR_REASON: &str = "clear";

/// How much of a bank's memory list one journal line keeps, before the rest
/// becomes a count. A bank of a hundred files must not be a hundred-file line.
const BANK_CHARS: usize = 2_000;
/// What a journal line shows for a field with nothing to report — distinct
/// from [`NONE`], which is a field the payload could have carried and did not.
const EMPTY: &str = "-";
/// What a journal line shows for a field the payload did not carry.
const NONE: &str = "none";
/// How much of an unparseable payload the journal keeps. Enough to see what
/// arrived; not so much that one bad hook fills the day's log.
const PAYLOAD_CHARS: usize = 2_000;

/// The whole surface, in one screen.
const USAGE: &str = "\
sandman — memory engine for Claude sessions

usage: sandman <verb> [args]

  remember \"<body>\" [--type user|feedback|project|reference] [--name N]
                    [--description D] [--bank KEY] [--cwd PATH]
      Commit one memory now. Defaults: type=feedback, name=the body's first
      eight words, description=its first line, bank=the cwd's bank key.

  take <session-id> [--force]
  take --hook            (SessionEnd payload on stdin; implies --force)
      Archive the session by move, then drop its .recent pointer. --hook reads
      a SessionEnd payload on stdin and exits quietly when it names no session,
      names one whose files forget already destroyed, or carries
      reason=resume — which Claude Code fires on the session it is adopting,
      a beginning wearing an ending's name. It also exits quietly when a live
      background job under ~/.claude/jobs would resume the session: the daemon
      retires idle workers, and a worker's exit is not the conversation's end.
      A job that has moved on speaks only for the session it would resume, and
      reason=clear reaches past that guard — an operator's wipe ends the
      transcript for good.
      A live-job decline is a debt, not a dead end: the owed take is written to
      <root>/pending-takes/<sid>.json, and every later take — and every reflect,
      for the stretches with no session endings — retries it once the job is
      gone, never forced.
      Set $SANDMAN_NO_TAKE to decline quietly — for machine-driven resume
      turns that must not count as endings; a session named by hand is taken
      regardless.
      A transcript touched in the last 120 s is refused as live — a heuristic,
      since the last line is written at the end of a turn, not of a session;
      --force takes it anyway.

  recall [--cwd PATH]
  recall --hook
      Compose what past sessions know: the cwd's bank, its ancestors' banks,
      the last three days of pointers, the log tail and the tool index, inside
      one budget. --hook reads a SessionStart payload on stdin and answers with
      the hook envelope. Silent when there is nothing to recall, and when
      $CLAUDE_MEMORY_PIPELINE=1.

  forget <session-id>
      The privacy ending: destroy every copy — transcript, subagent directory,
      archives, pointer. Never touches a bank.

  dream [--now]
      Route the queue: for each undreamed pointer, three minds (sonnet, opus,
      fable) read the archived conversation in parallel and propose memories;
      a memory commits on 2-of-3 agreement. Take spawns this at queue depth
      10; --now is documentation, both forms behave the same. Models move with
      $SANDMAN_MIND_SONNET / _OPUS / _FABLE, the binary with
      $SANDMAN_CLAUDE_BIN. Each mind's own transcript is kept at
      <root>/.dream/<claude project>/<session-id>.jsonl.

  reflect
      The 24 h pass: the day page and log index, the pointer sweep (dreamt and
      older than 72 h), and one gated opus upkeep call per grown bank
      ($SANDMAN_MIND_UPKEEP).

  version
      The crate version and the commit it was built from — the same stamp
      every journal line carries as v=, so a line can be read against the
      build that wrote it rather than whatever is checked out now.

Every verb journals its decisions — one line each, stamped and carrying the
writing pid and build — to <root>/log/<verb>-<date>.log, so a hook that
declined can still be explained afterwards. forget alone is silent,
deliberately: a line naming the session it destroyed would be the trace it
promises not to leave.

Data root: $SANDMAN_ROOT, else ~/.sandman. Transcripts: ~/.claude/projects/.
Logs: <root>/log/<verb>-<date>.log.";

/// Run the process. Everything the binary does is here.
#[must_use]
pub fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Error(error)) => {
            eprintln!("sandman: {error}");
            ExitCode::from(EXIT_ERROR)
        }
        Err(Failure::Usage(message)) => {
            eprintln!("sandman: {message}\n\n{USAGE}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

/// Why the process is not exiting 0.
enum Failure {
    /// The verb ran and failed.
    Error(Error),
    /// The command line itself is wrong.
    Usage(String),
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self::Error(error)
    }
}

/// Dispatch one command line.
fn run(args: &[String]) -> Result<(), Failure> {
    let Some(verb) = args.first() else {
        return Err(Failure::Usage("no verb".to_owned()));
    };
    let rest = &args[1..];
    match verb.as_str() {
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        "dream" => dream_verb(rest),
        "forget" => forget_verb(rest),
        "recall" => recall_verb(rest),
        "reflect" => reflect_verb(rest),
        "remember" => remember_verb(rest),
        "take" => take_verb(rest),
        "version" => version_verb(rest),
        other => Err(Failure::Usage(format!("unknown verb `{other}`"))),
    }
}

/// `remember "<body>" [flags]`.
fn remember_verb(args: &[String]) -> Result<(), Failure> {
    let mut request = remember::Remember::default();
    let mut body: Option<String> = None;
    let mut cursor = Cursor::new(args);
    while let Some(arg) = cursor.next() {
        match arg {
            "-h" | "--help" => return help(),
            "--bank" => request.bank = Some(cursor.value("--bank")?),
            "--cwd" => request.cwd = Some(PathBuf::from(cursor.value("--cwd")?)),
            "--description" => request.description = Some(cursor.value("--description")?),
            "--name" => request.name = Some(cursor.value("--name")?),
            "--type" => {
                let value = cursor.value("--type")?;
                request.kind = Some(
                    MemoryType::from_str(&value)
                        .map_err(|_| Failure::Usage(format!("unknown --type `{value}`")))?,
                );
            }
            option if option.starts_with("--") => return Err(unknown_option(option)),
            positional => {
                if body.is_some() {
                    return Err(Failure::Usage("remember takes one body".to_owned()));
                }
                body = Some(positional.to_owned());
            }
        }
    }
    let Some(body) = body else {
        return Err(Failure::Usage("remember needs a body".to_owned()));
    };
    request.body = body;
    request.session_id = env::var(SESSION_ENV).ok();

    let data_root = paths::data_root()?;
    let outcome = remember::remember(&data_root, request)?;
    // Read the bank and the type back off what the commit path wrote, rather
    // than restating its defaults here. Never the body: a memory belongs in
    // its bank, not doubled into the day's log.
    let bank = outcome
        .path
        .parent()
        .and_then(Path::file_name)
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or(NONE);
    let kind = outcome.filename.split('_').next().unwrap_or(NONE);
    note(
        Some(&data_root),
        "remember",
        &format!("wrote {} bank={bank} type={kind}", outcome.path.display()),
    );
    println!("{}", outcome.path.display());
    Ok(())
}

/// `take <session-id> [--force] | --hook` (hook mode implies force).
fn take_verb(args: &[String]) -> Result<(), Failure> {
    let mut force = false;
    let mut from_hook = false;
    let mut session_id: Option<String> = None;
    let mut cursor = Cursor::new(args);
    while let Some(arg) = cursor.next() {
        match arg {
            "-h" | "--help" => return help(),
            "--force" => force = true,
            "--hook" => from_hook = true,
            option if option.starts_with("--") => return Err(unknown_option(option)),
            positional => {
                if session_id.is_some() {
                    return Err(Failure::Usage("take takes one session id".to_owned()));
                }
                session_id = Some(positional.to_owned());
            }
        }
    }

    let journal = paths::data_root().ok();
    let result = take_run(journal.as_deref(), from_hook, force, session_id);
    // A hook has no terminal to fail into: without this line the only trace of
    // a broken take is the exit code Claude Code discards.
    if from_hook && let Err(Failure::Error(error)) = &result {
        note(journal.as_deref(), "take", &format!("error: {error}"));
    }
    result
}

/// The whole of `take` once the command line is understood.
fn take_run(
    journal: Option<&Path>,
    from_hook: bool,
    mut force: bool,
    session_id: Option<String>,
) -> Result<(), Failure> {
    let session_id = if from_hook {
        if session_id.is_some() {
            return Err(Failure::Usage(
                "take --hook reads the session from stdin".to_owned(),
            ));
        }
        // A resume turn ends the session it borrowed. Taking it would move the
        // transcript out of the live set mid-conversation, leaving the next
        // turn nothing to resume — so the caller driving those turns declares
        // that its endings are not endings, and the hook declines.
        if env::var_os(NO_TAKE_ENV).is_some_and(|value| !value.is_empty()) {
            note(journal, "take", "declined no-take-env");
            return Ok(());
        }
        // A dream mind's own ending is not a session to keep: archiving it
        // feeds the very queue that spawned it, and at depth the next take
        // spawns another dream — the recursion that flooded the machine with
        // claude processes on 2026-08-19. Recall already goes silent under
        // this variable; take must too.
        if env::var(PIPELINE_ENV).as_deref() == Ok("1") {
            // One line per dream turn. They are rare, and the alternative is a
            // silence indistinguishable from the hook never having run. The
            // payload is read here only for the id it names: a decline that
            // cannot be tied to a session is a decline no later reclaim can
            // account for. A payload that will not read still declines
            // quietly — under this variable there is nothing to take either
            // way, and a dream turn must never fail its own hook.
            let named = stdin()
                .ok()
                .and_then(|payload| hook::session_end(&payload).ok())
                .and_then(|ending| ending.session_id);
            note(
                journal,
                "take",
                &named.map_or_else(
                    || "declined pipeline".to_owned(),
                    |session_id| format!("declined pipeline session={session_id}"),
                ),
            );
            return Ok(());
        }
        // SessionEnd is the proof the session is over — the hook fires the
        // instant the transcript's last line lands, which is exactly what the
        // by-hand live-window heuristic refuses. Hook mode implies force.
        force = true;
        let payload = stdin()?;
        let ending = match hook::session_end(&payload) {
            Ok(ending) => ending,
            Err(error) => {
                // The payload is the evidence, and a hook that cannot be
                // parsed is exactly the case nobody can reconstruct later.
                note(
                    journal,
                    "take",
                    &format!(
                        "hook payload unparseable: {error} payload={}",
                        truncate_chars(payload.trim(), PAYLOAD_CHARS)
                    ),
                );
                return Err(Failure::Error(error));
            }
        };
        note(
            journal,
            "take",
            &format!(
                "hook session={} reason={} cwd={}",
                ending.session_id.as_deref().unwrap_or(NONE),
                ending.reason.as_deref().unwrap_or(NONE),
                ending
                    .cwd
                    .as_deref()
                    .map_or_else(|| NONE.to_owned(), |cwd| cwd.display().to_string())
            ),
        );
        // …with one exception, and it is the whole reason the payload's
        // `reason` is read at all: Claude Code fires `SessionEnd` with
        // `reason: "resume"` on the session it is *adopting*, at the moment of
        // adoption. Forcing there moves the transcript out from under a
        // conversation that is about to append to it — Claude Code then
        // recreates the file, the next ending takes that live fragment too,
        // and the pointer ends up naming a stub while the real conversation
        // sits orphaned in the archive (observed across twelve sessions on
        // 2026-08-25). A beginning is not an ending; decline it.
        // stele:landmark resume-is-not-an-ending
        if ending.is_resume() {
            note(
                journal,
                "take",
                &format!(
                    "declined resume session={}",
                    ending.session_id.as_deref().unwrap_or(NONE)
                ),
            );
            return Ok(());
        }
        // An operator wiping the conversation is the one ending no worker can
        // be behind: `/clear` closes the transcript for good, and the job it
        // may have run under carries on under a new session id. Read before
        // the id is taken out of the payload, because the job guard below is
        // the only thing it turns off.
        let cleared = ending.reason.as_deref() == Some(CLEAR_REASON);
        // A payload with no session is a session with nothing to take.
        let Some(session_id) = ending.session_id else {
            note(journal, "take", "declined no-session");
            return drained(journal);
        };
        // Nor is a retirement. The daemon retires a background worker that has
        // sat idle and done, and the retired process's exit fires SessionEnd
        // with an ordinary reason — but a job directory still naming the
        // session says the conversation is resumable and will be resumed. The
        // guard is the hook's alone: a session named by hand is taken on the
        // operator's word, job or no job — and a `/clear` reaches past it for
        // the same reason. The operator wiped the conversation: the transcript
        // is definitively over, no retiring worker can have fired that ending,
        // and the job — if the clear happened inside one — has already moved
        // to a new session id (2026-08-28).
        if !cleared && let Some(job) = take::live_job(&paths::claude_root()?, &session_id) {
            note(
                journal,
                "take",
                &format!("declined live-job session={session_id} job={job}"),
            );
            // The decline is right and it is not the end of the matter: the
            // job outlives every worker, and when the operator deletes it
            // nothing fires a second `SessionEnd`. Left at the decline, a
            // transcript sat in the live set for 22 h behind a job that no
            // longer existed (2026-08-30), so the owed take is written down
            // and every later take — and reflect — retries it.
            remember_pending(journal, &session_id, &job);
            return drained(journal);
        }
        session_id
    } else {
        let session_id =
            session_id.ok_or_else(|| Failure::Usage("take needs a session id".to_owned()))?;
        // The job guard above is the hook's alone, and stays that way: a
        // session named by hand is taken on the operator's word. But the take
        // that pulled a live conversation out from under its own job was a
        // by-hand one, and the only thing that made it unreconstructable was
        // that nothing recorded the guard would have fired. So it is recorded,
        // before the take rather than after — a take that panics or is killed
        // mid-move still leaves the marker that explains what it was doing.
        let guard = paths::claude_root()
            .ok()
            .and_then(|claude_root| take::live_job(&claude_root, &session_id));
        note(
            journal,
            "take",
            &format!(
                "by-hand session={session_id} guard={}",
                guard.as_deref().unwrap_or(EMPTY)
            ),
        );
        session_id
    };

    let data_root = paths::data_root()?;
    let outcome = match take::take(&data_root, &paths::claude_root()?, &session_id, force) {
        Ok(outcome) => outcome,
        // `forget` destroys every copy before the session ends, so the hook
        // that follows it finds nothing — the designed sequence, not a fault.
        // Asked for by hand, a missing transcript is still an error.
        Err(Error::NotFound { .. }) if from_hook => {
            note(
                journal,
                "take",
                &format!("declined forgotten session={session_id}"),
            );
            return drained(journal);
        }
        Err(error) => return Err(Failure::Error(error)),
    };
    let directory = outcome
        .archived_directory
        .as_ref()
        .map_or_else(String::new, |directory| {
            format!(" directory={}", directory.display())
        });
    note(
        journal,
        "take",
        &format!(
            "took session={session_id} from-hook={from_hook} forced={force} archived={} bytes={} queue={}{directory}",
            outcome.archived.display(),
            outcome.archived_bytes,
            outcome.queue_depth
        ),
    );

    println!("{}", outcome.archived.display());
    if outcome.dream_due() {
        // A held lock means a run is already draining the queue — starting
        // another would only fork a process to watch it yield.
        if dream::lock_held(&data_root) {
            note(
                journal,
                "take",
                &format!("dream held queue={}", outcome.queue_depth),
            );
            eprintln!(
                "queue at {} — a dream is already running",
                outcome.queue_depth
            );
            return drained(journal);
        }
        match spawn_dream(&data_root) {
            Ok(log) => {
                note(
                    journal,
                    "take",
                    &format!(
                        "dream spawned queue={} log={}",
                        outcome.queue_depth,
                        log.display()
                    ),
                );
                eprintln!(
                    "queue at {} — dreaming in the background, logging to {}",
                    outcome.queue_depth,
                    log.display()
                );
            }
            Err(error) => {
                note(
                    journal,
                    "take",
                    &format!("dream spawn-failed queue={}: {error}", outcome.queue_depth),
                );
                eprintln!(
                    "queue at {} — dream could not be spawned: {error}",
                    outcome.queue_depth
                );
            }
        }
    }
    drained(journal)
}

/// Write down a take a live job deferred, so a later one can settle it.
///
/// Best-effort, like the journal itself: a decline's contract is to exit
/// quietly, and a ledger that could fail the hook would be a new way for a
/// session edge to break. Recorded or not, the attempt leaves a line.
fn remember_pending(journal: Option<&Path>, session_id: &str, job: &str) {
    let Some(data_root) = journal else {
        return;
    };
    match Timestamp::now().and_then(|now| take::remember_pending(data_root, session_id, job, now)) {
        Ok(_) => note(
            journal,
            "take",
            &format!("pending session={session_id} job={job}"),
        ),
        Err(error) => note(
            journal,
            "take",
            &format!("pending unrecorded session={session_id}: {error}"),
        ),
    }
}

/// Settle the takes a live job deferred, then answer for the run.
///
/// Every ending that reaches the end of `take_run` passes through here — the
/// take that happened, and the declines that are still endings. The three
/// that do not are the two quiet ones (`$SANDMAN_NO_TAKE`, the pipeline: a
/// resume turn and a dream mind must not archive bystanders) and `resume`,
/// which is a beginning and should do nothing at all.
///
/// A drain never fails the verb: a named take that succeeded stays a success
/// however the ledger went, so the roots are resolved leniently and the drain
/// itself hands back outcomes rather than a result.
fn drained(journal: Option<&Path>) -> Result<(), Failure> {
    if let Some(data_root) = journal
        && let Ok(claude_root) = paths::claude_root()
    {
        take::drain_pending(data_root, &claude_root);
    }
    Ok(())
}

/// Journal one line, when a data root could be resolved to write it under.
///
/// The journal must never be why a verb failed — least of all a decline, whose
/// whole contract is to exit quietly — so a root that would not resolve simply
/// means no line, never an error.
fn note(data_root: Option<&Path>, verb: &str, line: &str) {
    if let Some(data_root) = data_root {
        journal::note(data_root, verb, line);
    }
}

/// Start `sandman dream` and walk away.
///
/// Take is called from a `SessionEnd` hook, which Claude Code will not wait
/// on, so the dream must outlive it: the child is never waited for, and both
/// its streams go to the day's dream log rather than to a terminal that is
/// already gone. Failing to start one is a note on stderr, never a failed
/// take — the queue is still there for the next run.
fn spawn_dream(data_root: &Path) -> Result<PathBuf, Error> {
    let log = paths::run_log(data_root, "dream", Timestamp::now()?);
    if let Some(dir) = log.parent() {
        fs::create_dir_all(dir).map_err(|source| Error::io(dir, source))?;
    }
    let out = OpenOptions::new()
        .append(true)
        .create(true)
        .open(&log)
        .map_err(|source| Error::io(&log, source))?;
    let err = out.try_clone().map_err(|source| Error::io(&log, source))?;
    let exe = env::current_exe().map_err(|source| Error::io("<current exe>", source))?;
    Command::new(exe)
        .arg("dream")
        .stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn()
        .map_err(|source| Error::io("<dream>", source))?;
    Ok(log)
}

/// `dream [--now]`.
fn dream_verb(args: &[String]) -> Result<(), Failure> {
    let mut cursor = Cursor::new(args);
    while let Some(arg) = cursor.next() {
        match arg {
            "-h" | "--help" => return help(),
            // The flag is documentation: dream always routes what is queued.
            "--now" => {}
            option if option.starts_with("--") => return Err(unknown_option(option)),
            positional => {
                return Err(Failure::Usage(format!(
                    "dream takes no arguments, got `{positional}`"
                )));
            }
        }
    }
    let data_root = paths::data_root()?;
    let options = dream::Options::from_env(&data_root, &paths::claude_root()?);
    let outcome = dream::dream(&data_root, &paths::home()?, &options)?;
    for path in &outcome.committed {
        println!("{}", path.display());
    }
    eprintln!(
        "dreamt {} pointer(s), {} left for want of a quorum, {} memories committed, \
{} already held",
        outcome.dreamed,
        outcome.skipped,
        outcome.committed.len(),
        outcome.deduped
    );
    Ok(())
}

/// `reflect`.
fn reflect_verb(args: &[String]) -> Result<(), Failure> {
    if let Some(arg) = args.first() {
        return match arg.as_str() {
            "-h" | "--help" => help(),
            option if option.starts_with("--") => Err(unknown_option(option)),
            positional => Err(Failure::Usage(format!(
                "reflect takes no arguments, got `{positional}`"
            ))),
        };
    }
    // The pass already writes one line per bank. What it had no line for is
    // the run: a nightly tick that died halfway through left a log ending in
    // an ordinary bank line, indistinguishable from one that finished.
    let journal = paths::data_root().ok();
    let started = Instant::now();
    let outcome = match reflect::reflect(
        &paths::data_root()?,
        &paths::claude_root()?,
        Timestamp::now()?,
        &reflect::Options::from_env(),
    ) {
        Ok(outcome) => outcome,
        Err(error) => {
            note(
                journal.as_deref(),
                "reflect",
                &format!(
                    "reflect error ms={}: {error}",
                    started.elapsed().as_millis()
                ),
            );
            return Err(Failure::Error(error));
        }
    };
    note(
        journal.as_deref(),
        "reflect",
        &format!(
            "reflect done banks={} due={} swept={} ms={}",
            outcome.banks.len(),
            outcome.due,
            outcome.swept,
            started.elapsed().as_millis()
        ),
    );
    println!("{}", outcome.day_page.display());
    println!("{}", outcome.index.display());
    eprintln!(
        "swept {} pointer(s); {} bank(s) considered",
        outcome.swept,
        outcome.banks.len()
    );
    Ok(())
}

/// `recall [--cwd PATH] | --hook`.
fn recall_verb(args: &[String]) -> Result<(), Failure> {
    let mut from_hook = false;
    let mut cwd: Option<PathBuf> = None;
    let mut cursor = Cursor::new(args);
    while let Some(arg) = cursor.next() {
        match arg {
            "-h" | "--help" => return help(),
            "--cwd" => cwd = Some(PathBuf::from(cursor.value("--cwd")?)),
            "--hook" => from_hook = true,
            option if option.starts_with("--") => return Err(unknown_option(option)),
            positional => {
                return Err(Failure::Usage(format!(
                    "recall takes no arguments, got `{positional}`"
                )));
            }
        }
    }
    let journal = paths::data_root().ok();
    let result = recall_run(journal.as_deref(), from_hook, cwd);
    if from_hook && let Err(Failure::Error(error)) = &result {
        note(journal.as_deref(), "recall", &format!("error: {error}"));
    }
    result
}

/// The whole of `recall` once the command line is understood.
fn recall_run(
    journal: Option<&Path>,
    from_hook: bool,
    mut cwd: Option<PathBuf>,
) -> Result<(), Failure> {
    if env::var(PIPELINE_ENV).as_deref() == Ok("1") {
        note(journal, "recall", "declined pipeline");
        return Ok(());
    }
    // The session the payload names is the only handle that ties this priming
    // to the conversation it went into — and to the `take` line the same id
    // writes when that conversation ends. A recall asked for by hand primes
    // nothing and names nobody, so it carries none.
    let mut session: Option<String> = None;
    if from_hook && cwd.is_none() {
        let payload = stdin()?;
        cwd = match hook::session_start(&payload) {
            Ok(start) => {
                session = start.session_id;
                start.cwd
            }
            Err(error) => {
                note(
                    journal,
                    "recall",
                    &format!(
                        "hook payload unparseable: {error} payload={}",
                        truncate_chars(payload.trim(), PAYLOAD_CHARS)
                    ),
                );
                return Err(Failure::Error(error));
            }
        };
    }
    let session = session.as_deref().unwrap_or(NONE);
    if from_hook {
        note(
            journal,
            "recall",
            &format!(
                "hook session={session} cwd={}",
                cwd.as_deref()
                    .map_or_else(|| NONE.to_owned(), |cwd| cwd.display().to_string())
            ),
        );
    }
    let cwd = match cwd {
        Some(cwd) => cwd,
        None => env::current_dir().map_err(|source| Error::io(".", source))?,
    };

    // Recall runs in front of every session start, so what it costs is a cost
    // the operator pays per session and can otherwise only guess at.
    let started = Instant::now();
    let composed = recall::compose(
        &paths::data_root()?,
        &paths::home()?,
        &cwd,
        Timestamp::now()?,
    );
    let elapsed = started.elapsed().as_millis();
    if composed.text.is_empty() {
        note(journal, "recall", "silent nothing-to-recall");
        return Ok(());
    }
    // The shape of the priming, never its content: a log that quoted the
    // memories back would be a second copy of the banks. `chars=` against
    // `budget=` is the one comparison worth making, so both are in the unit
    // the budget is actually spent in — `trimmed=`, `banks_degraded=` and
    // `cut_lines=` say what it cost to get there, which a size alone cannot.
    let trimmed = if composed.trimmed.sections.is_empty() {
        EMPTY.to_owned()
    } else {
        composed.trimmed.sections.join(",")
    };
    note(
        journal,
        "recall",
        &format!(
            "recalled cwd={} session={session} banks={} memories={} pointers={} \
chars={} budget={} trimmed={trimmed} banks_degraded={} cut_lines={} ms={elapsed}",
            cwd.display(),
            composed.banks.len(),
            composed.memories,
            composed.pointers,
            composed.text.chars().count(),
            recall::BUDGET_CHARS,
            composed.trimmed.banks_degraded,
            composed.trimmed.ceiling_lines,
        ),
    );
    // One line per bank, naming the files that went in. Identities, not
    // bodies: "was that rule in front of the session that broke this?" is a
    // question the shape line cannot answer and this one can.
    for bank in &composed.banks {
        note(
            journal,
            "recall",
            &format!(
                "recalled-bank session={session} bank={} memories={} rendering={}",
                bank.key,
                bank_memories(&bank.memories),
                if bank.degraded { "index" } else { "full" }
            ),
        );
    }
    if from_hook {
        println!("{}", hook::session_start_reply(&composed.text));
    } else {
        println!("{}", composed.text);
    }
    Ok(())
}

/// A bank's memory filenames, cut to one line's worth.
///
/// Whole or not at all would make a large bank's line unreadable and a small
/// bank's line perfect; a count for the tail keeps both honest, and the cut is
/// visible rather than silent.
fn bank_memories(files: &[String]) -> String {
    let mut listed = String::new();
    for (index, file) in files.iter().enumerate() {
        let separator = if index == 0 { "" } else { "," };
        if listed.chars().count() + separator.len() + file.chars().count() > BANK_CHARS {
            let _ = write!(listed, "…+{}", files.len() - index);
            break;
        }
        listed.push_str(separator);
        listed.push_str(file);
    }
    listed
}

/// `version`.
fn version_verb(args: &[String]) -> Result<(), Failure> {
    if let Some(arg) = args.first() {
        return match arg.as_str() {
            "-h" | "--help" => help(),
            option if option.starts_with("--") => Err(unknown_option(option)),
            positional => Err(Failure::Usage(format!(
                "version takes no arguments, got `{positional}`"
            ))),
        };
    }
    println!("{VERSION}");
    Ok(())
}

/// `forget <session-id>`.
fn forget_verb(args: &[String]) -> Result<(), Failure> {
    let mut session_id: Option<String> = None;
    let mut cursor = Cursor::new(args);
    while let Some(arg) = cursor.next() {
        match arg {
            "-h" | "--help" => return help(),
            option if option.starts_with("--") => return Err(unknown_option(option)),
            positional => {
                if session_id.is_some() {
                    return Err(Failure::Usage("forget takes one session id".to_owned()));
                }
                session_id = Some(positional.to_owned());
            }
        }
    }
    let session_id =
        session_id.ok_or_else(|| Failure::Usage("forget needs a session id".to_owned()))?;

    // Deliberately unjournaled, and it must stay that way. Every other verb
    // writes what it decided to <root>/log; this one destroys every copy of a
    // session, and a line naming the session it destroyed would be precisely
    // the pointer the verb exists to remove.
    for path in forget::forget(&paths::data_root()?, &paths::claude_root()?, &session_id)? {
        println!("{}", path.display());
    }
    Ok(())
}

/// Print the usage as a successful answer to `--help`.
fn help() -> Result<(), Failure> {
    println!("{USAGE}");
    Ok(())
}

/// The refusal for a flag no verb has.
fn unknown_option(option: &str) -> Failure {
    Failure::Usage(format!("unknown option `{option}`"))
}

/// The whole of stdin — a hook payload.
fn stdin() -> Result<String, Failure> {
    let mut payload = String::new();
    io::stdin()
        .read_to_string(&mut payload)
        .map_err(|source| Error::io("<stdin>", source))?;
    Ok(payload)
}

/// A cursor over the arguments of one verb.
struct Cursor<'a> {
    /// The verb's arguments.
    args: &'a [String],
    /// How far parsing has read.
    index: usize,
}

impl<'a> Cursor<'a> {
    /// Start at the first argument.
    fn new(args: &'a [String]) -> Self {
        Self { args, index: 0 }
    }

    /// The next argument.
    #[allow(
        clippy::should_implement_trait,
        reason = "an inherent `next` reads better here than an Iterator impl on a one-use cursor"
    )]
    fn next(&mut self) -> Option<&'a str> {
        let arg = self.args.get(self.index)?;
        self.index += 1;
        Some(arg)
    }

    /// The value belonging to `flag`.
    fn value(&mut self, flag: &str) -> Result<String, Failure> {
        self.next()
            .map(ToOwned::to_owned)
            .ok_or_else(|| Failure::Usage(format!("{flag} needs a value")))
    }
}
