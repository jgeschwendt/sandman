//! The runner seam — one `claude -p` process per mind.
//!
//! A mind is a model asked one question in its own OS process: no
//! orchestrating session, no workflow, no shared state. Independence is what
//! makes the dream's 2-of-3 consensus worth anything, and a process boundary
//! is the cheapest way to buy it.
//!
//! Every run is memory-blind (`CLAUDE_MEMORY_PIPELINE=1`, which is where
//! `recall` exits), reads nothing from stdin, and is killed at the timeout —
//! a mind that does not answer in time abstains, and the pass carries on with
//! the ones that did.
//!
//! A mind's own transcript is either kept or never written: [`Keep`] pins the
//! session id and relocates the file the run leaves behind; without it the run
//! is not persisted at all. Nothing in between — an unclaimed transcript is
//! junk in `~/.claude/projects`, one session per mind per pointer.

use std::collections::hash_map::DefaultHasher;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt::{self, Write as _};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::json::{self, Value};

/// The binary the minds are run through, overridable for tests.
pub const CLAUDE_BIN_ENV: &str = "SANDMAN_CLAUDE_BIN";
/// The binary when `$SANDMAN_CLAUDE_BIN` says nothing.
pub const CLAUDE_BIN_DEFAULT: &str = "claude";
/// Set on every mind. A memory-blind run recalls nothing, so extraction can
/// never echo the memories it is about to propose.
pub const PIPELINE_ENV: &str = "CLAUDE_MEMORY_PIPELINE";
/// The model reflect's bank upkeep runs on — one call, not the dream trio.
pub const UPKEEP_ENV: &str = "SANDMAN_MIND_UPKEEP";
/// The upkeep model when `$SANDMAN_MIND_UPKEEP` says nothing.
pub const UPKEEP_MODEL_DEFAULT: &str = "claude-opus-5";
/// How long a mind may take before it is killed and counted as abstaining.
pub const TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
/// How often the runner looks in on a child it is waiting for.
const POLL: Duration = Duration::from_millis(20);

/// A mind's tier.
///
/// The variant order is the strength order, weakest first — it is what
/// `derive(Ord)` gives the consensus pass when it picks whose wording carries
/// a group. This is the one list in the crate that is deliberately not
/// alpha-sorted.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Tier {
    /// The fastest of the three.
    Sonnet,
    /// The middle tier; also reflect's upkeep model.
    Opus,
    /// The strongest — its draft carries a group it agrees with.
    Fable,
}

impl Tier {
    /// The three tiers, weakest first.
    pub const ALL: [Self; 3] = [Self::Sonnet, Self::Opus, Self::Fable];

    /// The tier's name, as it appears in logs and in a memory's `source:`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fable => "fable",
            Self::Opus => "opus",
            Self::Sonnet => "sonnet",
        }
    }

    /// The environment variable that moves this tier's model.
    #[must_use]
    pub const fn model_env(self) -> &'static str {
        match self {
            Self::Fable => "SANDMAN_MIND_FABLE",
            Self::Opus => "SANDMAN_MIND_OPUS",
            Self::Sonnet => "SANDMAN_MIND_SONNET",
        }
    }

    /// The model the tier runs on when its variable says nothing.
    #[must_use]
    pub const fn model_default(self) -> &'static str {
        match self {
            Self::Fable => "claude-fable-5",
            Self::Opus => "claude-opus-5",
            Self::Sonnet => "claude-sonnet-5",
        }
    }

    /// The model id to invoke, environment override included.
    #[must_use]
    pub fn model(self) -> String {
        override_from(self.model_env()).unwrap_or_else(|| self.model_default().to_owned())
    }
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One mind: a tier and the model id it resolves to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mind {
    /// The model id passed to `--model`.
    pub model: String,
    /// Which tier this is — the consensus pass's tie-breaker.
    pub tier: Tier,
}

impl Mind {
    /// The mind for `tier`, model resolved from the environment.
    #[must_use]
    pub fn new(tier: Tier) -> Self {
        Self {
            model: tier.model(),
            tier,
        }
    }
}

/// The dream's three minds, weakest first.
#[must_use]
pub fn trio() -> Vec<Mind> {
    Tier::ALL.into_iter().map(Mind::new).collect()
}

