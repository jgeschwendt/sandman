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
use std::fs::OpenOptions;
use std::io::{self, Read};
use std::net::{SocketAddr, TcpListener};
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
    breathe, dream, forget, mcp, recall, reconcile, reflect, remember, report, score, shift, take,
    trail,
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
      archives, pointer. Never touches a bank. The id must be a whole uuid.

  breathe init
  breathe sources [--due] [--kind skim|deep] [--json]
  breathe file --url U --repo R --title T --problem P --summary S
               [--source NAME] [--bearing F] [--novelty F] [--spike]
  breathe list [--repo R] [--state new|spiked|adopted|dropped] [--json]
  breathe spike|adopt|drop <file> [--reason TEXT]
  breathe pass --kind skim|deep [--dry-run]
  breathe attach --id ID [--bg-id X] [--session-id S]
  breathe reaped --id ID
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
      nothing. attach records the pass's session; reaped stamps reaped: <now>
      once the runner has removed it and taken its transcript; close requires --cost (0
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

  report [--since T | --week yyyy-Www | --all] [--mark-read]
  report --hook          (SessionStart payload on stdin)
      The owner's digest: the shifts' report.md files started after the read
      cursor (<root>/.reports-read), newest first under a --- rule, headed by
      every one's Worth-your-eye bullets and the proposal: threads awaiting
      him; `nothing new.` when there are none. --since, --week and --all
      select by the shift's start and ignore the cursor; --mark-read moves it
      to the newest report printed. --hook answers with the hook envelope,
      capped at 6,000 characters (whole reports cut oldest first), and never
      moves the cursor; silent when nothing is unread and for the machine's
      own sessions — the open shift's session_id, a cwd that is its path, or
      a cwd under the data root.

  report serve [--bind 127.0.0.1:1705] [--requests N]
      The report as a loopback page (a non-loopback --bind exits 2), printing
      `listening on http://<address>`: GET / shows a night — the newest with
      a shift, which moves the read cursor to its newest report, or
      ?night=yyyy-mm-dd, a night keyed by the day it began on —
      its header, the proposal: threads awaiting the owner each with an
      approve/revise/decline form, Worth your eye, Landed, every shift's
      report, and the last 14 nights. POST /answer (key, answer, note) runs
      `thread reply <id> --as user --answer <answer> [--body <note>]`
      ($SANDMAN_THREAD_BIN, else ~/.local/bin/thread), appends a decision
      event and redirects to /; 404 for an unknown key, 502 when thread
      fails. The Rotation form checks every rotation trunk not off; POST
      /rotation (on, repeated) leaves off the rest, writes
      <root>/shifts/rotation.toml, appends a guard event and redirects to /. A Host other than the bound address, or a foreign Origin, is
      403. --requests answers N requests, then exits.

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
                --file PATH) [--cost USD] [--truncate] [--cloud]
  shift open --cloud [--mind M]
  shift pick [--path] [--shift ID | --untrusted]
  shift profile [--kind shift|breath]
  shift reaped --shift ID
  shift rotation [--off REPO]... [--on REPO]...
  shift status
      The perpetual machine's unit of work, bounded by the five-hour window.
      brief picks the target, fills every {{slot}} of prompts/shift.md
      ($SANDMAN_PROMPTS, else the sandman trunk's prompts/) under a
      24,000-character cap (the Context slots fenced as data, cut in a fixed
      order, the template's own text never — exit 1 when it still will not
      fit), writes <root>/shifts/<id>/{brief.md,meta.json},
      takes shifts/current.json, refreshes breath/problems.md, appends a
      shift.start event whose refs fingerprint every slot, and prints the id;
      --dry-run prints the brief and writes nothing. The theme is that of
      the America/New_York weekday the night began on: Sun Hunt, Mon
      Redesign, Tue Refactor, Wed Measure, Thu Harden, Fri Ship, Sat Rest —
      the Sabbath, which fills
      prompts/sabbath.md instead and works jgeschwendt/sandman's trunk, once
      (exit 1, `sabbath kept: <id> handed off at <time>`, after the night's); a
      repo whose docs/plan.md names nothing next takes its line from the
      Sabbath's score/<week>-targets.md. pick is the rotation — the least
      recently shifted trunk of the grove's $SANDMAN_SHIFT_OWNERS repos
      (default jgeschwendt), Saturday shifts aside — never ~, ~/.routines or
      the data root — ties broken by a failing stele check, open threads and
      breaths; it skips the open shift's repo, $SANDMAN_SHIFT_SKIP (colon-separated slugs),
      a repo the owner switched off (<root>/shifts/rotation.toml's off = [...];
      `skipped: <repo> (off)` on stderr), one he froze (`freeze`, `frozen` or
      `on ice` in its targets line,
      `on ice` in his answer on a proposal: thread; `skipped: <repo> (frozen)` on
      stderr) and every trunk Claude Code has not trusted ($SANDMAN_CLAUDE_JSON, else
      ~/.claude.json) — exit 1 listing them when no other is left, and brief
      exits 3; --untrusted lists those trunks; --shift prints the target a
      shift was briefed on. rotation prints every trunk · on/off · last
      shifted · frozen, after --off/--on (repeatable; a repo outside the
      rotation exits 2) rewrite the off list. Shifts run by night — $SANDMAN_SHIFT_NIGHT, New
      York hours HH-HH (default 21-07) — and a night carries the theme of the
      day it began on. clock reads the window
      file ($CLAUDE_WINDOW_FILE, else $XDG_STATE_HOME/claude/window.json, else
      ~/.local/state/claude/window.json); --check exits 1 by day (`day: the
      night opens at <instant>`), under $SANDMAN_SHIFT_MIN_WINDOW_MIN (default
      60) minutes of night left (`night ends in <m>m`) or of window left, or
      at 90% used; 0 when the file is missing or stale (written over 30 min
      ago); clock and --check exit 3 when the file is
      there but cannot be read, is not a five_hour object with numeric
      resets_at and non-negative used_percentage, is stamped at more than
      5 min ahead, or resets more than 6 h ahead. handoff finalizes a shift —
      copies --file in, accepts the handoff.md the session wrote, or records
      --abandoned (--reason TEXT, only beside --abandoned, heads the handoff
      and is the event's rationale as `Abandoned: <reason>`) — appends
      shift.handoff (--cost, the session's own total, as its cost_usd; 0
      journals a cost zero note), copies current.json's bg_id and session_id
      into meta.json and releases current.json; a shift already handed off
      exits 1, and so does a handoff over 2,000 characters
      unless --truncate keeps its head and notes the cut (--abandoned always
      truncates); every handoff but --abandoned refuses (exit 1) without a
      shifts/<id>/report.md carrying the report's five headings in order, under
      6,000 characters. attach records the session the runner
      started in current.json; reaped stamps reaped: <now> into meta.json once
      the runner has removed the session and taken its transcript. status
      prints current.json and the clock.
      handoff_by is the window's reset less 15 min (now + 5 h less 15 min
      when unknown), never past the night's end.
      profile prints the Claude Code settings a shift or breath session runs
      under (profiles/session.json, compiled in; one profile for both kinds).
      $SANDMAN_NOW (ISO-8601) fixes the instant; $SANDMAN_SHIFT_MIND (default
      opus) and $SANDMAN_SHIFT_BUDGET_USD (default 15) fill the brief.
      open --cloud is the cloud routine's brief: in the state clone
      ($SANDMAN_ROOT) it pulls --rebase, takes the committed current.json as
      the lock (exit 1 `busy: <id> open since <ts>` under 70 min old, else
      abandoned `cloud: stale lock`), picks among the clones in $SANDMAN_GROVE
      (<dir>/<repo>, the state and $SANDMAN_THREADS clones aside, every clone
      trusted, approved proposals first), composes as brief does with
      handoff_by now + 50 min and no night gate, adds cloud: true and
      session_url ($CLAUDE_CODE_REMOTE_SESSION_ID) to current.json and
      meta.json, commits, pushes (one pull --rebase on a rejection, then exit
      1 `state push rejected`) and prints the brief; templates default to
      $SANDMAN_GROVE/sandman/prompts. handoff --cloud commits and pushes the
      same way. On a Redesign night pick puts approved proposals first too.

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
      machine commit carries. Kinds: breath change correction decay decision
      guard promote reconcile revoke shift.handoff shift.start — append
      refuses decision (exit 2), the owner's answer, which report serve writes.

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
        Err(Failure::Help) => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Err(Failure::Usage(message)) => {
            eprintln!("sandman: {message}\n\n{USAGE}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

/// Why a verb stopped short of running.
enum Failure {
    /// The verb ran and failed.
    Error(Error),
    /// A runner's fault: the clock's window file, the pass's template or
    /// sources.
    Fault(Error),
    /// `-h`/`--help`: the usage on stdout, exit 0.
    Help,
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
        "-h" | "--help" | "help" => Err(Failure::Help),
        "breathe" => breathe_verb(rest),
        "dream" => dream_verb(rest),
        "forget" => forget_verb(rest),
        "mcp" => mcp_verb(rest),
        "recall" => recall_verb(rest),
        "reconcile" => reconcile_verb(rest),
        "reflect" => reflect_verb(rest),
        "remember" => remember_verb(rest),
        "report" => report_verb(rest),
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
    let parsed = Args::parse(
        args,
        "remember",
        &["--bank", "--cwd", "--description", "--name", "--type"],
        &[],
        1,
    )?;
    let request = remember::Remember {
        bank: parsed.flag("--bank"),
        body: parsed.positional("a body")?,
        cwd: parsed.flag("--cwd").map(PathBuf::from),
        description: parsed.flag("--description"),
        kind: parsed
            .flag("--type")
            .map(|value| {
                MemoryType::from_str(&value)
                    .map_err(|_| Failure::Usage(format!("unknown --type `{value}`")))
            })
            .transpose()?,
        name: parsed.flag("--name"),
        session_id: env::var(SESSION_ENV).ok(),
    };

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
    crate::atomic::ensure_parent(&log)?;
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
    // The flag is documentation: dream always routes what is queued.
    Args::parse(args, "dream", &[], &["--now"], 0)?;
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
    match Args::parse(args, "reflect", &["--day"], &[], 0)?.flag("--day") {
        Some(value) => reflect_one_day(
            calendar_day(&value)
                .ok_or_else(|| Failure::Usage(format!("--day wants yyyy-mm-dd, got `{value}`")))?,
        ),
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
    let parsed = Args::parse(args, "recall", &["--cwd"], &["--hook"], 0)?;
    let from_hook = parsed.set("--hook");
    let journal = paths::data_root().ok();
    let result = recall_run(
        journal.as_deref(),
        from_hook,
        parsed.flag("--cwd").map(PathBuf::from),
    );
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
    let trimmed = composed.trimmed.listed();
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
    Args::parse(args, "mcp", &[], &[], 0)?;
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
        "-h" | "--help" => Err(Failure::Help),
        "append" => trail_append(rest),
        "log" => trail_log(rest),
        "revoke" => trail_revoke(rest),
        "show" => trail_show(rest),
        "today" => {
            Args::parse(rest, "trail today", &[], &[], 0)?;
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
    let parsed = Args::parse(
        args,
        "trail append",
        &[
            "--branch",
            "--cost",
            "--json",
            "--kind",
            "--model",
            "--parent",
            "--rationale",
            "--ref",
            "--repo",
            "--sha",
            "--shift",
        ],
        &[],
        0,
    )?;
    let mut draft = match parsed.flag("--json") {
        Some(json) => trail::Draft::from_json(&json)
            .map_err(|error| Failure::Usage(format!("--json: {error}")))?,
        None => trail::Draft::default(),
    };
    // Flags land after `--json`, so a flag overrides the object's key.
    for (flag, value) in parsed.values {
        match flag.as_str() {
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
            "--shift" => draft.shift = Some(value),
            _ => {}
        }
    }
    if draft.kind.is_empty() {
        return Err(Failure::Usage("trail append needs --kind".to_owned()));
    }
    if !trail::is_kind(&draft.kind) {
        return Err(Failure::Usage(format!("unknown kind `{}`", draft.kind)));
    }
    if draft.kind == "decision" {
        return Err(Failure::Usage(
            "kind `decision` is the owner's answer to a proposal, written by `report serve` only"
                .to_owned(),
        ));
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
    let parsed = Args::parse(
        args,
        "trail log",
        &["--kind", "--repo", "--shift", "--since"],
        &["--json"],
        0,
    )?;
    let since = parsed.flag("--since");
    if let Some(value) = since
        .as_deref()
        .filter(|value| calendar_day(value).is_none())
    {
        return Err(Failure::Usage(format!(
            "--since wants yyyy-mm-dd, got `{value}`"
        )));
    }
    let entries = trail::read(&paths::data_root()?, since.as_deref())?;
    let matches = |wanted: &str, have: Option<&str>| {
        parsed
            .flag(wanted)
            .is_none_or(|wanted| have == Some(&wanted))
    };
    for entry in &entries {
        let draft = &entry.event.draft;
        if !matches("--kind", Some(&draft.kind))
            || !matches("--shift", draft.shift.as_deref())
            || !matches("--repo", draft.repo.as_deref())
        {
            continue;
        }
        if parsed.set("--json") {
            println!("{}", entry.raw);
        } else {
            println!("{}", entry.event.summary());
        }
    }
    Ok(())
}

/// `trail show <id>`.
fn trail_show(args: &[String]) -> Result<(), Failure> {
    let id = Args::parse(args, "trail show", &[], &[], 1)?.positional("an id")?;
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
    let parsed = Args::parse(args, "trail revoke", &["--reason"], &[], 1)?;
    let id = parsed.positional("an id")?;
    let reason = parsed
        .flag("--reason")
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
    let parsed = Args::parse(args, "trail trailers", &["--event", "--shift"], &[], 0)?;
    let (Some(shift), Some(event)) = (parsed.flag("--shift"), parsed.flag("--event")) else {
        return Err(Failure::Usage(
            "trail trailers needs --shift and --event".to_owned(),
        ));
    };
    println!("{}", trail::trailers(&shift, &event));
    Ok(())
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
    let parsed = Args::parse(args, "reconcile", &[], &["--dry-run"], 1)?;
    let (id, dry_run) = (
        parsed.positional("an event or shift id")?,
        parsed.set("--dry-run"),
    );
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

/// `report [--since T | --week W | --all] [--mark-read] | --hook | serve …`.
fn report_verb(args: &[String]) -> Result<(), Failure> {
    if args.first().is_some_and(|arg| arg == "serve") {
        return report_serve(&args[1..]);
    }
    let parsed = Args::parse(
        args,
        "report",
        &["--since", "--week"],
        &["--all", "--hook", "--mark-read"],
        0,
    )?;
    let since = parsed
        .flag("--since")
        .map(|text| {
            Timestamp::parse_iso8601(&text)
                .ok_or_else(|| Failure::Usage(format!("--since wants RFC 3339, got `{text}`")))
        })
        .transpose()?;
    let week = parsed
        .flag("--week")
        .map(|text| {
            score::Week::parse(&text)
                .ok_or_else(|| Failure::Usage(format!("--week wants yyyy-Www, got `{text}`")))
        })
        .transpose()?;
    let select = match (since, week, parsed.set("--all")) {
        (None, None, false) => report::Select::Unread,
        (Some(at), None, false) => report::Select::Since(at),
        (None, Some(week), false) => report::Select::Week(week),
        (None, None, true) => report::Select::All,
        _ => {
            return Err(Failure::Usage(
                "report takes one of --since, --week, --all".to_owned(),
            ));
        }
    };
    let hook = parsed.set("--hook");
    if hook && (select != report::Select::Unread || parsed.set("--mark-read")) {
        return Err(Failure::Usage(
            "report --hook takes no other flag".to_owned(),
        ));
    }
    let data_root = paths::data_root()?;
    let threads = paths::threads_dir(&paths::home()?);
    if hook {
        let start = hook::session_start(&stdin()?)?;
        if report::own(&data_root, &start) {
            note(
                Some(&data_root),
                "report",
                "hook declined the machine's own session",
            );
            return Ok(());
        }
        // never the cursor: a shift's session can start before `attach` names
        // it, and only the page the owner opens marks reports read
        let digest = report::digest(&data_root, &threads, select, report::REPORT_CHARS);
        if digest.newest.is_some() {
            println!("{}", hook::session_start_reply(&digest.text));
        }
        return Ok(());
    }
    let digest = report::digest(&data_root, &threads, select, usize::MAX);
    print!("{}", digest.text);
    if parsed.set("--mark-read")
        && let Some(newest) = digest.newest
    {
        report::mark_read(&data_root, newest)?;
    }
    Ok(())
}

/// `report serve [--bind ADDR] [--requests N]`.
fn report_serve(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "report serve", &["--bind", "--requests"], &[], 0)?;
    let bind = parsed
        .flag("--bind")
        .unwrap_or_else(|| report::serve::BIND.to_owned());
    let address = bind
        .parse::<SocketAddr>()
        .ok()
        .filter(|address| address.ip().is_loopback())
        .ok_or_else(|| Failure::Usage(format!("--bind wants a loopback address, got `{bind}`")))?;
    let requests = parsed
        .flag("--requests")
        .map(|text| {
            text.parse()
                .map_err(|_| Failure::Usage(format!("--requests wants a count, got `{text}`")))
        })
        .transpose()?;
    let ctx = shift_context()?;
    let server = report::serve::Server {
        pinned: env::var_os("SANDMAN_NOW").is_some_and(|now| !now.is_empty()),
        requests,
        thread: env::var_os("SANDMAN_THREAD_BIN")
            .filter(|bin| !bin.is_empty())
            .map_or(ctx.home.join(".local/bin/thread"), PathBuf::from),
        ctx,
    };
    let listener = TcpListener::bind(address).map_err(|source| Error::io(&bind, source))?;
    report::serve::serve(&listener, &server);
    Ok(())
}

/// `score [--week YYYY-Www] [--json]`.
fn score_verb(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "score", &["--week"], &["--json"], 0)?;
    let ctx = shift_context()?;
    let week = match parsed.flag("--week") {
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
    if parsed.set("--json") {
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
        return Err(Failure::Help);
    }
    match sub.as_str() {
        "-h" | "--help" => Err(Failure::Help),
        "attach" => shift_attach(rest),
        "brief" => shift_brief(rest),
        "clock" => shift_clock(rest),
        "handoff" => shift_handoff(rest),
        "open" => shift_open(rest),
        "pick" => shift_pick(rest),
        "profile" => shift_profile(rest),
        "reaped" => shift_reaped(rest),
        "rotation" => shift_rotation(rest),
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
        cloud: None,
        data_root: paths::data_root()?,
        now,
        min_window_seconds: knob(
            "SANDMAN_SHIFT_MIN_WINDOW_MIN",
            "minutes 0–300",
            shift::DEFAULT_MIN_WINDOW_MINUTES,
            |text| {
                text.parse::<i64>()
                    .ok()
                    .filter(|minutes| (0..=300).contains(minutes))
            },
        )? * 60,
        night: night()?,
        owners: list("SANDMAN_SHIFT_OWNERS", &["jgeschwendt"]),
        prompts: paths::prompts_dir()?,
        roots: grove_roots()?,
        skip: list("SANDMAN_SHIFT_SKIP", &[]),
        threads: paths::threads_dir(&home),
        window: paths::claude_window_file()?,
        home,
    })
}

/// `$SANDMAN_SHIFT_NIGHT`, else 21–07.
fn night() -> Result<shift::Night, Failure> {
    knob(
        "SANDMAN_SHIFT_NIGHT",
        "HH-HH, two different New York hours 0–23",
        shift::DEFAULT_NIGHT,
        shift::Night::parse,
    )
}

/// `$name` read through `parse`, else `default`; a value `parse` refuses is the
/// operator's to fix, `want` saying what it takes.
fn knob<T>(
    name: &str,
    want: &str,
    default: T,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<T, Failure> {
    match env::var(name) {
        Ok(text) if !text.is_empty() => parse(&text)
            .ok_or_else(|| Failure::Usage(format!("${name} wants {want}, got `{text}`"))),
        _ => Ok(default),
    }
}

/// `breathe <init|sources|file|list|spike|adopt|drop|pass|attach|close|reweight> …`.
fn breathe_verb(args: &[String]) -> Result<(), Failure> {
    let Some(sub) = args.first() else {
        return Err(Failure::Usage("breathe needs a subcommand".to_owned()));
    };
    let rest = &args[1..];
    if rest.iter().any(|arg| arg == "-h" || arg == "--help") {
        return Err(Failure::Help);
    }
    match sub.as_str() {
        "-h" | "--help" => Err(Failure::Help),
        "adopt" => breathe_transition(rest, breathe::Transition::Adopt),
        "attach" => breathe_attach(rest),
        "close" => breathe_close(rest),
        "drop" => breathe_transition(rest, breathe::Transition::Drop),
        "file" => breathe_file(rest),
        "init" => breathe_init(rest),
        "list" => breathe_list(rest),
        "pass" => breathe_pass(rest),
        "reaped" => breathe_reaped(rest),
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
        None => Timestamp::now().map_err(Failure::from),
    }
}

/// A verb's parsed command line: `(flag, value)` pairs in order (a flag may
/// repeat), the switches set, and the positionals.
struct Args {
    positionals: Vec<String>,
    switches: Vec<String>,
    values: Vec<(String, String)>,
    /// The verb, for the refusals that name it.
    what: String,
}

impl Args {
    /// Parse `args`: each named flag takes a value, each switch none, at most
    /// `positionals` bare arguments are accepted, and `-h` stops at the usage.
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
            what: what.to_owned(),
        };
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            if flags.contains(&arg.as_str()) {
                let value = args
                    .next()
                    .ok_or_else(|| Failure::Usage(format!("{arg} needs a value")))?;
                parsed.values.push((arg.clone(), value.clone()));
            } else if switches.contains(&arg.as_str()) {
                parsed.switches.push(arg.clone());
            } else if arg == "-h" || arg == "--help" {
                return Err(Failure::Help);
            } else if arg.starts_with("--") {
                return Err(Failure::Usage(format!("unknown option `{arg}`")));
            } else if parsed.positionals.len() < positionals {
                parsed.positionals.push(arg.clone());
            } else {
                return Err(Failure::Usage(match positionals {
                    0 => format!("{what} takes no arguments, got `{arg}`"),
                    _ => format!("{what} takes at most {positionals} argument(s), got `{arg}`"),
                }));
            }
        }
        Ok(parsed)
    }

    /// Every value given for `flag`, in order.
    fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> {
        self.values
            .iter()
            .filter(move |(flag, _)| flag == name)
            .map(|(_, value)| value.as_str())
    }

    /// The last value given for `flag`.
    fn flag(&self, name: &str) -> Option<String> {
        self.all(name).last().map(ToOwned::to_owned)
    }

    /// The first positional, `noun`, which must be there.
    fn positional(&self, noun: &str) -> Result<String, Failure> {
        self.positionals
            .first()
            .cloned()
            .ok_or_else(|| Failure::Usage(format!("{} needs {noun}", self.what)))
    }

    /// The value for `flag`, which must be there.
    fn required(&self, name: &str) -> Result<String, Failure> {
        self.flag(name)
            .ok_or_else(|| Failure::Usage(format!("{} needs {name}", self.what)))
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
    fn count(&self, name: &str) -> Result<u64, Failure> {
        let text = self.required(name)?;
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
    Args::parse(args, "breathe init", &[], &[], 0)?;
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
    let parsed = Args::parse(
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
    let parsed = Args::parse(
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
        problem: parsed.required("--problem")?,
        repo: parsed.required("--repo")?,
        source: parsed.flag("--source"),
        spike: parsed.set("--spike"),
        summary: parsed.required("--summary")?,
        title: parsed.required("--title")?,
        url: parsed.required("--url")?,
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
    let parsed = Args::parse(args, "breathe list", &["--repo", "--state"], &["--json"], 0)?;
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
    let parsed = Args::parse(args, &what, &["--reason"], &[], 1)?;
    let file = parsed.positional("a breath file")?;
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
    let parsed = Args::parse(args, "breathe pass", &["--kind"], &["--dry-run"], 0)?;
    let kind = breathe::PassKind::parse(&parsed.required("--kind")?)?;
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
        Failure::Error(error)
            if !matches!(error, Error::Refused { .. } | Error::LockHeld { .. }) =>
        {
            Failure::Fault(error)
        }
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

/// `breathe reaped --id ID`.
fn breathe_reaped(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "breathe reaped", &["--id"], &[], 0)?;
    let id = parsed.required("--id")?;
    let ctx = breathe_context()?;
    breathe_journal(&ctx, "reaped", breathe::reaped(&ctx, &id))
}

/// `breathe attach --id ID [--bg-id X] [--session-id S]`.
fn breathe_attach(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(
        args,
        "breathe attach",
        &["--bg-id", "--id", "--session-id"],
        &[],
        0,
    )?;
    let id = parsed.required("--id")?;
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
    let parsed = Args::parse(
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
    let id = parsed.required("--id")?;
    let cursors = parsed
        .all("--cursor")
        .map(|value| {
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
        dropped: parsed.count("--dropped")?,
        filed: parsed.count("--filed")?,
        read: parsed.count("--read")?,
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
    let parsed = Args::parse(args, "breathe reweight", &[], &["--dry-run"], 0)?;
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

/// `shift reaped --shift ID`.
fn shift_reaped(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "shift reaped", &["--shift"], &[], 0)?;
    let id = parsed.required("--shift")?;
    Ok(shift::reaped(&shift_context()?, &id)?)
}

/// `shift attach --shift ID [--bg-id X] [--session-id S]`.
fn shift_attach(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(
        args,
        "shift attach",
        &["--bg-id", "--session-id", "--shift"],
        &[],
        0,
    )?;
    let id = parsed.required("--shift")?;
    let ctx = shift_context()?;
    let (bg_id, session_id) = (parsed.flag("--bg-id"), parsed.flag("--session-id"));
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

/// What a brief is composed for: `--mind`, else `$SANDMAN_SHIFT_MIND`, else
/// opus, and `$SANDMAN_SHIFT_BUDGET_USD`, else 15.
fn brief_options(parsed: &Args, dry_run: bool) -> Result<shift::BriefOptions, Failure> {
    let budget_usd = usd_env("SANDMAN_SHIFT_BUDGET_USD", 15.0)?;
    let mind = parsed
        .flag("--mind")
        .or_else(|| {
            env::var("SANDMAN_SHIFT_MIND")
                .ok()
                .filter(|mind| !mind.is_empty())
        })
        .unwrap_or_else(|| "opus".to_owned());
    Ok(shift::BriefOptions {
        budget_usd,
        dry_run,
        mind,
    })
}

/// `shift brief [--mind M] [--dry-run]`.
fn shift_brief(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "shift brief", &["--mind"], &["--dry-run"], 0)?;
    let ctx = shift_context()?;
    let options = brief_options(&parsed, parsed.set("--dry-run"))?;
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

/// `shift open --cloud [--mind M]`.
fn shift_open(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "shift open", &["--mind"], &["--cloud"], 0)?;
    let clones = paths::cloud_clones().filter(|_| parsed.set("--cloud"));
    let Some(clones) = clones else {
        return Err(Failure::Usage(
            "shift open runs in the cloud: --cloud, with $SANDMAN_GROVE naming the clones"
                .to_owned(),
        ));
    };
    let ctx = shift::Context {
        cloud: Some(shift::Cloud {
            clones,
            session_url: env::var("CLAUDE_CODE_REMOTE_SESSION_ID")
                .ok()
                .filter(|id| !id.is_empty())
                .map(|id| format!("https://claude.ai/code/{id}")),
        }),
        ..shift_context()?
    };
    let briefed = shift::open(&ctx, &brief_options(&parsed, false)?).inspect_err(|error| {
        note(
            Some(&ctx.data_root),
            "shift",
            &format!("open error: {error}"),
        );
    })?;
    note(
        Some(&ctx.data_root),
        "shift",
        &format!(
            "opened shift={} repo={} cloud",
            briefed.shift, briefed.target.label
        ),
    );
    println!("{}", briefed.text);
    Ok(())
}

/// `shift clock [--check]`.
fn shift_clock(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "shift clock", &[], &["--check"], 0)?;
    let ctx = shift_context()?;
    let clock = shift::read_clock(&ctx).map_err(Failure::Fault)?;
    println!("{}", clock.line());
    if parsed.set("--check")
        && let Some(refusal) = clock.refusal(ctx.min_window_seconds)
    {
        return Err(Error::refused(refusal).into());
    }
    Ok(())
}

/// `shift handoff --shift ID (--from-session | --abandoned [--reason TEXT] | --file PATH)
/// [--cost USD] [--truncate]`.
fn shift_handoff(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(
        args,
        "shift handoff",
        &["--cost", "--file", "--reason", "--shift"],
        &["--abandoned", "--cloud", "--from-session", "--truncate"],
        0,
    )?;
    let cost = parsed
        .flag("--cost")
        .map(|value| {
            value
                .parse::<f64>()
                .ok()
                .filter(|cost| cost.is_finite() && *cost >= 0.0)
                .ok_or_else(|| Failure::Usage(format!("--cost wants USD, got `{value}`")))
        })
        .transpose()?;
    let reason = parsed.flag("--reason");
    let id = parsed.required("--shift")?;
    let file = parsed.flag("--file");
    let abandoned = parsed.set("--abandoned");
    if reason.is_some() && !abandoned {
        return Err(Failure::Usage(
            "shift handoff takes --reason only with --abandoned".to_owned(),
        ));
    }
    let source = match (abandoned, parsed.set("--from-session"), file) {
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
    let truncate = parsed.set("--truncate");
    match shift::handoff(&ctx, &id, &source, truncate, cost) {
        Ok(event) => {
            if cost.is_some_and(|cost| cost <= 0.0) {
                note(
                    Some(&ctx.data_root),
                    "shift",
                    &format!("cost zero shift={id}: the session could not read its cost"),
                );
            }
            note(
                Some(&ctx.data_root),
                "shift",
                &format!("handed off shift={id} event={}", event.id),
            );
            if parsed.set("--cloud") {
                shift::sync(&ctx.data_root, &format!("shift({id}): handoff"))?;
            }
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
    let parsed = Args::parse(
        args,
        "shift pick",
        &["--shift"],
        &["--path", "--untrusted"],
        0,
    )?;
    let ctx = shift_context()?;
    let targets = match parsed.flag("--shift") {
        _ if parsed.set("--untrusted") => shift::untrusted(&ctx),
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
        if parsed.set("--path") {
            println!("{}", path.display());
        } else {
            println!("{label}");
        }
    }
    Ok(())
}

/// `shift profile [--kind shift|breath]`.
fn shift_profile(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "shift profile", &["--kind"], &[], 0)?;
    let kind = parsed.flag("--kind").unwrap_or_else(|| "shift".to_owned());
    let profile = shift::profile(&kind).ok_or_else(|| {
        Failure::Usage(format!(
            "shift profile --kind wants one of {}, got `{kind}`",
            shift::PROFILE_KINDS.join(", ")
        ))
    })?;
    print!("{profile}");
    Ok(())
}

/// `shift rotation [--off REPO]… [--on REPO]…`.
fn shift_rotation(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "shift rotation", &["--off", "--on"], &[], 0)?;
    let ctx = shift_context()?;
    let mut rows = shift::rotation(&ctx)?;
    for (flag, on) in [("--off", false), ("--on", true)] {
        for repo in parsed.all(flag) {
            rows.iter_mut()
                .find(|row| row.label == repo)
                .ok_or_else(|| Failure::Usage(format!("{flag}: `{repo}` is not in the rotation")))?
                .on = on;
        }
    }
    let off = rows
        .iter()
        .filter(|row| !row.on)
        .map(|row| row.label.clone());
    if !parsed.values.is_empty() {
        shift::set_off(&ctx.data_root, off)?;
    }
    for row in rows {
        println!(
            "{} · {} · {}{}",
            row.label,
            if row.on { "on" } else { "off" },
            row.last
                .map_or_else(|| "never shifted".to_owned(), Timestamp::iso8601),
            if row.frozen { " · frozen" } else { "" }
        );
    }
    Ok(())
}

/// `shift status`.
fn shift_status(args: &[String]) -> Result<(), Failure> {
    Args::parse(args, "shift status", &[], &[], 0)?;
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
    Args::parse(args, "version", &[], &[], 0)?;
    println!("{VERSION}");
    Ok(())
}

/// `forget <session-id>`.
fn forget_verb(args: &[String]) -> Result<(), Failure> {
    let session_id = Args::parse(args, "forget", &[], &[], 1)?.positional("a session id")?;

    // Deliberately unjournaled, and it must stay that way. Every other verb
    // writes what it decided to <root>/.trace; this one destroys every copy of a
    // session, and a line naming the session it destroyed would be precisely
    // the pointer the verb exists to remove.
    for path in forget::forget(&paths::data_root()?, &paths::claude_root()?, &session_id)? {
        println!("{}", path.display());
    }
    Ok(())
}

/// The whole of stdin — a hook payload.
fn stdin() -> Result<String, Failure> {
    let mut payload = String::new();
    io::stdin()
        .read_to_string(&mut payload)
        .map_err(|source| Error::io("<stdin>", source))?;
    Ok(payload)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_long_bank_line_is_cut_with_a_count_for_the_tail() {
        let files = ["a".repeat(1_500), "b".repeat(400), "c".repeat(400)];
        let listed = super::bank_memories(&files);
        assert_eq!(listed, format!("{},{}…+1", files[0], files[1]));
    }
}
