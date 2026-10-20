//! `take`: a session's ending archived, and the dream it starts.

use std::env;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::Error;
use crate::hook;
use crate::paths;
use crate::slug::truncate_chars;
use crate::time::Timestamp;
use crate::verbs::{dream, take};

use super::{Args, EMPTY, Failure, NONE, PAYLOAD_CHARS, PIPELINE_ENV, note, stdin};

/// Set by a caller that drives `claude -p --resume` turns: each turn ends the
/// session it resumes, and the take would move the transcript out from under
/// the next one.
const NO_TAKE_ENV: &str = "SANDMAN_NO_TAKE";

/// `take <session-id> [--force] | --hook` (hook mode implies force).
pub(super) fn take_verb(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "take", &[], &["--force", "--hook"], 1)?;
    let from_hook = parsed.set("--hook");
    let journal = paths::data_root().ok();
    let result = take_run(
        journal.as_deref(),
        from_hook,
        // SessionEnd is the proof the session is over — the hook fires the
        // instant the transcript's last line lands, which is exactly what the
        // by-hand live-window heuristic refuses.
        from_hook || parsed.set("--force"),
        parsed.positionals.into_iter().next(),
    )
    .map(|drain| {
        // Never fails: a take that succeeded stays a success however the ledger went.
        if drain
            && let Some(data_root) = journal.as_deref()
            && let Ok(claude_root) = paths::claude_root()
        {
            take::drain_pending(data_root, &claude_root);
        }
    });
    // A hook has no terminal to fail into: without this line the only trace of
    // a broken take is the exit code Claude Code discards.
    if from_hook && let Err(Failure::Error(error)) = &result {
        note(journal.as_deref(), "take", &format!("error: {error}"));
    }
    result
}