/// Reflect's upkeep mind — one opus call, pinned apart from the trio so the
/// dream's models can move without moving upkeep's.
#[must_use]
pub fn upkeep() -> Mind {
    Mind {
        model: override_from(UPKEEP_ENV).unwrap_or_else(|| UPKEEP_MODEL_DEFAULT.to_owned()),
        tier: Tier::Opus,
    }
}

/// The `claude` binary to run, environment override included.
#[must_use]
pub fn claude_bin() -> OsString {
    env::var_os(CLAUDE_BIN_ENV)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| OsString::from(CLAUDE_BIN_DEFAULT))
}

/// A non-empty `$name`.
fn override_from(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

/// Why a mind said nothing usable. Every variant is an abstention: the pass
/// carries on without this mind rather than failing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Abstained {
    /// The wrapper said the run failed, or the process exited non-zero.
    Failed(String),
    /// The process could not be started at all.
    Spawn(String),
    /// It ran past the timeout and was killed.
    Timeout,
    /// The process answered, but not in the shape a reply has.
    Unreadable(String),
}

impl Abstained {
    /// The one word a log line carries.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Failed(_) => "failed",
            Self::Spawn(_) => "spawn",
            Self::Timeout => "timeout",
            Self::Unreadable(_) => "unreadable",
        }
    }
}

impl fmt::Display for Abstained {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed(detail) | Self::Spawn(detail) | Self::Unreadable(detail) => {
                write!(f, "{}: {detail}", self.as_str())
            }
            Self::Timeout => f.write_str("timeout"),
        }
    }
}

/// Where a mind's own transcript is kept.
///
/// Claude Code writes a persisted run to `<projects>/<slug of the working
/// directory>/<session id>.jsonl`, so the run is pinned from both ends: the
/// working directory decides which subdirectory it lands in, the session id
/// decides the file name, and the file is moved to [`Self::into`] once the
/// child is done.
#[derive(Clone, Debug)]
pub struct Keep {
    /// The directory the child runs in.
    pub cwd: PathBuf,
    /// The directory the finished transcript is moved to.
    pub into: PathBuf,
    /// Claude Code's projects directory, where the transcript is written.
    pub projects: PathBuf,
}

/// One question for one mind.
#[derive(Clone, Debug)]
pub struct Ask {
    /// The `claude` binary to run.
    pub binary: OsString,
    /// Where to keep the run's own transcript. `None` discards it.
    pub keep: Option<Keep>,
    /// The model id.
    pub model: String,
    /// The whole prompt — passed as an argument, never on stdin.
    pub prompt: String,
    /// How long the mind may take before it is killed.
    pub timeout: Duration,
}

/// Ask one mind and read its reply text.
///
/// The child is `claude -p <prompt> --model <model> --output-format json`
/// with `CLAUDE_MEMORY_PIPELINE=1` and no stdin. What it does with its own
/// transcript is [`Ask::keep`]'s business, and there are exactly two answers:
///
/// - `Some` — the run is pinned to a generated session id and to
///   [`Keep::cwd`], and the transcript Claude Code wrote is moved to
///   [`Keep::into`] as `<session id>.jsonl` so it can be evaluated later. The
///   move is best effort and runs on every outcome — an answer, a non-zero
///   exit, a timeout kill — because all three leave a transcript behind. It
///   never changes what this function returns.
/// - `None` — `--no-session-persistence`: nothing is written at all. A mind's
///   run is not a conversation to keep unless somebody claimed it, and an
///   unclaimed one is junk in `~/.claude/projects`, one session per mind per
///   pointer. The pipeline env guard on `take` only stops it being re-queued,
///   not written.
///
/// Every failure is an [`Abstained`], never an error: a dream survives a mind
/// that does not answer.
pub fn ask(request: &Ask) -> Result<String, Abstained> {
    let session = request.keep.as_ref().and_then(prepare);
    let mut child = spawn(request, session.as_deref())
        .map_err(|source| Abstained::Spawn(format!("{}: {source}", display(&request.binary))))?;
    let answer = run(request, &mut child);
    if let (Some(keep), Some(session)) = (request.keep.as_ref(), session.as_deref()) {
        relocate(keep, session);
    }
    answer
}

