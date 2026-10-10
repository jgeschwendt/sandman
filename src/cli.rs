//! The dispatcher: `sandman <verb> [args]`.
//!
//! Hand-rolled parsing, std only. Four exit codes and nothing else — 0 for a
//! verb that ran, 1 for a failure (one line on stderr), 2 for anything the
//! operator has to fix in the command line itself, unbuilt verbs included, and
//! 3 for a fault a runner must tell apart from a refusal — `shift clock`
//! finding its window file there but unreadable, `breathe pass` without a
//! template or a valid `sources.toml`, `shift brief` with no trusted trunk.
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
use crate::grove;
use crate::hook;
use crate::journal;
use crate::memory::MemoryType;
use crate::paths;
use crate::slug::truncate_chars;
use crate::time::Timestamp;
use crate::verbs::{
    breathe, dream, forget, mcp, recall, reconcile, reflect, remember, score, shift, take, trail,
};
use crate::version::VERSION;

/// A failure that ran a verb badly.
const EXIT_ERROR: u8 = 1;
/// A failure in the command line itself.
const EXIT_USAGE: u8 = 2;
/// A fault a runner reports rather than waits on: a clock that cannot read
/// its window file, a pass that cannot read its template or sources.
const EXIT_FAULT: u8 = 3;

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

  breathe init
  breathe sources [--due] [--kind skim|deep] [--json]
  breathe file --url U --repo R --title T --problem P --summary S
               [--source NAME] [--bearing F] [--novelty F] [--spike]
  breathe list [--repo R] [--state new|spiked|adopted|dropped] [--json]
  breathe spike|adopt|drop <file> [--reason TEXT]
  breathe pass --kind skim|deep [--dry-run]
  breathe attach --id ID [--bg-id X] [--session-id S]
  breathe close --id ID --read N --filed M --dropped K --cost USD
                [--cursor NAME=VALUE]...
  breathe reweight [--dry-run]
      The ruach's store under <root>/breath/; never calls a model or the
      network. init makes breath/, ledger/ and passes/ and seeds sources.toml.
      sources lists the feeds; --due (or --kind) only those a pass reads now —
      a skim those unread for an hour, a deep pass all. file writes
      breath/<date>-<slug>.md (state new, or spiked with --spike), counts it
      on its --source and appends a breath event — unless the normalized URL,
      or within the same repo a title with token Jaccard >= 0.6 and the same
      numbers, matches an existing breath (a dropped one only for its own
      repo), which then counts seen_again instead. --repo is * or owner/name. spike takes new; adopt
      takes new|spiked; drop needs --reason; adopt and drop append a breath
      event ($SANDMAN_SHIFT adds adopted-by:<shift>). pass fills every
      {{slot}} of prompts/breathe.md, writes passes/<id>/pass.json and
      passes/<id>/prompt.md, and prints the id; an open pass past its timeout
      (skim 15 min, deep 60 min from started) is marked dead and charged
      $SANDMAN_BREATH_DEAD_COST_USD (default 2). It exits 1 with `nothing
      due` when no source is due, `pass in flight` while a pass of the same
      kind is open within its timeout, or `budget spent` when today's closed
      and dead passes have spent $SANDMAN_BREATH_BUDGET_USD (default 10; a
      pass in flight counts 0 and is listed), and 3 when the template or
      sources.toml cannot be read; --dry-run prints the prompt and writes
      nothing. attach records the pass's session; close requires --cost (0
      journals a cost zero note), records its counters,
      advances last_read (and each --cursor) on the sources it listed, and
      appends a breath event.
      reweight moves each weight +0.1 per breath whose adopted: stamp is
      after the last reweight (cap 2.0) and -0.1 after 30 days of silence (floor 0.2),
      rewrites sources.toml (every rewrite drops its comments) and appends a
      breath event; --dry-run prints the table only. $SANDMAN_NOW pins the instant.

  dream [--now]
      Route the queue: for each undreamed pointer, three minds (sonnet, opus,
      fable) read the archived conversation in parallel and propose memories;
      a memory commits on 2-of-3 agreement. Take spawns this at queue depth
      10; --now is documentation, both forms behave the same. Models move with
      $SANDMAN_MIND_SONNET / _OPUS / _FABLE, the binary with
      $SANDMAN_CLAUDE_BIN. Each mind's own transcript is kept at
      <root>/.dream/<claude project>/<session-id>.jsonl.

  reflect [--day <yyyy-mm-dd>]
      The 24 h pass: the voyage entry for the day that ended (one
      $SANDMAN_MIND_LOG call, skipped when the day's sources are unchanged),
      the log index, the pointer sweep (dreamt and older than 72 h), and one
      gated opus upkeep call per grown bank ($SANDMAN_MIND_UPKEEP). --day
      renders that day's entry only — no drain, no sweep, no upkeep.

  mcp
      Serve the banks over MCP on stdio: JSON-RPC 2.0, one message per line,
      requests on stdin, responses on stdout, nothing else on stdout ever.
      Three tools — recall, remember, banks — over the same code paths the
      verbs use. Journals to <root>/.trace/mcp-<date>.log. Exits 0 on EOF.
      Register: claude mcp add --scope user sandman -- ~/.local/bin/sandman mcp

  reconcile <event-id|shift-id> [--dry-run]
      Walk the trail back past a wrong turn: the event and every event whose
      parent chain reaches it are revoked — for a shift id, every event of the
      shift and theirs; every commit carrying one of them in an Event: trailer
      (or, for a shift id, naming it in Shift:) — in each checkout under ~/.grove/code
      ($SANDMAN_GROVE_CODE) and in ~, ~/.routines and the data root
      ($SANDMAN_RECONCILE_EXTRA, colon-separated) — that the repo's trunk
      reaches is reverted newest-first on a reconcile/<event-id> branch cut
      from trunk (the manifest's trunk — one that does not resolve fails —
      else main, else origin/HEAD; a repo with none, or a trunk behind
      origin, fails), in a scratch worktree, with the repo's hooks off; hits
      trunk does not reach are reported off trunk. A conflict ends that repo,
      and so does a trailered merge: the machine lands by fast-forward only,
      so merges are never reverted but left to the operator. Appends one
      reconcile event — none when nothing new was reverted — and lists the
      memories whose source: names a revoked event, for reflect to retire;
      exits 1 when any repo ended in a conflict or a failure. --dry-run
      prints the plan and changes nothing.

  score [--week yyyy-Www] [--json]
      The week's numbers from the trail, shifts/*/meta.json, the breath
      store, ~/.threads ($SANDMAN_THREADS) and the shift and breathe-*
      runs.jsonl under ~/.routines/.state ($SANDMAN_ROUTINES_STATE): repo
      coverage, shift outcomes, events by kind/repo/model, cost, recurring
      corrections, fresh context, breaths, open threads, routine ticks. The
      week is ISO, its days New York's, the current one by default
      ($SANDMAN_NOW pins it). Rewrites <root>/score/<yyyy>-W<ww>.md —
      frontmatter, the worst three, a table per section — and prints its
      path; --json prints the metrics object instead. No model, no git.

  shift attach --shift ID [--bg-id X] [--session-id S]
  shift brief [--mind M] [--dry-run]
  shift clock [--check]
  shift handoff --shift ID (--from-session | --abandoned [--reason TEXT] |
                --file PATH) [--truncate]
  shift pick [--path] [--shift ID | --untrusted]
  shift profile [--kind shift|breath]
  shift status
      The perpetual machine's unit of work, bounded by the five-hour window.
      brief picks the target, fills every {{slot}} of prompts/shift.md
      ($SANDMAN_PROMPTS, else the sandman trunk's prompts/) under a
      24,000-character cap (the Context slots fenced as data, cut in a fixed
      order, the template's own text never — exit 1 when it still will not
      fit), writes <root>/shifts/<id>/{brief.md,meta.json},
      takes shifts/current.json, refreshes breath/problems.md, appends a
      shift.start event whose refs fingerprint every slot, and prints the id;
      --dry-run prints the brief and writes nothing. The theme is the
      America/New_York weekday's: Sun Hunt, Mon Redesign, Tue Refactor, Wed
      Measure, Thu Harden, Fri Ship, Sat Rest. pick is the rotation — the least
      recently shifted trunk of the grove's $SANDMAN_SHIFT_OWNERS repos
      (default jgeschwendt) — never ~, ~/.routines or the data root — ties
      broken by a failing stele check, open threads and breaths; it skips the
      open shift's repo, $SANDMAN_SHIFT_SKIP (colon-separated slugs) and
      every trunk Claude Code has not trusted ($SANDMAN_CLAUDE_JSON, else
      ~/.claude.json) — exit 1 listing them when no other is left, and brief
      exits 3; --untrusted lists those trunks; --shift prints the target a
      shift was briefed on. clock reads the window
      file ($CLAUDE_WINDOW_FILE, else $XDG_STATE_HOME/claude/window.json, else
      ~/.local/state/claude/window.json); --check exits 1 under
      $SANDMAN_SHIFT_MIN_WINDOW_MIN (default 60) minutes left or at 90% used,
      0 when the file is missing; clock and --check exit 3 when the file is
      there but cannot be read, is not a five_hour object with numeric
      resets_at and non-negative used_percentage, is stamped at more than
      5 min ahead, or resets more than 6 h ahead. handoff finalizes a shift —
      copies --file in, accepts the handoff.md the session wrote, or records
      --abandoned (--reason TEXT, only beside --abandoned, heads the handoff
      and is the event's rationale as `Abandoned: <reason>`) — appends
      shift.handoff and releases current.json; a shift already handed off
      exits 1, and so does a handoff over 2,000 characters
      unless --truncate keeps its head and notes the cut (--abandoned always
      truncates). attach records the session the runner
      started in current.json. status prints current.json and the clock.
      profile prints the Claude Code settings a shift or breath session runs
      under (profiles/session.json, compiled in; one profile for both kinds).
      $SANDMAN_NOW (ISO-8601) fixes the instant; $SANDMAN_SHIFT_MIND (default
      opus) and $SANDMAN_SHIFT_BUDGET_USD (default 15) fill the brief.

  trail append --kind K [--shift S] [--repo R] [--branch B] [--sha SHA]
               [--rationale TEXT] [--cost USD] [--model M] [--parent ID]
               [--ref X]... [--json '<object>']
  trail log [--since yyyy-mm-dd] [--kind K] [--shift S] [--repo R] [--json]
  trail show <id>
  trail revoke <event-id|shift-id> --reason TEXT
  trail today
  trail trailers --shift S --event E
      The append-only event log, one JSONL file per UTC day under
      <root>/trail/<yyyy>/<mm>/<dd>.jsonl. append prints the new id; log
      prints newest last; show adds the event's descendants and the commits
      carrying it; revoke appends a revoke event (once — a second is a note),
      parent the event, or ref shift:<id> for a whole shift;
      today prints today's file; trailers prints the Shift:/Event: lines every
      machine commit carries. Kinds: breath change correction decay guard
      promote reconcile revoke shift.handoff shift.start.

  version
      The crate version and the commit it was built from — the same stamp
      every journal line carries as v=, so a line can be read against the
      build that wrote it rather than whatever is checked out now.

Every verb journals its decisions — one line each, stamped and carrying the
writing pid and build — to <root>/.trace/<verb>-<date>.log, so a hook that
declined can still be explained afterwards. forget alone is silent,
deliberately: a line naming the session it destroyed would be the trace it
promises not to leave.

Data root: $SANDMAN_ROOT, else ~/.sandman. Transcripts: ~/.claude/projects/.
Logs: <root>/.trace/<verb>-<date>.log.";

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
        Err(Failure::Fault(error)) => {
            eprintln!("sandman: {error}");
            ExitCode::from(EXIT_FAULT)
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
    /// A runner's fault: the clock's window file, the pass's template or
    /// sources.
    Fault(Error),
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
        "breathe" => breathe_verb(rest),
        "dream" => dream_verb(rest),
        "forget" => forget_verb(rest),
        "mcp" => mcp_verb(rest),
        "recall" => recall_verb(rest),
        "reconcile" => reconcile_verb(rest),
        "reflect" => reflect_verb(rest),
        "remember" => remember_verb(rest),
        "score" => score_verb(rest),
        "shift" => shift_verb(rest),
        "take" => take_verb(rest),
        "trail" => trail_verb(rest),
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

/// `reflect [--day <yyyy-mm-dd>]`.
fn reflect_verb(args: &[String]) -> Result<(), Failure> {
    let mut day: Option<reflect::Day> = None;
    let mut cursor = Cursor::new(args);
    while let Some(arg) = cursor.next() {
        match arg {
            "-h" | "--help" => return help(),
            "--day" => {
                let value = cursor.value("--day")?;
                day = Some(calendar_day(&value).ok_or_else(|| {
                    Failure::Usage(format!("--day wants yyyy-mm-dd, got `{value}`"))
                })?);
            }
            option if option.starts_with("--") => return Err(unknown_option(option)),
            positional => {
                return Err(Failure::Usage(format!(
                    "reflect takes no arguments, got `{positional}`"
                )));
            }
        }
    }
    match day {
        Some(day) => reflect_one_day(day),
        None => reflect_pass(),
    }
}

/// A `yyyy-mm-dd` argument as a calendar day. Shape only — a day the calendar
/// does not hold simply has no sources, which the entry step already answers.
fn calendar_day(text: &str) -> Option<reflect::Day> {
    let mut fields = text.split('-');
    let (year, month, day) = (fields.next()?, fields.next()?, fields.next()?);
    if fields.next().is_some() || (year.len(), month.len(), day.len()) != (4, 2, 2) {
        return None;
    }
    Some(reflect::Day {
        day: day.parse().ok()?,
        month: month.parse().ok()?,
        year: year.parse().ok()?,
    })
}

/// The whole 24 h pass.
fn reflect_pass() -> Result<(), Failure> {
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
            "reflect done banks={} due={} swept={} entry={} ms={}",
            outcome.banks.len(),
            outcome.due,
            outcome.swept,
            outcome.entry_status,
            started.elapsed().as_millis()
        ),
    );
    println!(
        "{}",
        entry_line(outcome.entry.as_deref(), outcome.entry_status)
    );
    println!("{}", outcome.index.display());
    eprintln!(
        "swept {} pointer(s); {} bank(s) considered",
        outcome.swept,
        outcome.banks.len()
    );
    Ok(())
}

/// One past day's entry and the index behind it, and nothing else — the
/// backfill hand-hold: no drain, no sweep, no upkeep.
fn reflect_one_day(day: reflect::Day) -> Result<(), Failure> {
    let journal = paths::data_root().ok();
    let started = Instant::now();
    let data_root = paths::data_root()?;
    let options = reflect::Options::from_env();
    let rendered = reflect::entry_step(&data_root, day, &options)
        .and_then(|rendered| reflect::write_index(&data_root, day).map(|_| rendered));
    let (entry, status) = match rendered {
        Ok(rendered) => rendered,
        Err(error) => {
            note(
                journal.as_deref(),
                "reflect",
                &format!(
                    "reflect error day={} ms={}: {error}",
                    day.key(),
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
            "reflect done day={} entry={status} ms={}",
            day.key(),
            started.elapsed().as_millis()
        ),
    );
    println!("{}", entry_line(entry.as_deref(), status));
    Ok(())
}

/// What the entry step has to show for itself: the path when it wrote one,
/// else the word that says why it did not.
fn entry_line(entry: Option<&Path>, status: &str) -> String {
    entry.map_or_else(|| status.to_owned(), |path| path.display().to_string())
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

/// `mcp` — the banks as a tool server, JSON-RPC on stdio.
///
/// The one verb whose whole conversation is stdout, so nothing else may print
/// there for as long as it runs. It takes no arguments: a client launches it
/// with none, and an argument here is a registration to fix rather than
/// something to guess at.
fn mcp_verb(args: &[String]) -> Result<(), Failure> {
    if let Some(arg) = args.first() {
        return match arg.as_str() {
            "-h" | "--help" => help(),
            option if option.starts_with("--") => Err(unknown_option(option)),
            positional => Err(Failure::Usage(format!(
                "mcp takes no arguments, got `{positional}`"
            ))),
        };
    }
    // The roots resolve here, as they do for every verb: the handlers below
    // take them as arguments and read the environment for nothing.
    let ctx = mcp::Context {
        data_root: paths::data_root()?,
        home: paths::home()?,
        session_id: env::var(SESSION_ENV).ok(),
    };
    // Locked for the whole conversation: one client, one message at a time, and
    // no other writer to interleave with on either stream.
    mcp::serve(&ctx, io::stdin().lock(), io::stdout().lock())
        .map_err(|source| Failure::Error(Error::io("<stdout>", source)))
}

/// `trail <subcommand> …`.
fn trail_verb(args: &[String]) -> Result<(), Failure> {
    let Some(sub) = args.first() else {
        return Err(Failure::Usage("trail needs a subcommand".to_owned()));
    };
    let rest = &args[1..];
    match sub.as_str() {
        "-h" | "--help" => help(),
        "append" => trail_append(rest),
        "log" => trail_log(rest),
        "revoke" => trail_revoke(rest),
        "show" => trail_show(rest),
        "today" => {
            if let Some(extra) = rest.first() {
                return Err(Failure::Usage(format!(
                    "trail today takes no arguments, got `{extra}`"
                )));
            }
            println!("{}", trail::today(&paths::data_root()?)?.display());
            Ok(())
        }
        "trailers" => trail_trailers(rest),
        other => Err(Failure::Usage(format!(
            "unknown trail subcommand `{other}`"
        ))),
    }
}

/// `trail append --kind K [flags] | --json '<object>'`.
fn trail_append(args: &[String]) -> Result<(), Failure> {
    let mut draft = trail::Draft::default();
    let mut flags: Vec<(&str, String)> = Vec::new();
    let mut cursor = Cursor::new(args);
    while let Some(arg) = cursor.next() {
        match arg {
            "-h" | "--help" => return help(),
            "--json" => {
                let value = cursor.value("--json")?;
                draft = trail::Draft::from_json(&value)
                    .map_err(|error| Failure::Usage(format!("--json: {error}")))?;
            }
            "--branch" | "--cost" | "--kind" | "--model" | "--parent" | "--rationale" | "--ref"
            | "--repo" | "--sha" | "--shift" => flags.push((arg, cursor.value(arg)?)),
            option if option.starts_with("--") => return Err(unknown_option(option)),
            positional => {
                return Err(Failure::Usage(format!(
                    "trail append takes no positional arguments, got `{positional}`"
                )));
            }
        }
    }
    // Flags land after `--json`, so a flag overrides the object's key.
    for (flag, value) in flags {
        match flag {
            "--branch" => draft.branch = Some(value),
            "--cost" => {
                draft.cost_usd = Some(
                    value
                        .parse::<f64>()
                        .ok()
                        .filter(|cost| cost.is_finite() && *cost >= 0.0)
                        .ok_or_else(|| {
                            Failure::Usage(format!("--cost wants USD, got `{value}`"))
                        })?,
                );
            }
            "--kind" => draft.kind = value,
            "--model" => draft.model = Some(value),
            "--parent" => draft.parent = Some(value),
            "--rationale" => draft.rationale = Some(value),
            "--ref" => draft.refs.push(value),
            "--repo" => draft.repo = Some(value),
            "--sha" => draft.sha = Some(value),
            _ => draft.shift = Some(value),
        }
    }
    if draft.kind.is_empty() {
        return Err(Failure::Usage("trail append needs --kind".to_owned()));
    }
    if !trail::is_kind(&draft.kind) {
        return Err(Failure::Usage(format!("unknown kind `{}`", draft.kind)));
    }
    let data_root = paths::data_root()?;
    let event = trail::append(&data_root, draft)?;
    note(
        Some(&data_root),
        "trail",
        &format!("appended id={} kind={}", event.id, event.draft.kind),
    );
    println!("{}", event.id);
    Ok(())
}

/// `trail log [--since D] [--kind K] [--shift S] [--repo R] [--json]`.
fn trail_log(args: &[String]) -> Result<(), Failure> {
    let (mut since, mut kind, mut shift, mut repo, mut raw) = (None, None, None, None, false);
    let mut cursor = Cursor::new(args);
    while let Some(arg) = cursor.next() {
        match arg {
            "-h" | "--help" => return help(),
            "--json" => raw = true,
            "--kind" => kind = Some(cursor.value("--kind")?),
            "--repo" => repo = Some(cursor.value("--repo")?),
            "--shift" => shift = Some(cursor.value("--shift")?),
            "--since" => {
                let value = cursor.value("--since")?;
                if calendar_day(&value).is_none() {
                    return Err(Failure::Usage(format!(
                        "--since wants yyyy-mm-dd, got `{value}`"
                    )));
                }
                since = Some(value);
            }
            option if option.starts_with("--") => return Err(unknown_option(option)),
            positional => {
                return Err(Failure::Usage(format!(
                    "trail log takes no positional arguments, got `{positional}`"
                )));
            }
        }
    }
    let entries = trail::read(&paths::data_root()?, since.as_deref())?;
    let matches = |wanted: &Option<String>, have: Option<&str>| {
        wanted.as_deref().is_none_or(|wanted| have == Some(wanted))
    };
    for entry in &entries {
        let draft = &entry.event.draft;
        if !matches(&kind, Some(&draft.kind))
            || !matches(&shift, draft.shift.as_deref())
            || !matches(&repo, draft.repo.as_deref())
        {
            continue;
        }
        if raw {
            println!("{}", entry.raw);
        } else {
            println!("{}", entry.event.summary());
        }
    }
    Ok(())
}

/// `trail show <id>`.
fn trail_show(args: &[String]) -> Result<(), Failure> {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        return help();
    }
    let id = one_id(args, "trail show")?;
    let entries = trail::read(&paths::data_root()?, None)?;
    let event = trail::find(&entries, &id).ok_or_else(|| Error::not_found("event", id.clone()))?;
    println!("{}", event.pretty());
    let descendants = trail::descendants(&entries, &id);
    println!("\ndescendants:");
    if descendants.is_empty() {
        println!("  -");
    }
    for descendant in &descendants {
        println!("  {}", descendant.summary());
    }
    let revocations = trail::revocations(&entries, &id);
    if !revocations.is_empty() {
        println!("\nrevoked:");
        for revocation in revocations {
            println!("  {}", revocation.summary());
        }
    }
    let mut events: std::collections::BTreeSet<String> =
        descendants.iter().map(|event| event.id.clone()).collect();
    events.insert(id);
    println!("\ncommits:");
    let repos = grove::repos_with_hits(&grove_roots()?, &events, None);
    if repos.is_empty() {
        println!("  -");
    }
    for repo in repos {
        for hit in repo.hits {
            println!("  {}@{}  {}", repo.label, hit.sha, hit.event);
        }
    }
    Ok(())
}

/// `trail revoke <id> --reason TEXT`.
fn trail_revoke(args: &[String]) -> Result<(), Failure> {
    let mut id: Option<String> = None;
    let mut reason: Option<String> = None;
    let mut cursor = Cursor::new(args);
    while let Some(arg) = cursor.next() {
        match arg {
            "-h" | "--help" => return help(),
            "--reason" => reason = Some(cursor.value("--reason")?),
            option if option.starts_with("--") => return Err(unknown_option(option)),
            positional => {
                if id.is_some() {
                    return Err(Failure::Usage("trail revoke takes one id".to_owned()));
                }
                id = Some(positional.to_owned());
            }
        }
    }
    let id = id.ok_or_else(|| Failure::Usage("trail revoke needs an id".to_owned()))?;
    let reason = reason
        .filter(|reason| !reason.trim().is_empty())
        .ok_or_else(|| Failure::Usage("trail revoke needs --reason".to_owned()))?;
    let data_root = paths::data_root()?;
    match trail::revoke(&data_root, &id, &reason)? {
        trail::Revoked::Already(earlier) => {
            eprintln!("{id} is already revoked by {}", earlier.id);
            println!("{}", earlier.id);
        }
        trail::Revoked::Now(event) => {
            note(
                Some(&data_root),
                "trail",
                &format!("revoked id={id} by={}", event.id),
            );
            println!("{}", event.id);
        }
    }
    Ok(())
}

/// `trail trailers --shift S --event E`.
fn trail_trailers(args: &[String]) -> Result<(), Failure> {
    let (mut shift, mut event) = (None, None);
    let mut cursor = Cursor::new(args);
    while let Some(arg) = cursor.next() {
        match arg {
            "-h" | "--help" => return help(),
            "--event" => event = Some(cursor.value("--event")?),
            "--shift" => shift = Some(cursor.value("--shift")?),
            option if option.starts_with("--") => return Err(unknown_option(option)),
            positional => {
                return Err(Failure::Usage(format!(
                    "trail trailers takes no positional arguments, got `{positional}`"
                )));
            }
        }
    }
    let (Some(shift), Some(event)) = (shift, event) else {
        return Err(Failure::Usage(
            "trail trailers needs --shift and --event".to_owned(),
        ));
    };
    println!("{}", trail::trailers(&shift, &event));
    Ok(())
}

/// The single id a subcommand takes.
fn one_id(args: &[String], what: &str) -> Result<String, Failure> {
    match args {
        [id] if !id.starts_with("--") => Ok(id.clone()),
        [option, ..] if option.starts_with("--") => Err(unknown_option(option)),
        _ => Err(Failure::Usage(format!("{what} takes one id"))),
    }
}

/// The repos `reconcile` and `trail show` reach, resolved from `crate::paths`.
fn grove_roots() -> Result<grove::Roots, Failure> {
    let grove_code = paths::grove_code()?;
    Ok(grove::Roots {
        extras: paths::reconcile_extras()?,
        home: paths::home()?,
        manifest: paths::grove_manifest(&grove_code),
        grove_code,
    })
}

/// `reconcile <event-id> [--dry-run]`.
fn reconcile_verb(args: &[String]) -> Result<(), Failure> {
    let mut dry_run = false;
    let mut id: Option<String> = None;
    let mut cursor = Cursor::new(args);
    while let Some(arg) = cursor.next() {
        match arg {
            "-h" | "--help" => return help(),
            "--dry-run" => dry_run = true,
            option if option.starts_with("--") => return Err(unknown_option(option)),
            positional => {
                if id.is_some() {
                    return Err(Failure::Usage("reconcile takes one id".to_owned()));
                }
                id = Some(positional.to_owned());
            }
        }
    }
    let id = id.ok_or_else(|| Failure::Usage("reconcile needs an event or shift id".to_owned()))?;
    let data_root = paths::data_root()?;
    let home = paths::home()?;
    let report = reconcile::reconcile(&data_root, &grove_roots()?, &id, dry_run)?;
    if dry_run {
        println!("dry run — nothing changed");
    }
    println!("revoked: {}", report.revoked.join(" "));
    for repo in &report.repos {
        println!(
            "\n{} ({}) on {} from {}",
            repo.label,
            paths::tildify(&repo.checkout, &home),
            repo.branch,
            if repo.base.is_empty() {
                "-"
            } else {
                &repo.base
            }
        );
        for line in &repo.notes {
            println!("  note: {line}");
        }
        for (label, shas) in [
            ("planned", &repo.planned),
            ("reverted", &repo.reverted),
            ("conflict", &repo.conflicts),
            ("already reverted", &repo.already),
            ("off trunk", &repo.off_trunk),
        ] {
            if dry_run && label == "reverted" {
                continue;
            }
            // a conflicting hit prints under `conflict:` alone
            for sha in shas
                .iter()
                .filter(|sha| label != "planned" || !repo.conflicts.contains(sha))
            {
                println!("  {label}: {sha}");
            }
        }
        if let Some(failed) = &repo.failed {
            println!("  failed: {failed}");
        }
    }
    if report.repos.is_empty() {
        println!("\nno commit carries a revoked event");
    }
    println!("\nfor reflect to retire:");
    if report.memories.is_empty() {
        println!("  -");
    }
    for memory in &report.memories {
        println!("  {}", paths::tildify(memory, &home));
    }
    if let Some(event) = &report.event {
        note(
            Some(&data_root),
            "reconcile",
            &format!(
                "reconciled target={id} event={} revoked={} repos={}",
                event.id,
                report.revoked.len(),
                report.repos.len()
            ),
        );
        println!("\nevent: {}", event.id);
    } else if !dry_run {
        println!("\nevent: none — nothing new to revert, the trail is unchanged");
    }
    for repo in &report.repos {
        for line in &repo.notes {
            note(
                Some(&data_root),
                "reconcile",
                &format!("repo={} {line}", repo.label),
            );
        }
    }
    let troubled: Vec<&str> = report
        .repos
        .iter()
        .filter(|repo| repo.troubled())
        .map(|repo| repo.label.as_str())
        .collect();
    if !troubled.is_empty() {
        return Err(Error::refused(format!(
            "reconcile left a conflict or failure in {}",
            troubled.join(", ")
        ))
        .into());
    }
    Ok(())
}

/// `score [--week YYYY-Www] [--json]`.
fn score_verb(args: &[String]) -> Result<(), Failure> {
    let (values, set) = shift_flags(args, "score", &["--week"], &["--json"])?;
    let ctx = shift_context()?;
    let week = match flag(&values, "--week") {
        None => score::Week::of(ctx.now),
        Some(text) => score::Week::parse(&text)
            .ok_or_else(|| Failure::Usage(format!("--week wants yyyy-Www, got `{text}`")))?,
    };
    let scored = score::score(&ctx, &paths::routines_state()?, week)?;
    let path = score::write(&ctx, &scored)?;
    note(
        Some(&ctx.data_root),
        "score",
        &format!("scored week={} path={}", week.name(), path.display()),
    );
    if set.iter().any(|switch| switch == "--json") {
        println!("{}", scored.json());
    } else {
        println!("{}", path.display());
    }
    Ok(())
}

/// `shift <subcommand> …`.
fn shift_verb(args: &[String]) -> Result<(), Failure> {
    let Some(sub) = args.first() else {
        return Err(Failure::Usage("shift needs a subcommand".to_owned()));
    };
    let rest = &args[1..];
    if rest.iter().any(|arg| arg == "-h" || arg == "--help") {
        return help();
    }
    match sub.as_str() {
        "-h" | "--help" => help(),
        "attach" => shift_attach(rest),
        "brief" => shift_brief(rest),
        "clock" => shift_clock(rest),
        "handoff" => shift_handoff(rest),
        "pick" => shift_pick(rest),
        "profile" => shift_profile(rest),
        "status" => shift_status(rest),
        other => Err(Failure::Usage(format!(
            "unknown shift subcommand `{other}`"
        ))),
    }
}

/// Everything the shift verbs read from the environment, resolved once.
fn shift_context() -> Result<shift::Context, Failure> {
    let now = pinned_now()?;
    let list = |name: &str, default: &[&str]| -> Vec<String> {
        env::var(name).map_or_else(
            |_| default.iter().map(|item| (*item).to_owned()).collect(),
            |value| {
                value
                    .split(':')
                    .filter(|item| !item.is_empty())
                    .map(ToOwned::to_owned)
                    .collect()
            },
        )
    };
    let home = paths::home()?;
    Ok(shift::Context {
        claude_json: paths::claude_json()?,
        data_root: paths::data_root()?,
        now,
        min_window_seconds: min_window_minutes()? * 60,
        owners: list("SANDMAN_SHIFT_OWNERS", &["jgeschwendt"]),
        prompts: paths::prompts_dir()?,
        roots: grove_roots()?,
        skip: list("SANDMAN_SHIFT_SKIP", &[]),
        threads: paths::threads_dir(&home),
        window: paths::claude_window_file()?,
        home,
    })
}

/// `$SANDMAN_SHIFT_MIN_WINDOW_MIN`, else the default: whole minutes, ≥ 0.
fn min_window_minutes() -> Result<i64, Failure> {
    match env::var("SANDMAN_SHIFT_MIN_WINDOW_MIN") {
        Ok(text) if !text.is_empty() => text
            .parse::<i64>()
            .ok()
            .filter(|minutes| (0..=300).contains(minutes))
            .ok_or_else(|| {
                Failure::Usage(format!(
                    "$SANDMAN_SHIFT_MIN_WINDOW_MIN wants minutes 0–300, got `{text}`"
                ))
            }),
        _ => Ok(shift::DEFAULT_MIN_WINDOW_MINUTES),
    }
}

/// A shift subcommand's parsed command line: `(flag, value)` pairs and the
/// switches that were set.
type ShiftArgs = (Vec<(String, String)>, Vec<String>);

/// A shift subcommand's flags: each named flag takes a value, each switch
/// none; positionals are refused.
fn shift_flags(
    args: &[String],
    what: &str,
    flags: &[&str],
    switches: &[&str],
) -> Result<ShiftArgs, Failure> {
    let (mut values, mut set) = (Vec::new(), Vec::new());
    let mut cursor = Cursor::new(args);
    while let Some(arg) = cursor.next() {
        if flags.contains(&arg) {
            values.push((arg.to_owned(), cursor.value(arg)?));
        } else if switches.contains(&arg) {
            set.push(arg.to_owned());
        } else if arg.starts_with("--") {
            return Err(unknown_option(arg));
        } else {
            return Err(Failure::Usage(format!(
                "{what} takes no positional arguments, got `{arg}`"
            )));
        }
    }
    Ok((values, set))
}

/// The last value given for `flag`.
fn flag(values: &[(String, String)], flag: &str) -> Option<String> {
    values
        .iter()
        .rev()
        .find(|(name, _)| name == flag)
        .map(|(_, value)| value.clone())
}

/// `breathe <init|sources|file|list|spike|adopt|drop|pass|attach|close|reweight> …`.
fn breathe_verb(args: &[String]) -> Result<(), Failure> {
    let Some(sub) = args.first() else {
        return Err(Failure::Usage("breathe needs a subcommand".to_owned()));
    };
    let rest = &args[1..];
    if rest.iter().any(|arg| arg == "-h" || arg == "--help") {
        return help();
    }
    match sub.as_str() {
        "-h" | "--help" => help(),
        "adopt" => breathe_transition(rest, breathe::Transition::Adopt),
        "attach" => breathe_attach(rest),
        "close" => breathe_close(rest),
        "drop" => breathe_transition(rest, breathe::Transition::Drop),
        "file" => breathe_file(rest),
        "init" => breathe_init(rest),
        "list" => breathe_list(rest),
        "pass" => breathe_pass(rest),
        "reweight" => breathe_reweight(rest),
        "sources" => breathe_sources(rest),
        "spike" => breathe_transition(rest, breathe::Transition::Spike),
        other => Err(Failure::Usage(format!(
            "unknown breathe subcommand `{other}`"
        ))),
    }
}

/// Everything the breathe verbs read from the environment, resolved once.
fn breathe_context() -> Result<breathe::Context, Failure> {
    Ok(breathe::Context {
        data_root: paths::data_root()?,
        now: pinned_now()?,
        prompts: paths::prompts_dir()?,
        shift: env::var("SANDMAN_SHIFT")
            .ok()
            .filter(|shift| !shift.is_empty()),
    })
}

/// `$SANDMAN_NOW`, else the clock.
fn pinned_now() -> Result<Timestamp, Failure> {
    match env::var("SANDMAN_NOW").ok().filter(|now| !now.is_empty()) {
        Some(text) => Timestamp::parse_iso8601(&text)
            .ok_or_else(|| Failure::Usage(format!("$SANDMAN_NOW wants ISO-8601, got `{text}`"))),
        None => Ok(Timestamp::now()?),
    }
}

/// A breathe subcommand's parsed command line: `(flag, value)` pairs in
/// order (a flag may repeat), the switches set, and the positionals.
struct BreatheArgs {
    positionals: Vec<String>,
    switches: Vec<String>,
    values: Vec<(String, String)>,
}

impl BreatheArgs {
    /// Parse `args`: each named flag takes a value, each switch none, and at
    /// most `positionals` bare arguments are accepted.
    fn parse(
        args: &[String],
        what: &str,
        flags: &[&str],
        switches: &[&str],
        positionals: usize,
    ) -> Result<Self, Failure> {
        let mut parsed = Self {
            positionals: Vec::new(),
            switches: Vec::new(),
            values: Vec::new(),
        };
        let mut cursor = Cursor::new(args);
        while let Some(arg) = cursor.next() {
            if flags.contains(&arg) {
                parsed.values.push((arg.to_owned(), cursor.value(arg)?));
            } else if switches.contains(&arg) {
                parsed.switches.push(arg.to_owned());
            } else if arg.starts_with("--") {
                return Err(unknown_option(arg));
            } else if parsed.positionals.len() < positionals {
                parsed.positionals.push(arg.to_owned());
            } else {
                return Err(Failure::Usage(format!(
                    "{what} takes {positionals} positional argument(s), got `{arg}`"
                )));
            }
        }
        Ok(parsed)
    }

    /// The last value given for `flag`.
    fn flag(&self, name: &str) -> Option<String> {
        flag(&self.values, name)
    }

    /// The value for `flag`, which must be there.
    fn required(&self, name: &str, what: &str) -> Result<String, Failure> {
        self.flag(name)
            .ok_or_else(|| Failure::Usage(format!("{what} needs {name}")))
    }

    /// Whether `switch` was set.
    fn set(&self, switch: &str) -> bool {
        self.switches.iter().any(|set| set == switch)
    }

    /// `flag` as a number, when given.
    fn number(&self, name: &str) -> Result<Option<f64>, Failure> {
        self.flag(name)
            .map(|text| {
                text.parse::<f64>()
                    .ok()
                    .filter(|number| number.is_finite())
                    .ok_or_else(|| Failure::Usage(format!("{name} wants a number, got `{text}`")))
            })
            .transpose()
    }

    /// `flag` as a count, which must be there.
    fn count(&self, name: &str, what: &str) -> Result<u64, Failure> {
        let text = self.required(name, what)?;
        text.parse::<u64>()
            .map_err(|_| Failure::Usage(format!("{name} wants a count, got `{text}`")))
    }
}

/// Run a breathe verb, journalling a failure before passing it on.
fn breathe_journal<T>(
    ctx: &breathe::Context,
    what: &str,
    result: crate::error::Result<T>,
) -> Result<T, Failure> {
    result.map_err(|error| {
        note(
            Some(&ctx.data_root),
            "breathe",
            &format!("{what} error: {error}"),
        );
        error.into()
    })
}

/// `breathe init`.
fn breathe_init(args: &[String]) -> Result<(), Failure> {
    BreatheArgs::parse(args, "breathe init", &[], &[], 0)?;
    let ctx = breathe_context()?;
    let made = breathe_journal(&ctx, "init", breathe::init(&ctx.data_root))?;
    note(
        Some(&ctx.data_root),
        "breathe",
        &format!("init made={}", made.len()),
    );
    for path in made {
        println!("{}", path.display());
    }
    Ok(())
}

/// `breathe sources [--due] [--kind skim|deep] [--json]`.
fn breathe_sources(args: &[String]) -> Result<(), Failure> {
    let parsed = BreatheArgs::parse(
        args,
        "breathe sources",
        &["--kind"],
        &["--due", "--json"],
        0,
    )?;
    let kind = match parsed.flag("--kind") {
        Some(kind) => breathe::PassKind::parse(&kind)?,
        None => breathe::PassKind::Skim,
    };
    let due = (parsed.set("--due") || parsed.flag("--kind").is_some()).then_some(kind);
    let ctx = breathe_context()?;
    let sources = breathe::sources(&ctx, due)?;
    if parsed.set("--json") {
        println!("{}", breathe::sources_json(&sources));
    } else {
        for source in &sources {
            println!("{}", source.line());
        }
    }
    Ok(())
}

/// `breathe file --url U --repo R --title T --problem P --summary S [--source
/// NAME] [--bearing F] [--novelty F] [--spike]`.
fn breathe_file(args: &[String]) -> Result<(), Failure> {
    let what = "breathe file";
    let parsed = BreatheArgs::parse(
        args,
        what,
        &[
            "--bearing",
            "--novelty",
            "--problem",
            "--repo",
            "--source",
            "--summary",
            "--title",
            "--url",
        ],
        &["--spike"],
        0,
    )?;
    let filing = breathe::Filing {
        bearing: parsed.number("--bearing")?,
        novelty: parsed.number("--novelty")?,
        problem: parsed.required("--problem", what)?,
        repo: parsed.required("--repo", what)?,
        source: parsed.flag("--source"),
        spike: parsed.set("--spike"),
        summary: parsed.required("--summary", what)?,
        title: parsed.required("--title", what)?,
        url: parsed.required("--url", what)?,
    };
    if !breathe::valid_repo(&filing.repo) {
        return Err(Failure::Usage(format!(
            "--repo wants `*` or owner/name, got `{}`",
            filing.repo
        )));
    }
    let ctx = breathe_context()?;
    match breathe_journal(&ctx, "file", breathe::file(&ctx, &filing))? {
        breathe::Filed::New(path, event) => {
            note(
                Some(&ctx.data_root),
                "breathe",
                &format!(
                    "filed {} repo={} event={}",
                    path.display(),
                    filing.repo,
                    event.id
                ),
            );
            println!("{}", path.display());
        }
        breathe::Filed::Seen(path) => {
            note(
                Some(&ctx.data_root),
                "breathe",
                &format!("seen_again {}", path.display()),
            );
            println!("seen_again {}", path.display());
        }
    }
    Ok(())
}

/// `breathe list [--repo R] [--state S] [--json]`.
fn breathe_list(args: &[String]) -> Result<(), Failure> {
    let parsed = BreatheArgs::parse(args, "breathe list", &["--repo", "--state"], &["--json"], 0)?;
    let ctx = breathe_context()?;
    let breaths = breathe::list(
        &ctx.data_root,
        parsed.flag("--repo").as_deref(),
        parsed.flag("--state").as_deref(),
    )?;
    if parsed.set("--json") {
        println!("{}", breathe::list_json(&breaths));
    } else {
        for breath in &breaths {
            println!("{}", breath.line());
        }
    }
    Ok(())
}

/// `breathe spike|adopt|drop <file> [--reason TEXT]`.
fn breathe_transition(args: &[String], to: breathe::Transition) -> Result<(), Failure> {
    let what = format!("breathe {}", to.verb());
    let parsed = BreatheArgs::parse(args, &what, &["--reason"], &[], 1)?;
    let file = parsed
        .positionals
        .first()
        .cloned()
        .ok_or_else(|| Failure::Usage(format!("{what} needs a breath file")))?;
    let reason = parsed.flag("--reason");
    if to == breathe::Transition::Drop
        && reason.as_deref().is_none_or(|text| text.trim().is_empty())
    {
        return Err(Failure::Usage("breathe drop needs --reason".to_owned()));
    }
    let ctx = breathe_context()?;
    let (breath, event) = breathe_journal(
        &ctx,
        to.verb(),
        breathe::transition(&ctx, &file, to, reason.as_deref()),
    )?;
    note(
        Some(&ctx.data_root),
        "breathe",
        &format!(
            "{} {} event={}",
            breath.state,
            breath.file,
            event.as_ref().map_or(EMPTY, |event| event.id.as_str())
        ),
    );
    println!("{}  {}", breath.state, breath.file);
    Ok(())
}

/// `breathe pass --kind skim|deep [--dry-run]`.
fn breathe_pass(args: &[String]) -> Result<(), Failure> {
    let parsed = BreatheArgs::parse(args, "breathe pass", &["--kind"], &["--dry-run"], 0)?;
    let kind = breathe::PassKind::parse(&parsed.required("--kind", "breathe pass")?)?;
    let budget_usd = usd_env("SANDMAN_BREATH_BUDGET_USD", breathe::DEFAULT_BUDGET_USD)?;
    let dead_usd = usd_env(
        "SANDMAN_BREATH_DEAD_COST_USD",
        breathe::DEFAULT_DEAD_COST_USD,
    )?;
    let dry_run = parsed.set("--dry-run");
    let ctx = breathe_context()?;
    // nothing due, in flight and budget spent are waits (1); anything else is a fault (3)
    let composed = breathe_journal(
        &ctx,
        "pass",
        breathe::pass(&ctx, kind, budget_usd, dead_usd, dry_run),
    )
    .map_err(|failure| match failure {
        Failure::Error(error @ (Error::Refused { .. } | Error::LockHeld { .. })) => {
            Failure::Error(error)
        }
        Failure::Error(error) => Failure::Fault(error),
        other => other,
    })?;
    if dry_run {
        println!("{}", composed.text);
        return Ok(());
    }
    note(
        Some(&ctx.data_root),
        "breathe",
        &format!(
            "pass {} sources={} chars={}",
            composed.id,
            composed.sources.len(),
            composed.text.chars().count()
        ),
    );
    println!("{}", composed.id);
    Ok(())
}

/// A USD amount from `$name`, else `default`; anything but a non-negative
/// number is a usage failure.
fn usd_env(name: &str, default: f64) -> Result<f64, Failure> {
    match env::var(name).ok().filter(|text| !text.is_empty()) {
        None => Ok(default),
        Some(text) => text
            .parse::<f64>()
            .ok()
            .filter(|usd| usd.is_finite() && *usd >= 0.0)
            .ok_or_else(|| Failure::Usage(format!("${name} wants USD, got `{text}`"))),
    }
}

/// `breathe attach --id ID [--bg-id X] [--session-id S]`.
fn breathe_attach(args: &[String]) -> Result<(), Failure> {
    let parsed = BreatheArgs::parse(
        args,
        "breathe attach",
        &["--bg-id", "--id", "--session-id"],
        &[],
        0,
    )?;
    let id = parsed.required("--id", "breathe attach")?;
    let (bg_id, session_id) = (parsed.flag("--bg-id"), parsed.flag("--session-id"));
    let ctx = breathe_context()?;
    breathe_journal(
        &ctx,
        "attach",
        breathe::attach(&ctx, &id, bg_id.as_deref(), session_id.as_deref()),
    )?;
    note(
        Some(&ctx.data_root),
        "breathe",
        &format!(
            "attached pass={id} bg={} session={}",
            bg_id.as_deref().unwrap_or(EMPTY),
            session_id.as_deref().unwrap_or(EMPTY)
        ),
    );
    Ok(())
}

/// `breathe close --id ID --read N --filed M --dropped K [--cost F]
/// [--cursor NAME=VALUE]…`.
fn breathe_close(args: &[String]) -> Result<(), Failure> {
    let what = "breathe close";
    let parsed = BreatheArgs::parse(
        args,
        what,
        &[
            "--cost",
            "--cursor",
            "--dropped",
            "--filed",
            "--id",
            "--read",
        ],
        &[],
        0,
    )?;
    let id = parsed.required("--id", what)?;
    let cursors = parsed
        .values
        .iter()
        .filter(|(name, _)| name == "--cursor")
        .map(|(_, value)| {
            value
                .split_once('=')
                .filter(|(name, _)| !name.is_empty())
                .map(|(name, cursor)| (name.to_owned(), cursor.to_owned()))
                .ok_or_else(|| Failure::Usage(format!("--cursor wants NAME=VALUE, got `{value}`")))
        })
        .collect::<Result<Vec<_>, Failure>>()?;
    let closing = breathe::Closing {
        cost_usd: parsed.number("--cost")?.ok_or_else(|| {
            Failure::Usage(
                "breathe close needs --cost (the session's own cost, or 0 when it cannot read it)"
                    .to_owned(),
            )
        })?,
        cursors,
        dropped: parsed.count("--dropped", what)?,
        filed: parsed.count("--filed", what)?,
        read: parsed.count("--read", what)?,
    };
    let ctx = breathe_context()?;
    let event = breathe_journal(&ctx, "close", breathe::close(&ctx, &id, &closing))?;
    if closing.cost_usd <= 0.0 {
        note(
            Some(&ctx.data_root),
            "breathe",
            &format!("cost zero pass={id}: the session could not read its cost"),
        );
    }
    note(
        Some(&ctx.data_root),
        "breathe",
        &format!(
            "closed pass={id} read={} filed={} dropped={} event={}",
            closing.read, closing.filed, closing.dropped, event.id
        ),
    );
    println!("{}", event.id);
    Ok(())
}

/// `breathe reweight [--dry-run]`.
fn breathe_reweight(args: &[String]) -> Result<(), Failure> {
    let parsed = BreatheArgs::parse(args, "breathe reweight", &[], &["--dry-run"], 0)?;
    let ctx = breathe_context()?;
    let (changes, event) = breathe_journal(
        &ctx,
        "reweight",
        breathe::reweight(&ctx, parsed.set("--dry-run")),
    )?;
    if changes.is_empty() {
        println!("no change");
    }
    for change in &changes {
        println!("{}", change.line());
    }
    if let Some(event) = event {
        note(
            Some(&ctx.data_root),
            "breathe",
            &format!("reweighted changes={} event={}", changes.len(), event.id),
        );
    }
    Ok(())
}

/// `shift attach --shift ID [--bg-id X] [--session-id S]`.
fn shift_attach(args: &[String]) -> Result<(), Failure> {
    let (values, _) = shift_flags(
        args,
        "shift attach",
        &["--bg-id", "--session-id", "--shift"],
        &[],
    )?;
    let id = flag(&values, "--shift")
        .ok_or_else(|| Failure::Usage("shift attach needs --shift".to_owned()))?;
    let ctx = shift_context()?;
    let (bg_id, session_id) = (flag(&values, "--bg-id"), flag(&values, "--session-id"));
    shift::attach(&ctx, &id, bg_id.as_deref(), session_id.as_deref())?;
    note(
        Some(&ctx.data_root),
        "shift",
        &format!(
            "attached shift={id} bg={} session={}",
            bg_id.as_deref().unwrap_or(EMPTY),
            session_id.as_deref().unwrap_or(EMPTY)
        ),
    );
    Ok(())
}

/// `shift brief [--mind M] [--dry-run]`.
fn shift_brief(args: &[String]) -> Result<(), Failure> {
    let (values, set) = shift_flags(args, "shift brief", &["--mind"], &["--dry-run"])?;
    let ctx = shift_context()?;
    let budget = env::var("SANDMAN_SHIFT_BUDGET_USD")
        .ok()
        .filter(|budget| !budget.is_empty());
    let budget_usd = match budget {
        None => 15.0,
        Some(text) => text
            .parse::<f64>()
            .ok()
            .filter(|budget| budget.is_finite() && *budget >= 0.0)
            .ok_or_else(|| {
                Failure::Usage(format!("$SANDMAN_SHIFT_BUDGET_USD wants USD, got `{text}`"))
            })?,
    };
    let mind = flag(&values, "--mind")
        .or_else(|| {
            env::var("SANDMAN_SHIFT_MIND")
                .ok()
                .filter(|mind| !mind.is_empty())
        })
        .unwrap_or_else(|| "opus".to_owned());
    let options = shift::BriefOptions {
        budget_usd,
        dry_run: set.iter().any(|switch| switch == "--dry-run"),
        mind,
    };
    let briefed = shift::brief(&ctx, &options);
    let briefed = match briefed {
        Ok(briefed) => briefed,
        Err(error) => {
            note(
                Some(&ctx.data_root),
                "shift",
                &format!("brief error: {error}"),
            );
            return Err(match error {
                Error::Untrusted(_) => Failure::Fault(error),
                error => error.into(),
            });
        }
    };
    if options.dry_run {
        println!("{}", briefed.text);
        return Ok(());
    }
    note(
        Some(&ctx.data_root),
        "shift",
        &format!(
            "briefed shift={} repo={} chars={} trimmed={} event={}",
            briefed.shift,
            briefed.target.label,
            briefed.text.chars().count(),
            if briefed.trimmed.is_empty() {
                EMPTY.to_owned()
            } else {
                briefed.trimmed.join(",")
            },
            briefed
                .event
                .as_ref()
                .map_or(EMPTY, |event| event.id.as_str()),
        ),
    );
    println!("{}", briefed.shift);
    Ok(())
}

/// `shift clock [--check]`.
fn shift_clock(args: &[String]) -> Result<(), Failure> {
    let (_, set) = shift_flags(args, "shift clock", &[], &["--check"])?;
    let ctx = shift_context()?;
    let clock = shift::read_clock(&ctx).map_err(Failure::Fault)?;
    println!("{}", clock.line());
    if set.iter().any(|switch| switch == "--check")
        && let Some(refusal) = clock.refusal(ctx.min_window_seconds)
    {
        return Err(Error::refused(refusal).into());
    }
    Ok(())
}

/// `shift handoff --shift ID (--from-session | --abandoned [--reason TEXT] | --file PATH)`.
fn shift_handoff(args: &[String]) -> Result<(), Failure> {
    let (values, set) = shift_flags(
        args,
        "shift handoff",
        &["--file", "--reason", "--shift"],
        &["--abandoned", "--from-session", "--truncate"],
    )?;
    let reason = flag(&values, "--reason");
    let id = flag(&values, "--shift")
        .ok_or_else(|| Failure::Usage("shift handoff needs --shift".to_owned()))?;
    let file = flag(&values, "--file");
    let abandoned = set.iter().any(|switch| switch == "--abandoned");
    if reason.is_some() && !abandoned {
        return Err(Failure::Usage(
            "shift handoff takes --reason only with --abandoned".to_owned(),
        ));
    }
    let source = match (
        abandoned,
        set.iter().any(|switch| switch == "--from-session"),
        file,
    ) {
        (true, false, None) => {
            shift::Source::Abandoned(reason.filter(|reason| !reason.trim().is_empty()))
        }
        (false, true, None) => shift::Source::Session,
        (false, false, Some(file)) => shift::Source::File(PathBuf::from(file)),
        _ => {
            return Err(Failure::Usage(
                "shift handoff takes one of --from-session, --abandoned, --file PATH".to_owned(),
            ));
        }
    };
    let ctx = shift_context()?;
    let truncate = set.iter().any(|switch| switch == "--truncate");
    match shift::handoff(&ctx, &id, &source, truncate) {
        Ok(event) => {
            note(
                Some(&ctx.data_root),
                "shift",
                &format!("handed off shift={id} event={}", event.id),
            );
            println!("{}", event.id);
            Ok(())
        }
        Err(error) => {
            note(
                Some(&ctx.data_root),
                "shift",
                &format!("handoff shift={id} error: {error}"),
            );
            Err(error.into())
        }
    }
}

/// `shift pick [--path] [--shift ID | --untrusted]`.
fn shift_pick(args: &[String]) -> Result<(), Failure> {
    let (values, set) = shift_flags(args, "shift pick", &["--shift"], &["--path", "--untrusted"])?;
    let ctx = shift_context()?;
    let targets = match flag(&values, "--shift") {
        _ if set.iter().any(|switch| switch == "--untrusted") => shift::untrusted(&ctx),
        Some(id) => {
            let target = shift::picked(&ctx, &id)?;
            vec![(target.label, target.path)]
        }
        None => {
            let target = shift::pick(&ctx)?
                .ok_or_else(|| Error::not_found("repo to shift", "the rotation"))?;
            vec![(target.label, target.path)]
        }
    };
    for (label, path) in targets {
        if set.iter().any(|switch| switch == "--path") {
            println!("{}", path.display());
        } else {
            println!("{label}");
        }
    }
    Ok(())
}

/// `shift profile [--kind shift|breath]`.
fn shift_profile(args: &[String]) -> Result<(), Failure> {
    let (values, _) = shift_flags(args, "shift profile", &["--kind"], &[])?;
    let kind = flag(&values, "--kind").unwrap_or_else(|| "shift".to_owned());
    let profile = shift::profile(&kind).ok_or_else(|| {
        Failure::Usage(format!(
            "shift profile --kind wants one of {}, got `{kind}`",
            shift::PROFILE_KINDS.join(", ")
        ))
    })?;
    print!("{profile}");
    Ok(())
}

/// `shift status`.
fn shift_status(args: &[String]) -> Result<(), Failure> {
    shift_flags(args, "shift status", &[], &[])?;
    let ctx = shift_context()?;
    match shift::status(&ctx)? {
        Some(current) => println!("{current}"),
        None => println!("no shift open"),
    }
    println!("clock: {}", shift::clock(&ctx).line());
    Ok(())
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
    // writes what it decided to <root>/.trace; this one destroys every copy of a
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
