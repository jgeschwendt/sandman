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

/// The `SessionEnd` reason an operator is behind: `/clear` wipes the
/// conversation, so the transcript it ends can never be appended to again.
const CLEAR_REASON: &str = "clear";

/// `take <session-id> [--force] | --hook` (hook mode implies force).
pub(super) fn take_verb(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "take", &[], &["--force", "--hook"], 1)?;
    let from_hook = parsed.set("--hook");
    let journal = paths::data_root().ok();
    let result = take_run(
        journal.as_deref(),
        from_hook,
        parsed.set("--force"),
        parsed.positionals.first().cloned(),
    );
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
        // ※ resume-is-not-an-ending
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

/// Start `sandman dream` and walk away.
///
/// Take is called from a `SessionEnd` hook, which Claude Code will not wait
/// on, so the dream must outlive it: the child is never waited for, and both
/// its streams go to the day's dream log rather than to a terminal that is
/// already gone. Failing to start one is a note on stderr, never a failed
/// take — the queue is still there for the next run.
fn spawn_dream(data_root: &Path) -> Result<PathBuf, Error> {
    use std::os::unix::process::CommandExt;
    let log = paths::run_log(data_root, "dream", Timestamp::now()?);
    crate::atomic::ensure_parent(&log)?;
    let out = OpenOptions::new()
        .append(true)
        .create(true)
        .open(&log)
        .map_err(|source| Error::io(&log, source))?;
    let err = out.try_clone().map_err(|source| Error::io(&log, source))?;
    let exe = env::current_exe().map_err(|source| Error::io("<current exe>", source))?;
    // Its own process group, so whoever signals the hook's group (a bg
    // session's supervisor, a closing pty) no longer kills the dream with it.
    Command::new(exe)
        .process_group(0)
        .arg("dream")
        .stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn()
        .map_err(|source| Error::io("<dream>", source))?;
    Ok(log)
}
