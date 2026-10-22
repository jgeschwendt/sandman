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

mod breathe;
mod recall;
mod report;
mod rest;
mod shift;
mod take;
mod trail;

use std::env;
use std::io::{self, Read};
use std::path::Path;
use std::process::ExitCode;

use crate::error::Error;
use crate::grove;
use crate::journal;
use crate::paths;
use crate::time::Timestamp;
use crate::verbs::{self, reflect};

use breathe::breathe_verb;
use recall::recall_verb;
use report::{report_verb, score_verb};
use rest::{dream_verb, forget_verb, mcp_verb, reflect_verb, remember_verb, version_verb};
use shift::shift_verb;
use take::take_verb;
use trail::{reconcile_verb, trail_verb};

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
      archives, the dream minds' kept transcripts, pointer. Never touches a bank. The id must be a whole uuid.

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
      appends a breath event; it lands on a pass marked dead, its --cost
      replacing the charge, and refuses one already closed.
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
      <root>/.dream/<claude project>/<session-id>/<mind session-id>.jsonl.

  reflect [--day <yyyy-mm-dd>]
      The 24 h pass: the voyage entry for the day that ended (one
      $SANDMAN_MIND_LOG call, skipped when the day's sources are unchanged),
      the log index, the pointer sweep (dreamt and older than 72 h), and one
      gated opus upkeep call per grown bank ($SANDMAN_MIND_UPKEEP). A bank
      that fails is logged and passed; the pass exits 1 once the rest has
      run. --day renders that day's entry only — no drain, no sweep, no
      upkeep.

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
      and so does a trailered merge: the machine merges by rebase only,
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

  report plist
      The launchd agent launchd/dev.gigantic.sandman.report.plist, which keeps
      report serve up, rendered on stdout: {{HOME}} spelled out as $HOME
      (launchd expands no ~), and $SANDMAN_SHIFT_NIGHT added to its
      environment when set. Install: sandman report plist >
      ~/Library/LaunchAgents/dev.gigantic.sandman.report.plist.

  report serve [--bind 127.0.0.1:1705] [--requests N]
      The report as a loopback page (a non-loopback --bind exits 2), printing
      `listening on http://<address>`: GET / shows a night — the newest with
      a shift, which moves the read cursor to its newest report, or
      ?night=yyyy-mm-dd, a night keyed by the day it began on —
      its header, the proposal: threads awaiting the owner each with an
      approve/revise/decline form, Worth your eye, Landed, every shift's
      report, and the last 14 nights. POST /answer (key, answer, note) runs
      `docket reply <id> --as user --answer <answer> [--body <note>]`
      ($SANDMAN_DOCKET_BIN, else ~/.local/bin/docket), appends a decision
      event and redirects to /; 404 for an unknown key, 502 when thread
      fails. The Rotation form checks every rotation trunk not off; POST
      /rotation (on, repeated) leaves off the rest — an off repo whose
      trunk is not found now is listed, unchecked — writes
      <root>/shifts/rotation.toml, appends a guard event and redirects to /. A Host other than the bound address, or a foreign Origin, is
      403. --requests answers N requests, then exits.

  score [--week yyyy-Www] [--json]
      The week's numbers from the trail, shifts/*/meta.json and report.md,
      the breath store, ~/.docket ($SANDMAN_DOCKET) and the shift and
      breathe-* runs.jsonl under ~/.routines/.state ($SANDMAN_ROUTINES_STATE):
      repo coverage, each repo's newest report Numbers line, shift outcomes,
      events by kind/repo/model, cost, recurring corrections, fresh context,
      breaths, open threads, routine ticks. The
      week is ISO, its days New York's, the current one by default
      ($SANDMAN_NOW pins it). Rewrites <root>/score/<yyyy>-W<ww>.md —
      frontmatter, the worst three, a table per section — and prints its
      path; --json prints the metrics object instead. No model, no git.

  shift attach --shift ID [--bg-id X] [--session-id S]
  shift brief [--mind M] [--dry-run]
  shift clock [--check]
  shift handoff --shift ID (--from-session | --abandoned [--reason TEXT] |
                --file PATH) [--cost USD] [--truncate]
  shift land --shift ID --branch NAME
  shift pick [--path] [--shift ID | --untrusted]
  shift pr --shift ID (--number N --url U | --auto BOOL | --red SHA |
           --merged | --closed)
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
      repo whose docs/index.html #next names nothing takes its line from the
      Sabbath's score/<week>-targets.md. pick is the rotation — the least
      recently shifted trunk of the grove's $SANDMAN_SHIFT_OWNERS repos
      (default jgeschwendt), Saturday shifts aside — never ~, ~/.routines or
      the data root — ties broken by a failing stele check (mise x -C <trunk>
      -- stele check), open threads and breaths, none of them for a repo whose
      last shift began under 6 h ago and lasted under 15 min (`skipped
      promotion: <repo> (cooled: last shift <m>m)` on stderr); it skips the open shift's repo, $SANDMAN_SHIFT_SKIP (colon-separated slugs),
      a repo the owner switched off (<root>/shifts/rotation.toml's off = [...];
      `skipped: <repo> (off)` on stderr), one he froze (`freeze`, `frozen` or
      `on ice` in its targets line,
      `on ice` in his answer on a proposal: thread; `skipped: <repo> (frozen)` on
      stderr) and every trunk Claude Code has not trusted ($SANDMAN_CLAUDE_JSON, else
      ~/.claude.json) — exit 1 listing them when no other is left, and brief
      exits 3; --untrusted lists those trunks; --shift prints the target a
      shift was briefed on. rotation prints every trunk · on/off · last
      shifted · frozen, and each off repo whose trunk is not found as
      <repo> · off · not found, after --off/--on (repeatable; a repo in
      neither exits 2) rewrite the off list. Shifts run by night — $SANDMAN_SHIFT_NIGHT, New
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
      exits 1 (releasing a current.json that still names it, after appending
      the shift.handoff it lacks from meta.json), and so does a
      handoff over 2,000 characters
      unless --truncate keeps its head and notes the cut (--abandoned always
      truncates); every handoff but --abandoned refuses (exit 1) without a
      shifts/<id>/report.md carrying the report's five headings in order, under
      6,000 characters. attach records the session the runner
      started in current.json; reaped stamps reaped: <now> into meta.json once
      the runner has removed the session and taken its transcript. land
      records the open shift's gated branch as meta.json's landing ({at,
      base, branch, head}) once it contains the local origin/main and every
      commit it adds carries Shift: and Event: trailers (exit 1 naming the
      first that does not, or for an unknown branch, an unrebased one, an
      empty one, or a handed-off shift), and stamps superseded_by on the
      unmerged landings of earlier shifts those trailers name; trunk is never
      pushed — the runner pushes the branch and opens the pull request after
      the handoff. pr is the runner's record of that pull request in
      meta.json's pr — --number/--url opens it (opened), --auto, --red,
      --merged and --closed mark it; the brief's prs.open lists the
      target's open ones. status
      prints current.json and the clock.
      handoff_by is the window's reset less 15 min (now + 5 h less 15 min
      when unknown), never past the night's end.
      profile prints the Claude Code settings a shift or breath session runs
      under (profiles/session.json, compiled in; one profile for both kinds).
      $SANDMAN_NOW (ISO-8601) fixes the instant; $SANDMAN_SHIFT_MIND (default
      opus) and $SANDMAN_SHIFT_BUDGET_USD (default 15) fill the brief.
      On a Redesign night pick puts approved proposals first.

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

/// Everything the shift verbs read from the environment, resolved once.
fn shift_context() -> Result<verbs::shift::Context, Failure> {
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
    Ok(verbs::shift::Context {
        claude_json: paths::claude_json()?,
        data_root: paths::data_root()?,
        docket: paths::docket_dir(&home),
        now,
        min_window_seconds: knob(
            "SANDMAN_SHIFT_MIN_WINDOW_MIN",
            "minutes 0–300",
            verbs::shift::DEFAULT_MIN_WINDOW_MINUTES,
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
        window: paths::claude_window_file()?,
        home,
    })
}

/// `$SANDMAN_SHIFT_NIGHT`, else 21–07.
fn night() -> Result<verbs::shift::Night, Failure> {
    knob(
        "SANDMAN_SHIFT_NIGHT",
        "HH-HH, two different New York hours 0–23",
        verbs::shift::DEFAULT_NIGHT,
        verbs::shift::Night::parse,
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

/// The whole of stdin — a hook payload.
fn stdin() -> Result<String, Failure> {
    let mut payload = String::new();
    io::stdin()
        .read_to_string(&mut payload)
        .map_err(|source| Error::io("<stdin>", source))?;
    Ok(payload)
}