/// The whole of `take` once the command line is understood; `Ok(true)` asks
/// the caller to settle the takes a live job deferred.
///
/// Every ending drains except the quiet declines (`$SANDMAN_NO_TAKE`, the
/// pipeline: they must not archive bystanders) and `resume`, a beginning that
/// does nothing.
fn take_run(
    journal: Option<&Path>,
    from_hook: bool,
    force: bool,
    session_id: Option<String>,
) -> Result<bool, Failure> {
    let say = |line: &str| note(journal, "take", line);
    let claude_root = paths::claude_root();
    let live_job = |id: &str| {
        claude_root
            .as_deref()
            .ok()
            .and_then(|root| take::live_job(root, id))
    };
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
            say("declined no-take-env");
            return Ok(false);
        }
        // A dream mind's own ending is not a session to keep: archiving it
        // feeds the very queue that spawned it, and at depth the next take
        // spawns another dream — the recursion that flooded the machine with
        // claude processes on 2026-08-19. Recall already goes silent under
        // this variable; take must too.
        if env::var(PIPELINE_ENV).as_deref() == Ok("1") {
            // One line per dream turn (rare; silence would look like the hook
            // never ran), naming the session when the payload reads — a decline
            // tied to no session is one no reclaim can account for. One that
            // will not read still declines quietly: a dream turn never fails
            // its hook.
            let session = stdin()
                .ok()
                .and_then(|payload| hook::session_end(&payload).ok())
                .and_then(|ending| ending.session_id)
                .map_or_else(String::new, |id| format!(" session={id}"));
            say(&format!("declined pipeline{session}"));
            return Ok(false);
        }
        let payload = stdin()?;
        let ending = hook::session_end(&payload).inspect_err(|error| {
            // The payload is the evidence, and a hook that cannot be
            // parsed is exactly the case nobody can reconstruct later.
            say(&format!(
                "hook payload unparseable: {error} payload={}",
                truncate_chars(payload.trim(), PAYLOAD_CHARS)
            ));
        })?;
        let named = ending.session_id.as_deref().unwrap_or(NONE);
        say(&format!(
            "hook session={named} reason={} cwd={}",
            ending.reason.as_deref().unwrap_or(NONE),
            ending.cwd.as_deref().unwrap_or(Path::new(NONE)).display()
        ));
        // Every ending is taken, with one exception, and it is the whole reason
        // the payload's `reason` is read at all: Claude Code fires `SessionEnd`
        // with `reason: "resume"` on the session it is *adopting*, at the moment
        // of adoption. Forcing there moves the transcript out from under a
        // conversation that is about to append to it — Claude Code then
        // recreates the file, the next ending takes that live fragment too,
        // and the pointer ends up naming a stub while the real conversation
        // sits orphaned in the archive (observed across twelve sessions on
        // 2026-08-25). A beginning is not an ending; decline it.
        // ※ resume-is-not-an-ending
        if ending.is_resume() {
            say(&format!("declined resume session={named}"));
            return Ok(false);
        }
        // A payload with no session is a session with nothing to take.
        let Some(session_id) = ending.session_id else {
            say("declined no-session");
            return Ok(true);
        };
        // A retirement is not an ending either. The daemon retires an idle,
        // done background worker, and its exit fires SessionEnd with an
        // ordinary reason — but a job directory still naming the session says
        // the conversation will be resumed. The guard is the hook's alone: a
        // session named by hand is taken on the operator's word, job or no job.
        // `/clear` is the one ending no worker can be behind: it closes the
        // transcript for good, and a job it ran under carries on under a new
        // session id (2026-08-28), so it alone passes the guard.
        if ending.reason.as_deref() != Some("clear")
            && let Some(data_root) = journal
            && let Some(job) = live_job(&session_id)
        {
            say(&format!("declined live-job session={session_id} job={job}"));
            // The decline is right and it is not the end of the matter: the
            // job outlives every worker, and when the operator deletes it
            // nothing fires a second `SessionEnd`. Left at the decline, a
            // transcript sat in the live set for 22 h behind a job that no
            // longer existed (2026-08-30), so the owed take is written down
            // and every later take — and reflect — retries it. Best-effort,
            // like the journal: a ledger must never fail the hook, and
            // recorded or not, the attempt leaves a line.
            say(&match Timestamp::now()
                .and_then(|now| take::remember_pending(data_root, &session_id, &job, now))
            {
                Ok(_) => format!("pending session={session_id} job={job}"),
                Err(error) => format!("pending unrecorded session={session_id}: {error}"),
            });
            return Ok(true);
        }
        session_id
    } else {
        let session_id =
            session_id.ok_or_else(|| Failure::Usage("take needs a session id".to_owned()))?;
        // The guard stays the hook's, but the take that pulled a live
        // conversation out from under its job was a by-hand one, made
        // unreconstructable only because nothing recorded the guard would have
        // fired. Recorded before the take, so one killed mid-move still leaves
        // the marker.
        say(&format!(
            "by-hand session={session_id} guard={}",
            live_job(&session_id).as_deref().unwrap_or(EMPTY)
        ));
        session_id
    };

    let data_root = paths::data_root()?;
    let outcome = match take::take(&data_root, &claude_root?, &session_id, force) {
        // `forget` destroys every copy before the session ends, so the hook
        // that follows it finds nothing — the designed sequence, not a fault.
        // Asked for by hand, a missing transcript is still an error.
        Err(Error::NotFound { .. }) if from_hook => {
            say(&format!("declined forgotten session={session_id}"));
            return Ok(true);
        }
        taken => taken?,
    };
    let directory = outcome
        .archived_directory
        .as_ref()
        .map_or_else(String::new, |directory| {
            format!(" directory={}", directory.display())
        });
    say(&format!(
        "took session={session_id} from-hook={from_hook} forced={force} archived={} bytes={} queue={}{directory}",
        outcome.archived.display(),
        outcome.archived_bytes,
        outcome.queue_depth
    ));

    println!("{}", outcome.archived.display());
    if outcome.dream_due() {
        let queue = outcome.queue_depth;
        // A held lock means a run is already draining the queue — starting
        // another would only fork a process to watch it yield.
        let (line, said) = if dream::lock_held(&data_root) {
            (
                format!("dream held queue={queue}"),
                "a dream is already running".to_owned(),
            )
        } else {
            match spawn_dream(&data_root) {
                Ok(log) => (
                    format!("dream spawned queue={queue} log={}", log.display()),
                    format!("dreaming in the background, logging to {}", log.display()),
                ),
                Err(error) => (
                    format!("dream spawn-failed queue={queue}: {error}"),
                    format!("dream could not be spawned: {error}"),
                ),
            }
        };
        say(&line);
        eprintln!("queue at {queue} — {said}");
    }
    Ok(true)
}

/// Start `sandman dream` and walk away.
///
/// Take is called from a `SessionEnd` hook, which Claude Code will not wait
/// on, so the dream must outlive it: the child is never waited for, and both
/// its streams go to the day's dream log rather than to a terminal that is
/// already gone. Failing to start one is a journal line and a note on stderr,
/// never a failed take — the queue is still there for the next run.
fn spawn_dream(data_root: &Path) -> Result<PathBuf, Error> {
    use std::os::unix::process::CommandExt;
    let log = paths::run_log(data_root, "dream", Timestamp::now()?);
    crate::atomic::ensure_parent(&log)?;
    let (out, err) = OpenOptions::new()
        .append(true)
        .create(true)
        .open(&log)
        .and_then(|out| out.try_clone().map(|err| (out, err)))
        .map_err(|source| Error::io(&log, source))?;
    let exe = env::current_exe().map_err(|source| Error::io("<current exe>", source))?;
    // Its own process group, so whoever signals the hook's group (a bg
    // session's supervisor, a closing pty) no longer kills the dream with it.
    Command::new(exe)
        .process_group(0)
        .arg("dream")
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .spawn()
        .map_err(|source| Error::io("<dream>", source))?;
    Ok(log)
}
