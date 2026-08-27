//! The dispatcher: `sandman <verb> [args]`.
//!
//! Hand-rolled parsing, std only. Three exit codes and nothing else — 0 for a
//! verb that ran, 1 for a failure (one line on stderr), 2 for anything the
//! operator has to fix in the command line itself, unbuilt verbs included.
//!
//! This is the only place that resolves the real roots (`crate::paths`); every
//! verb below it takes them as arguments.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::str::FromStr;

use crate::error::Error;
use crate::hook;
use crate::memory::MemoryType;
use crate::paths;
use crate::time::Timestamp;
use crate::verbs::{dream, forget, recall, reflect, remember, take};

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
      background job under ~/.claude/jobs still names the session: the daemon
      retires idle workers, and a worker's exit is not the conversation's end.
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

    let outcome = remember::remember(&paths::data_root()?, request)?;
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
            return Ok(());
        }
        // A dream mind's own ending is not a session to keep: archiving it
        // feeds the very queue that spawned it, and at depth the next take
        // spawns another dream — the recursion that flooded the machine with
        // claude processes on 2026-08-19. Recall already goes silent under
        // this variable; take must too.
        if env::var(PIPELINE_ENV).as_deref() == Ok("1") {
            return Ok(());
        }
        // SessionEnd is the proof the session is over — the hook fires the
        // instant the transcript's last line lands, which is exactly what the
        // by-hand live-window heuristic refuses. Hook mode implies force.
        force = true;
        let ending = hook::session_end(&stdin()?)?;
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
            return Ok(());
        }
        // A payload with no session is a session with nothing to take.
        let Some(session_id) = ending.session_id else {
            return Ok(());
        };
        // Nor is a retirement. The daemon retires a background worker that has
        // sat idle and done, and the retired process's exit fires SessionEnd
        // with an ordinary reason — but a job directory still naming the
        // session says the conversation is resumable and will be resumed. The
        // guard is the hook's alone: a session named by hand is taken on the
        // operator's word, job or no job.
        if take::names_live_job(&paths::claude_root()?, &session_id) {
            return Ok(());
        }
        session_id
    } else {
        session_id.ok_or_else(|| Failure::Usage("take needs a session id".to_owned()))?
    };

    let outcome = match take::take(
        &paths::data_root()?,
        &paths::claude_root()?,
        &session_id,
        force,
    ) {
        Ok(outcome) => outcome,
        // `forget` destroys every copy before the session ends, so the hook
        // that follows it finds nothing — the designed sequence, not a fault.
        // Asked for by hand, a missing transcript is still an error.
        Err(Error::NotFound { .. }) if from_hook => return Ok(()),
        Err(error) => return Err(Failure::Error(error)),
    };
    println!("{}", outcome.archived.display());
    if outcome.dream_due() {
        let root = paths::data_root()?;
        // A held lock means a run is already draining the queue — starting
        // another would only fork a process to watch it yield.
        if dream::lock_held(&root) {
            eprintln!(
                "queue at {} — a dream is already running",
                outcome.queue_depth
            );
            return Ok(());
        }
        match spawn_dream(&root) {
            Ok(log) => eprintln!(
                "queue at {} — dreaming in the background, logging to {}",
                outcome.queue_depth,
                log.display()
            ),
            Err(error) => eprintln!(
                "queue at {} — dream could not be spawned: {error}",
                outcome.queue_depth
            ),
        }
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
    let outcome = reflect::reflect(
        &paths::data_root()?,
        Timestamp::now()?,
        &reflect::Options::from_env(),
    )?;
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
    if env::var(PIPELINE_ENV).as_deref() == Ok("1") {
        return Ok(());
    }
    if from_hook && cwd.is_none() {
        cwd = hook::session_start(&stdin()?)?.cwd;
    }
    let cwd = match cwd {
        Some(cwd) => cwd,
        None => env::current_dir().map_err(|source| Error::io(".", source))?,
    };

    let text = recall::recall(
        &paths::data_root()?,
        &paths::home()?,
        &cwd,
        Timestamp::now()?,
    );
    if text.is_empty() {
        return Ok(());
    }
    if from_hook {
        println!("{}", hook::session_start_reply(&text));
    } else {
        println!("{text}");
    }
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