/// The session id a kept run is pinned to, or `None` to fall back to a run
/// that persists nothing. A working directory that cannot be created is not
/// worth failing an ask over.
fn prepare(keep: &Keep) -> Option<String> {
    fs::create_dir_all(&keep.cwd).ok()?;
    Some(session_id())
}

/// Move a finished mind's transcript out of Claude Code's projects tree.
///
/// Silent throughout: a transcript that was never written, or will not move,
/// costs the pass nothing.
fn relocate(keep: &Keep, session: &str) {
    let name = format!("{session}.jsonl");
    let Some(source) = transcript(&keep.projects, &name) else {
        return;
    };
    if fs::create_dir_all(&keep.into).is_err() {
        return;
    }
    let target = keep.into.join(&name);
    let moved = fs::rename(&source, &target).is_ok()
        // A projects tree on another volume cannot be renamed across.
        || (fs::copy(&source, &target).is_ok() && fs::remove_file(&source).is_ok());
    if moved {
        // The slug directory the move emptied; `remove_dir` refuses a
        // non-empty one, so a directory with anyone else's sessions stays.
        if let Some(parent) = source.parent() {
            let _ = fs::remove_dir(parent);
        }
    }
}

/// `<projects>/<any one project>/<name>`, if it is there. Which project slug
/// Claude Code chose is its own business, so every immediate subdirectory is
/// a candidate.
fn transcript(projects: &Path, name: &str) -> Option<PathBuf> {
    fs::read_dir(projects).ok()?.flatten().find_map(|entry| {
        let path = entry.path().join(name);
        path.is_file().then_some(path)
    })
}

/// `ETXTBSY` on Linux: exec of a just-written executable fails while any
/// concurrently forked child still holds the writer's fd across its own
/// fork→exec window. Only freshly written binaries are exposed — the test
/// stubs here, never an installed `claude` — and the window is transient, so
/// the spawn is retried briefly (the same remedy cargo applies to its own
/// just-built artifacts).
fn spawn(request: &Ask, session: Option<&str>) -> io::Result<std::process::Child> {
    const ETXTBSY: i32 = 26;
    let mut tries = 0;
    loop {
        match command(request, session).spawn() {
            Err(error) if error.raw_os_error() == Some(ETXTBSY) && tries < 40 => {
                tries += 1;
                thread::sleep(Duration::from_millis(5));
            }
            other => return other,
        }
    }
}

/// The child, built fresh per spawn attempt.
fn command(request: &Ask, session: Option<&str>) -> Command {
    let mut command = Command::new(&request.binary);
    command
        .arg("-p")
        .arg(&request.prompt)
        .arg("--model")
        .arg(&request.model)
        .arg("--output-format")
        .arg("json");
    match (session, request.keep.as_ref()) {
        (Some(session), Some(keep)) => {
            command
                .arg("--session-id")
                .arg(session)
                .current_dir(&keep.cwd);
        }
        _ => {
            command.arg("--no-session-persistence");
        }
    }
    command
        .env(PIPELINE_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// A fresh uuid-v4 for `--session-id`, which is the name the transcript is
/// then found under. Format only — nothing here needs it to be unguessable,
/// only unique against the other minds in the pass.
fn session_id() -> String {
    uuid(entropy())
}

/// 16 bytes as a uuid-v4: the version and variant nibbles are stamped in, the
/// rest is rendered 8-4-4-4-12 in lowercase hex.
fn uuid(mut bytes: [u8; 16]) -> String {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let mut out = String::with_capacity(36);
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// 16 bytes for a session id, `/dev/urandom` first.
fn entropy() -> [u8; 16] {
    let mut bytes = [0_u8; 16];
    if let Ok(mut source) = fs::File::open("/dev/urandom")
        && source.read_exact(&mut bytes).is_ok()
    {
        return bytes;
    }
    seeded()
}

/// 16 bytes derived from the clock, the process and a counter — enough to
/// keep the minds of one pass apart when `/dev/urandom` cannot be read.
fn seeded() -> [u8; 16] {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let seed = (
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos()),
        u128::from(std::process::id()),
        u128::from(SERIAL.fetch_add(1, Ordering::Relaxed)),
    );
    let mut bytes = [0_u8; 16];
    for (half, chunk) in bytes.chunks_mut(8).enumerate() {
        let mut hasher = DefaultHasher::new();
        seed.hash(&mut hasher);
        half.hash(&mut hasher);
        chunk.copy_from_slice(&hasher.finish().to_le_bytes());
    }
    bytes
}

/// Drive a spawned mind to its reply, timeout included.
fn run(request: &Ask, child: &mut std::process::Child) -> Result<String, Abstained> {
    // Both pipes are drained on their own threads: a child that fills one
    // while the runner polls `try_wait` would otherwise deadlock instead of
    // timing out.
    let out = child.stdout.take();
    let err = child.stderr.take();
    let out_reader = thread::spawn(move || drain(out));
    let err_reader = thread::spawn(move || drain(err));

    let deadline = Instant::now() + request.timeout;
    let mut status = None;
    loop {
        match child.try_wait() {
            Ok(Some(exit)) => {
                status = Some(exit);
                break;
            }
            Ok(None) => {}
            Err(source) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Abstained::Spawn(source.to_string()));
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        thread::sleep(POLL);
    }

    let Some(status) = status else {
        // The readers are deliberately not joined here: killing the process
        // sandman started does not kill a grandchild it left holding the
        // pipe, so a join would wait out exactly the run the timeout just
        // refused to wait out. The threads end when the pipe finally closes.
        return Err(Abstained::Timeout);
    };
    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    if !status.success() {
        return Err(Abstained::Failed(format!(
            "exit {}: {}",
            status.code().unwrap_or(-1),
            crate::transcript::one_line(&stderr, 200)
        )));
    }
    reply(&stdout)
}

/// Read the reply text out of the CLI's JSON wrapper.
///
/// Observed 2026-08-12 from `claude -p "reply with exactly OK" --model
/// claude-sonnet-5 --output-format json`: one JSON object, keys in no
/// particular order, carrying `"type":"result"`, `"subtype":"success"`,
/// `"is_error":false`, `"result":"OK"` and a pile of usage/cost fields. The
/// reply text is `result`; `is_error` is the wrapper's own verdict.
///
/// The scan falls back to the last line that parses as such an object, so a
/// binary that prefixes a warning line still answers.
fn reply(stdout: &str) -> Result<String, Abstained> {
    let text = stdout.trim();
    if text.is_empty() {
        return Err(Abstained::Unreadable("no output".to_owned()));
    }
    let value = json::parse(text).ok().or_else(|| {
        text.lines()
            .rev()
            .filter_map(|line| json::parse(line.trim()).ok())
            .find(|value| value.get("result").is_some())
    });
    let Some(value) = value else {
        return Err(Abstained::Unreadable(one_line_200(text)));
    };
    if value.get("is_error").and_then(Value::as_bool) == Some(true) {
        return Err(Abstained::Failed(
            value
                .get("result")
                .and_then(Value::as_str)
                .map_or_else(|| "is_error".to_owned(), one_line_200),
        ));
    }
    value
        .get("result")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| Abstained::Unreadable("no `result` in the wrapper".to_owned()))
}

/// One line of `text`, cut for a log.
fn one_line_200(text: &str) -> String {
    crate::transcript::one_line(text, 200)
}

/// Read a child pipe to the end, lossily — a mind's bytes are not sandman's
/// to validate.
fn drain<R: Read>(pipe: Option<R>) -> String {
    let Some(mut pipe) = pipe else {
        return String::new();
    };
    let mut bytes = Vec::new();
    let _ = pipe.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// A binary path for an error message.
fn display(binary: &OsStr) -> String {
    binary.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::{
        Abstained, Ask, Keep, TIMEOUT_DEFAULT, Tier, ask, reply, session_id, trio, upkeep,
    };
    use crate::testutil::TempDir;
    use std::path::PathBuf;
    use std::time::Duration;

    #[test]
    fn the_tiers_order_weakest_to_strongest() {
        assert!(Tier::Sonnet < Tier::Opus);
        assert!(Tier::Opus < Tier::Fable);
        assert_eq!(
            Tier::ALL.map(Tier::as_str),
            ["sonnet", "opus", "fable"],
            "the log's mind list is this order"
        );
    }

    #[test]
    fn the_trio_and_the_upkeep_mind_carry_their_default_models() {
        // The overrides are process-wide environment, which the unit tests
        // cannot set (`unsafe_code = "forbid"`); `tests/cli.rs` drives them
        // through a child process instead.
        let minds = trio();
        assert_eq!(minds.len(), 3);
        for (mind, tier) in minds.iter().zip(Tier::ALL) {
            assert_eq!(mind.tier, tier);
            if std::env::var_os(tier.model_env()).is_none() {
                assert_eq!(mind.model, tier.model_default());
            }
        }
        if std::env::var_os(super::UPKEEP_ENV).is_none() {
            assert_eq!(upkeep().model, super::UPKEEP_MODEL_DEFAULT);
            assert_eq!(upkeep().tier, Tier::Opus);
        }
    }

    #[test]
    fn the_wrapper_yields_its_result_text() {
        let wrapper = r#"{"is_error":false,"session_id":"s","subtype":"success","result":"OK","type":"result"}"#;
        assert_eq!(reply(wrapper), Ok("OK".to_owned()));
        assert_eq!(reply(&format!("\n{wrapper}\n")), Ok("OK".to_owned()));
        // A warning line ahead of the wrapper costs nothing.
        assert_eq!(
            reply(&format!("warning: something\n{wrapper}")),
            Ok("OK".to_owned())
        );
    }

    #[test]
    fn a_wrapper_without_an_answer_is_an_abstention() {
        assert_eq!(
            reply(""),
            Err(Abstained::Unreadable("no output".to_owned()))
        );
        assert!(matches!(reply("not json"), Err(Abstained::Unreadable(_))));
        assert!(matches!(
            reply(r#"{"type":"result","subtype":"success"}"#),
            Err(Abstained::Unreadable(_))
        ));
        assert!(matches!(
            reply(r#"{"is_error":true,"result":"refused","type":"result"}"#),
            Err(Abstained::Failed(_))
        ));
    }

    #[test]
    fn a_binary_that_is_not_there_is_an_abstention() {
        let request = Ask {
            binary: "/nonexistent/sandman-test-claude".into(),
            keep: None,
            model: "claude-sonnet-5".to_owned(),
            prompt: "hello".to_owned(),
            timeout: TIMEOUT_DEFAULT,
        };
        assert!(matches!(ask(&request), Err(Abstained::Spawn(_))));
    }

    #[cfg(unix)]
    #[test]
    fn a_mind_that_runs_past_the_timeout_is_killed_and_abstains() {
        let temp = TempDir::new("mind-timeout");
        let stub = crate::testutil::stub_script(temp.path(), "slow", "sleep 30\n");
        let request = Ask {
            binary: stub.into(),
            keep: None,
            model: "claude-sonnet-5".to_owned(),
            prompt: "hello".to_owned(),
            timeout: Duration::from_millis(150),
        };
        let started = std::time::Instant::now();
        assert_eq!(ask(&request), Err(Abstained::Timeout));
        assert!(started.elapsed() < Duration::from_secs(10), "it waited");
    }

    #[cfg(unix)]
    #[test]
    fn a_stub_mind_answers_through_the_wrapper() {
        let temp = TempDir::new("mind-stub");
        let stub = crate::testutil::stub_script(
            temp.path(),
            "echo",
            "printf '%s' '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"hello from '\"$4\"'\"}'\n",
        );
        let request = Ask {
            binary: stub.into(),
            keep: None,
            model: "claude-fable-5".to_owned(),
            prompt: "hello".to_owned(),
            timeout: Duration::from_secs(30),
        };
        assert_eq!(ask(&request), Ok("hello from claude-fable-5".to_owned()));
    }

    #[cfg(unix)]
    #[test]
    fn a_mind_that_exits_non_zero_abstains() {
        let temp = TempDir::new("mind-failure");
        let stub = crate::testutil::stub_script(temp.path(), "boom", "echo broke >&2\nexit 3\n");
        let request = Ask {
            binary: stub.into(),
            keep: None,
            model: "claude-opus-5".to_owned(),
            prompt: "hello".to_owned(),
            timeout: Duration::from_secs(30),
        };
        match ask(&request) {
            Err(Abstained::Failed(detail)) => assert!(detail.contains("exit 3"), "{detail}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn a_session_id_is_a_uuid_v4() {
        for id in [session_id(), super::uuid(super::seeded())] {
            assert_eq!(id.len(), 36, "{id}");
            let groups: Vec<&str> = id.split('-').collect();
            assert_eq!(
                groups.iter().map(|group| group.len()).collect::<Vec<_>>(),
                [8, 4, 4, 4, 12],
                "{id}"
            );
            assert!(
                groups.iter().all(|group| group
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())),
                "{id}"
            );
            // Version 4, variant 10xx.
            assert_eq!(&groups[2][0..1], "4", "{id}");
            assert!(matches!(&groups[3][0..1], "8" | "9" | "a" | "b"), "{id}");
        }
        assert_ne!(session_id(), session_id());
    }

    /// A stub that writes the transcript Claude Code would have written, into
    /// the project subdirectory it would have chosen.
    #[cfg(unix)]
    fn keeping_stub(dir: &std::path::Path, projects: &std::path::Path, tail: &str) -> PathBuf {
        crate::testutil::stub_script(
            dir,
            "keeper",
            &format!(
                concat!(
                    "sid=\"\"\n",
                    "while [ $# -gt 0 ]; do case \"$1\" in --session-id) sid=\"$2\"; shift 2;; *) shift;; esac; done\n",
                    "mkdir -p \"{projects}/-a-slug\"\n",
                    "pwd > \"{projects}/../ran-in\"\n",
                    "printf 'the mind said things\\n' > \"{projects}/-a-slug/$sid.jsonl\"\n",
                    "{tail}",
                ),
                projects = projects.display(),
                tail = tail,
            ),
        )
    }

    #[cfg(unix)]
    #[test]
    fn a_kept_minds_transcript_is_moved_out_of_the_projects_tree() {
        let temp = TempDir::new("mind-keep");
        let projects = temp.path().join("projects");
        let keep = Keep {
            cwd: temp.path().join("dream-cwd"),
            into: temp.path().join("kept").join("-Users-you"),
            projects: projects.clone(),
        };
        let stub = keeping_stub(
            temp.path(),
            &projects,
            "printf '%s' '{\"type\":\"result\",\"is_error\":false,\"result\":\"OK\"}'\n",
        );
        let request = Ask {
            binary: stub.into(),
            keep: Some(keep.clone()),
            model: "claude-sonnet-5".to_owned(),
            prompt: "hello".to_owned(),
            timeout: Duration::from_secs(30),
        };

        assert_eq!(ask(&request), Ok("OK".to_owned()));
        // The child ran where the keep said, so the slug is sandman's to know.
        let ran_in = std::fs::read_to_string(temp.path().join("ran-in")).expect("cwd");
        assert!(
            std::path::Path::new(ran_in.trim()).ends_with("dream-cwd"),
            "{ran_in}"
        );
        // Exactly one transcript, under `into`, and nothing left behind.
        let kept: Vec<PathBuf> = std::fs::read_dir(&keep.into)
            .expect("kept dir")
            .map(|entry| entry.expect("entry").path())
            .collect();
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(
            kept[0].extension().and_then(std::ffi::OsStr::to_str),
            Some("jsonl")
        );
        assert!(
            !projects.join("-a-slug").exists(),
            "the move empties the slug directory, and the empty shell goes too"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_kept_mind_that_fails_still_leaves_its_transcript() {
        let temp = TempDir::new("mind-keep-failure");
        let projects = temp.path().join("projects");
        let keep = Keep {
            cwd: temp.path().join("dream-cwd"),
            into: temp.path().join("kept").join("orphans"),
            projects: projects.clone(),
        };
        let stub = keeping_stub(temp.path(), &projects, "exit 3\n");
        let request = Ask {
            binary: stub.into(),
            keep: Some(keep.clone()),
            model: "claude-opus-5".to_owned(),
            prompt: "hello".to_owned(),
            timeout: Duration::from_secs(30),
        };

        assert!(matches!(ask(&request), Err(Abstained::Failed(_))));
        assert_eq!(
            std::fs::read_dir(&keep.into).expect("kept dir").count(),
            1,
            "a failed run still wrote a transcript"
        );
    }
}
