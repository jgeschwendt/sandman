//! `sandman`, rewritten from scratch: `mark` appends a session's start or end, or a guard, to the
//! day's ledger file, `take` archives a marked session at its end, `chart` prints what the data
//! root holds as one JSON object, `check` holds every note to the memory cycle's invariants,
//! `recall` scores the memories against an agent's cue and `show` prints one, over the memories,
//! the knowledge pages and the traces of both, `rethink` records a verdict on one, `defer`, `due`, `done` and `drop` keep work put off
//! for later, `mcp` serves the agent's verbs as MCP tools,
//! `serve` is the daemon (reconcile, settle, encode, consolidate, reflect), `status` asks it for its
//! health, `setup` installs its launchd agent, `forget` erases one session from every store,
//! `version` prints the build.

use serde_json::{Map, Value, json};
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use signal_hook::low_level::signal_name;
use std::cell::Cell;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsStr;
use std::fs::{self, DirBuilder, File, OpenOptions, TryLockError};
use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
use std::iter;
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt as _;
use std::panic;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, ExitStatus, Stdio, id};
use std::str::FromStr;
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Phrases of a memory claiming its own importance, the reference's `CLAIM` lowercased: a name or
/// description holding one, or `every`, `any` or `all` `request[s]` ending a word, earns no bank
/// bonus (docs/recall.html#risks).
const CLAIM: [&str; 21] = [
    "must-read",
    "always recall",
    "always relevant",
    "always include",
    "always be recall",
    "always be relevant",
    "always be include",
    "relevant to every request",
    "relevant to every task",
    "relevant to all request",
    "relevant to all task",
    "relevant to any request",
    "relevant to any task",
    "in every recall",
    "required reading",
    "critical context",
    "ignore the question",
    "answer yes",
    "score 1.0",
    "scores 1.0",
    "applies to every",
];

/// The model command when `$SANDMAN_ENCODER` or `$SANDMAN_CONSOLIDATOR` names none: a headless
/// Claude Opus that reads only stdin, with no tools, no customizations (hooks, MCP servers,
/// CLAUDE.md) and no saved session.
const CLAUDE: &str = r#"claude -p --model opus --output-format text --safe-mode --tools "" --no-session-persistence"#;

/// Recall's embedding model and the instruct prefix of its query (docs/recall.html#chosen).
const EMBED: [&str; 2] = [
    "qwen3-embedding:0.6b",
    "Instruct: Given what an AI coding agent is about to do, retrieve the memory notes it needs to do it correctly\nQuery: ",
];

/// The cross-repository banks' labels, which earn a candidate 1.
const GENERAL: [&str; 2] = ["home (~)", "grove root (every repository)"];

/// The four memory types, a trace's and a memory's `type`, filed by repository.
const KINDS: [&str; 4] = ["feedback", "project", "reference", "user"];

/// The two knowledge types, filed by topic: a `knowledge` page (semantic memory, true beyond any
/// one episode) and a `playbook` (procedural memory, how a recurring task is done).
const KNOWN: [&str; 2] = ["knowledge", "playbook"];

/// A rewrite line's `op` (docs/index.html#cycle-rewrite).
const OPS: [&str; 9] = [
    "graduate", "keep", "merge", "prune", "rehome", "release", "retitle", "revert", "revise",
];

/// A note's `origin`, its source when it descends from no session and no note: `hand` (remembered
/// without a session), `import` (brought in from elsewhere), `routine` (a pipeline's).
const ORIGINS: [&str; 3] = ["hand", "import", "routine"];

/// Each model job's default guidance and its fixed contract, sent before its data; `guide` reads
/// `<root>/prompts/<job>.md` in place of the guidance.
const PROMPTS: [(&str, [&str; 2]); 4] = [
    (
        "consolidate",
        [
            include_str!("prompts/consolidate.md"),
            include_str!("prompts/consolidate.contract.md"),
        ],
    ),
    (
        "encode",
        [
            include_str!("prompts/encode.md"),
            include_str!("prompts/encode.contract.md"),
        ],
    ),
    (
        "judge",
        [
            include_str!("prompts/judge.md"),
            include_str!("prompts/judge.contract.md"),
        ],
    ),
    (
        "reflect",
        [
            include_str!("prompts/reflect.md"),
            include_str!("prompts/reflect.contract.md"),
        ],
    ),
];

/// Recall's fixed question, after each candidate's record: never read from a memory. The
/// reference's, byte for byte (docs/recall.html#chosen).
const QUESTION: &str = "Does the agent need this memory to carry out the request correctly?";

/// The old sandman's launchd agents, retired with the cutover (2026-11-14): `setup` boots each out
/// and removes its plist, and never writes one. Its `.upgrade` is not among them: hotswap's
/// `ensure` owns that label (2026-11-17).
const RETIRED: [&str; 3] = [
    "dev.gigantic.sandman.breathe",
    "dev.gigantic.sandman.reflect",
    "dev.gigantic.sandman.report",
];

/// The seed's instructions, first in a session's context; `<id>` is the session's id, `<me>` the
/// name the binary was invoked as.
const SEED: &str = "Memory from earlier sessions is yours to pull: run `<me> recall --session <id> --cue \"<what you are about to do>\"`, with `--about <repository path>` when the work is in one repository, before work in a repository, before a choice the person may already have ruled on, and when a correction may already be recorded; not every turn. Each line is a pointer: read it with `<me> show <path> --session <id>` before acting on it; a `~` marks one remembered but not yet consolidated. When the person states a preference, corrects you, or decides something a later session needs, run `<me> remember \"<their words>\" --session <id>` at once, one fact per call, with `--about <repository path>` when it holds in one repository, `--type knowledge|playbook --topic <topic>` for general knowledge or a playbook. When the person defers work (later, tomorrow, after X), run `<me> defer \"<their words>\" --when <today|tomorrow|friday|YYYY-MM-DD|+3d|the event> --session <id>` at once; close it with `<me> done <its id>` once finished. Once a recalled note proves used, wrong, stale or irrelevant, run `<me> rethink <path> --used|--wrong|--stale|--irrelevant --session <id>`.";

/// Recall's lexical gate: a cue whose every token is one of these asks for nothing.
const STEER: &str = "go yes yeah yep ok okay continue next thanks thank done saved good great morning back hi hello signed sure fine cool nice where we are what now";

/// The words `toks` drops.
const STOPWORDS: &str = "a an and are as at be by can do for from has have i if in into is it its me my no not of on or our so that the then this to was we what when where which will with you your about should would could lets let im dont get going want need make use using also just like than them there these they";

/// The three stores of notes, in reader order.
const STORES: [&str; 3] = ["memories", "knowledge", "traces"];

/// A `rethink` verdict: `irrelevant` blames the recall that sent the note, `stale` and `wrong`
/// the note, `used` raises it (docs/index.html#cycle-rethink).
const VERDICTS: [&str; 4] = ["irrelevant", "stale", "used", "wrong"];

/// The release (else the crate version) and the build, the commit it was built from (`build.rs`):
/// what `version` prints, the daemon's health carries and `status` compares.
const VERSION: &str = concat!(env!("SANDMAN_RELEASE"), "-", env!("SANDMAN_BUILD"));

/// The stop signal's name (`TERM`, `INT`, `HUP`), set by the signal thread before it wakes `accept`.
static STOP: OnceLock<&str> = OnceLock::new();

/// Puts each `(job, guidance)` in force: the guidance in force first copied to
/// `<root>/prompts/.history/<yyyy>/<mm>/<dd>/<HHMMSS>-<job>.md` (UTC, `dated`), then the new one
/// written to `<root>/prompts/<job>.md` by temp + rename; the copies' paths under the root, by job.
fn adopt(root: &Path, changes: &[(&str, String)]) -> io::Result<Map<String, Value>> {
    let (mut claimed, mut history) = (BTreeSet::new(), Map::new());
    for change in changes {
        let copy = dated(
            root,
            "prompts/.history",
            &format!("{}.md", change.0),
            &mut claimed,
        );
        put(&copy, &guide(root, change.0))?;
        put(&root.join(format!("prompts/{}.md", change.0)), &change.1)?;
        let kept = copy.strip_prefix(root).unwrap_or(&copy).to_string_lossy();
        drop(history.insert(change.0.to_owned(), kept.into()));
    }
    Ok(history)
}

/// The due block `sandman due` and the seed print: a header, then a row per open intention, `<id>
/// <due date | waiting: <event>> — <what>[ (<repository>)]`; the time-based ones due by the local
/// today, most overdue first, then every event-based one (no date tells when an event happened),
/// then with `all` the upcoming ones by date; past `cap` rows, `… <n> more: <me> due`. Empty
/// without a row.
fn agenda<F: Fn(&str) -> String>(root: &Path, all: bool, cap: usize, me: &str, label: F) -> String {
    let now = today();
    let mut open: Vec<(u8, Value)> = intentions(root)
        .into_iter()
        .filter(|held| held["state"] == "open")
        .map(|held| {
            let group = held["due"]
                .as_str()
                .map_or(1, |due| if due <= now.as_str() { 0 } else { 2 });
            (group, held)
        })
        .filter(|pair| all || pair.0 < 2)
        .collect();
    open.sort_by_key(|pair| {
        (
            pair.0,
            text(&pair.1["due"]).to_owned(),
            text(&pair.1["created"]).to_owned(),
        )
    });
    let more = open.len().saturating_sub(cap);
    let shown: String = open
        .into_iter()
        .take(cap)
        .map(|(_, held)| {
            format!(
                "  \u{b7} {} {} \u{2014} {}{}\n",
                text(&held["id"]),
                held["due"].as_str().map_or_else(
                    || format!("waiting: {}", text(&held["after"])),
                    str::to_owned
                ),
                one_line(text(&held["body"]), 160),
                held["about"].as_str().map_or_else(String::new, |about| {
                    format!(" ({})", label(&key(&project(Path::new(about)))))
                })
            )
        })
        .chain((more > 0).then(|| format!("  \u{2026} {more} more: {me} due\n")))
        .collect();
    if shown.is_empty() {
        shown
    } else {
        format!(
            "due \u{b7} deferred work: raise it with the person, never start it unasked\n{shown}"
        )
    }
}

/// `fields` with each of `changes` set in place, else appended; an empty value drops out when
/// `note` writes it.
fn amend<'text>(fields: &mut Vec<(&'text str, &'text str)>, changes: &[(&'text str, &'text str)]) {
    for &(field, value) in changes {
        match fields.iter_mut().find(|held| held.0 == field) {
            Some(held) => held.1 = value,
            None => fields.push((field, value)),
        }
    }
}

/// Appends `text` to `path` (created) in one `O_APPEND` write under a shared `flock`; a file
/// `forget` replaced while this waited on the lock is opened again, so no line lands in an unlinked
/// one.
fn append(path: &Path, text: &str) -> io::Result<()> {
    loop {
        let file = OpenOptions::new().append(true).create(true).open(path)?;
        file.lock_shared()?;
        if fs::metadata(path)?.ino() == file.metadata()?.ino() {
            return (&file).write_all(text.as_bytes());
        }
    }
}

/// The common git dir of the linked worktree at `dir`: its `.git` a file naming
/// `<common>/worktrees/<name>`, the shape `git worktree add` writes and a submodule's does not.
fn common(dir: &Path) -> Option<PathBuf> {
    let gitdir = dir.join(
        fs::read_to_string(dir.join(".git"))
            .ok()?
            .trim()
            .strip_prefix("gitdir:")?
            .trim(),
    );
    let worktrees = gitdir.parent()?;
    let repo = worktrees.parent().unwrap_or(worktrees);
    (name(worktrees) == "worktrees")
        .then(|| fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf()))
}

/// `whole` within `budget` bytes: past it, the head and the tail halves, cut back to whole lines,
/// around `… <n> bytes omitted …`.
fn cut(whole: String, budget: usize) -> String {
    if whole.len() <= budget {
        return whole;
    }
    let half = budget.div_euclid(2);
    let head = whole
        .get(..whole.floor_char_boundary(half))
        .and_then(|kept| kept.rfind('\n'))
        .map_or(0, |at| at.saturating_add(1));
    let from = whole.ceil_char_boundary(whole.len().saturating_sub(half));
    let tail = whole
        .get(from..)
        .and_then(|kept| kept.find('\n'))
        .map_or(whole.len(), |at| from.saturating_add(at).saturating_add(1));
    format!(
        "{}\u{2026} {} bytes omitted \u{2026}\n{}",
        whole.get(..head).unwrap_or_default(),
        tail.saturating_sub(head),
        whole.get(tail..).unwrap_or_default()
    )
}

/// Day `at` since the epoch as `YYYY-MM-DD`.
fn date(at: u64) -> String {
    let time = UNIX_EPOCH.checked_add(Duration::from_secs(at.saturating_mul(86_400)));
    utc(time.unwrap_or(UNIX_EPOCH))
        .get(..10)
        .unwrap_or_default()
        .to_owned()
}

/// A free `<root>/<dir>/<yyyy>/<mm>/<dd>/<HHMMSS>-<file>` at this UTC second, claimed; a clash
/// with a file or a claimed path takes `<HHMMSS>-<n>-<file>`, never the earlier one.
fn dated(root: &Path, dir: &str, file: &str, claimed: &mut BTreeSet<PathBuf>) -> PathBuf {
    let at = utc(SystemTime::now());
    let base = format!(
        "{}-{file}",
        at.get(11..19).unwrap_or_default().replace(':', "")
    );
    free(
        &root
            .join(dir)
            .join(at.get(..10).unwrap_or_default().replace('-', "/")),
        iter::once(base.clone())
            .chain((2u32..1000).map(|n| base.replacen('-', &format!("-{n}-"), 1))),
        claimed,
    )
}

/// `when`'s day since the epoch when it is a real `YYYY-MM-DD` from 1970 (Howard Hinnant's
/// `days_from_civil`, held to `date`'s round trip).
fn day(when: &str) -> Option<u64> {
    let mut parts = when.split('-').map(|part| part.parse::<u64>().ok());
    let (year, month, of) = (parts.next()??, parts.next()??, parts.next()??);
    let shifted = year.checked_sub(u64::from(month <= 2))?;
    let (era, yoe) = (shifted.div_euclid(400), shifted.rem_euclid(400));
    let doy = month
        .checked_add(9)?
        .rem_euclid(12)
        .checked_mul(153)?
        .checked_add(2)?
        .div_euclid(5)
        .checked_add(of)?
        .checked_sub(1)?;
    let days = era
        .checked_mul(146_097)?
        .checked_add(yoe.checked_mul(365)?)?
        .checked_add(yoe.div_euclid(4))?
        .checked_sub(yoe.div_euclid(100))?
        .checked_add(doy)?
        .checked_sub(719_468)?;
    (date(days) == when).then_some(days)
}

/// Each of `texts` embedded by `$SANDMAN_OLLAMA`'s `/api/embed`, 16 a request, as the reference
/// sends them (raw, truncated to 1,024 tokens), L2-normalised.
#[expect(clippy::float_arithmetic, reason = "the norm is float math")]
fn embed(texts: &[String], begun: Instant, limit: Duration) -> io::Result<Vec<Vec<f64>>> {
    let url = format!(
        "{}/api/embed",
        setting("SANDMAN_OLLAMA", "http://127.0.0.1:11434".to_owned())
    );
    texts.chunks(16).try_fold(Vec::new(), |mut done, batch| {
        let request = json!({"input": batch, "model": EMBED[0], "options": {"num_ctx": 1024u16}, "truncate": true});
        let reply = post(&url, &request.to_string(), "", begun, limit).0?;
        for vector in reply["embeddings"]
            .as_array()
            .filter(|vectors| vectors.len() == batch.len())
            .ok_or_else(|| io::Error::other("reply: not one embedding per text"))?
        {
            let values: Vec<f64> = vector
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_f64)
                .collect();
            let length = total(values.iter().map(|value| value * value)).sqrt();
            let norm = if length > 0.0f64 { length } else { 1.0f64 };
            done.push(values.iter().map(|value| value / norm).collect());
        }
        Ok(done)
    })
}

/// Each note a `rewrite` line of `lines` ended, in ledger order: `(the line's index, the line, its
/// from entry)` per entry with an id that is not the line's `to` id (a keep continues the note).
fn ended(lines: &[Value]) -> Vec<(usize, &Value, &Value)> {
    lines
        .iter()
        .enumerate()
        .filter(|pair| pair.1["kind"] == "rewrite")
        .flat_map(|(at, line)| {
            line["from"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(move |entry| entry["id"].is_string() && entry["id"] != line["to"]["id"])
                .map(move |entry| (at, line, entry))
        })
        .collect()
}

/// Ends `session`'s ledger story with one end line, returned as written: `decided` (skipped,
/// held) as is, else the take — the transcript and its sidecar renamed into the archive, then the
/// pointer — or `gone` when there is no transcript.
fn end(
    root: &Path,
    claude: &Path,
    session: &str,
    cwd: &Value,
    reason: &Value,
    decided: Option<Value>,
) -> io::Result<Value> {
    let mut line = match (decided, transcript(claude, session)) {
        (Some(line), _) => line,
        (None, None) => json!({"outcome": "gone"}),
        (None, Some(path)) => {
            let archived = dated(
                root,
                ".archive/claude",
                &path
                    .strip_prefix(claude)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('/', "-"),
                &mut BTreeSet::new(),
            );
            let ended = utc(fs::metadata(&path)?.modified()?);
            archived.parent().map_or(Ok(()), fs::create_dir_all)?;
            fs::rename(&path, &archived)?;
            let sidecar = path.with_extension("");
            if sidecar.is_dir() {
                let beside = format!(
                    "{}-dir",
                    archived.to_string_lossy().trim_end_matches(".jsonl")
                );
                fs::rename(sidecar, beside)?;
            }
            let lines = jsonl(&archived);
            let title = lines
                .iter()
                .flat_map(parts)
                .filter(|&(role, _)| role == "user")
                .map(|(_, said)| one_line(&said, 80))
                .find(|title| !title.is_empty())
                .unwrap_or_else(|| session.to_owned());
            let pointer = format!(
                "{{\"archived\":{},\"cwd\":{},\"ended\":{},\"title\":{},\"highlights\":{}}}\n",
                json!(archived.to_string_lossy()),
                json!(place(&lines, cwd)),
                json!(ended),
                json!(title),
                json!(highlights(&lines))
            );
            let recent = root.join(format!("memories/.recent/{session}.json"));
            put(&recent, &pointer)?;
            json!({"archived": archived.to_string_lossy(), "outcome": "taken"})
        }
    };
    line["cwd"] = cwd.clone();
    line["kind"] = "end".into();
    line["reason"] = reason.clone();
    line["session"] = session.into();
    record(root, &mut line).map(|()| line)
}

/// An episode's digest from its end line and the ledger's `lines` (AGENTS.md `### encode`), cut to
/// `budget` bytes, the directory it ran in, and the notes its `# recalled` lists.
fn episode(
    root: &Path,
    lines: &[Value],
    closing: &Value,
    budget: usize,
) -> (String, Option<String>, Vec<Value>) {
    let (session, turns, all) = (
        text(&closing["session"]),
        jsonl(Path::new(text(&closing["archived"]))),
        every(root),
    );
    let cwd = place(&turns, &closing["cwd"]);
    // what the session already remembered is done; a highlight it covers (the same body,
    // whitespace collapsed) is dropped
    let remembered: Vec<&Value> = all
        .iter()
        .filter(|note| note["session"] == session && text(&note["source"]).starts_with("remember"))
        .collect();
    // the notes its recall lines sent and no verdict of the session judged, each shown or not
    let (named, mine) = (
        |line: &Value, note: &Value| {
            line["file"] == note["path"] || (note["id"].is_string() && line["id"] == note["id"])
        },
        |kind: &'static str| {
            lines
                .iter()
                .filter(move |line| line["kind"] == kind && line["session"] == session)
        },
    );
    let recalled: Vec<Value> = all
        .iter()
        .filter(|note| {
            mine("recall").any(|line| {
                line["sent"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|entry| named(entry, note))
            }) && !mine("rethink").any(|line| named(line, note))
        })
        .cloned()
        .collect();
    let digest = format!(
        "# episode\ndate: {}\ncwd: {}\n\n{}# transcript\n{}",
        text(&closing["at"]),
        cwd.unwrap_or("unknown"),
        sections(&[
            (
                "already remembered",
                remembered
                    .iter()
                    .map(|note| {
                        format!(
                            "{} \u{2014} {}",
                            text(&note["name"]),
                            text(&note["description"])
                        )
                    })
                    .collect(),
            ),
            (
                "highlights",
                highlights(&turns)
                    .into_iter()
                    .filter(|body| {
                        !remembered
                            .iter()
                            .any(|note| one_line(text(&note["body"]), 4000) == *body)
                    })
                    .collect(),
            ),
            (
                "recalled",
                recalled
                    .iter()
                    .map(|note| {
                        format!(
                            "{} \u{b7} {} \u{2014} {} \u{b7} {}",
                            text(&note["path"]),
                            text(&note["name"]),
                            one_line(text(&note["description"]), 160),
                            ["not shown", "shown"]
                                [usize::from(mine("show").any(|line| named(line, note)))]
                        )
                    })
                    .collect(),
            ),
        ]),
        turns
            .iter()
            .flat_map(parts)
            .map(|(role, said)| [role, ": ", &said, "\n"].concat())
            .collect::<String>()
    );
    (cut(digest, budget), cwd.map(str::to_owned), recalled)
}

/// A rewrite line's `from` entry for `note` as it stands: its file's digest, its id, its path and
/// the sessions it descends from.
fn entry(note: &Value) -> Value {
    json!({"digest": sha256(text(&note["text"])), "id": note["id"], "path": note["path"], "sessions": lineage([note])})
}

/// Every note recall and `show` read, in reader order: the memories, the knowledge pages, the
/// traces.
fn every(root: &Path) -> Vec<Value> {
    STORES.iter().flat_map(|dir| notes(root, dir)).collect()
}

/// What is embedded of a candidate: its head and the first 1,500 characters of its body.
fn excerpt(memory: &Value, label: &str) -> String {
    format!(
        "{}\n{}",
        head(memory, label),
        text(&memory["body"]).chars().take(1500).collect::<String>()
    )
}

/// The first of `names` under `dir` that no file holds and this run has not claimed, claimed.
fn free<I: IntoIterator<Item = String>>(
    dir: &Path,
    names: I,
    claimed: &mut BTreeSet<PathBuf>,
) -> PathBuf {
    let path = names
        .into_iter()
        .map(|file| dir.join(file))
        .find(|path| !path.exists() && !claimed.contains(path))
        .unwrap_or_default();
    let _: bool = claimed.insert(path.clone());
    path
}

/// A note's frontmatter: its `key: value` lines between `---` fences, in order, trimmed.
fn front(text: &str) -> Vec<(&str, &str)> {
    text.strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---"))
        .map_or("", |(head, _)| head)
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim(), value.trim()))
        .collect()
}

/// A job's guidance in force: `<root>/prompts/<job>.md` when it holds any non-blank text, else the
/// default built in.
fn guide(root: &Path, job: &str) -> String {
    let custom = fs::read_to_string(root.join(format!("prompts/{job}.md"))).unwrap_or_default();
    if custom.trim().is_empty() {
        PROMPTS
            .iter()
            .find(|prompt| prompt.0 == job)
            .map_or("", |prompt| prompt.1[0])
            .to_owned()
    } else {
        custom
    }
}

/// An embedding's cache key, the reference's: the SHA-256 of `<model>\n1024\n<text>`.
fn hash(text: &str) -> String {
    sha256(&format!("{}\n1024\n{text}", EMBED[0]))
}

/// A candidate's head as the reference writes it: `<label> · <name>\n<description, one-lined to
/// 400>`.
fn head(memory: &Value, label: &str) -> String {
    format!(
        "{label} \u{b7} {}\n{}",
        text(&memory["name"]),
        one_line(text(&memory["description"]), 400)
    )
}

/// Whether `text` is a note's id: 12 lowercase hex digits.
fn hexid(text: &str) -> bool {
    text.len() == 12
        && text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// The quoted bodies after `sandman remember ` and `orrery attend ` in the commands a transcript's
/// session ran (an assistant's `tool_use`, a user's `!` line), one-lined, a repeat moved to the
/// end, the last 20.
fn highlights(lines: &[Value]) -> Vec<String> {
    let mut highlights: Vec<String> = Vec::new();
    for command in lines.iter().flat_map(|line| {
        let content = &line["message"]["content"];
        let user = line["type"] == "user";
        let bang = content
            .as_str()
            .filter(|said| user && said.starts_with("<bash-input>"));
        let ran = content
            .as_array()
            .into_iter()
            .flatten()
            .filter(|block| line["type"] == "assistant" && block["type"] == "tool_use");
        bang.into_iter()
            .chain(ran.filter_map(|block| block["input"]["command"].as_str()))
    }) {
        for marker in ["sandman remember ", "orrery attend "] {
            let mut rest = command;
            while let Some((_, after)) = rest.split_once(marker) {
                rest = after;
                let mut body = String::new();
                // `\"…\"` (shell-escaped) closes at the next `\"`; `"…"` at the next
                // unescaped `"`, unescaping `\"` and `\\`
                match (after.strip_prefix("\\\""), after.strip_prefix('"')) {
                    (Some(open), _) => {
                        let Some((inner, tail)) = open.split_once("\\\"") else {
                            continue;
                        };
                        (body, rest) = (inner.replace("\\\\", "\\"), tail);
                    }
                    (None, Some(open)) => {
                        let mut chars = open.chars();
                        loop {
                            match (chars.next(), chars.as_str().chars().next()) {
                                (Some('"'), _) => break rest = chars.as_str(),
                                (Some('\\'), Some(escaped @ ('"' | '\\'))) => {
                                    body.push(escaped);
                                    chars = chars.as_str().get(1..).unwrap_or_default().chars();
                                }
                                (Some(other), _) => body.push(other),
                                (None, _) => break body.clear(),
                            }
                        }
                    }
                    (None, None) => {}
                }
                let kept = one_line(&body, 4000);
                if !kept.is_empty() {
                    highlights.retain(|held| *held != kept);
                    highlights.push(kept);
                }
            }
        }
    }
    highlights.split_off(highlights.len().saturating_sub(20))
}

/// Every intention, `<root>/intentions/<id>.md`, as `read` gives it with its `id`, by id.
fn intentions(root: &Path) -> Vec<Value> {
    tree(&root.join("intentions"), 1)
        .iter()
        .filter(|path| path.extension() == Some("md".as_ref()))
        .map(|path| {
            read(
                json!({"id": path.file_stem().and_then(OsStr::to_str)}),
                fs::read_to_string(path).unwrap_or_default(),
            )
        })
        .collect()
}

/// The background job under `<claude>/jobs/` that names `session`: its `resumeSessionId`, else
/// its `sessionId` (a `/clear` moves a job on to a new id and leaves the old one in `sessionId`);
/// a finished one (`state` `done` or `stopped`, which Claude Code keeps on disk) holds nothing.
fn job(claude: &Path, session: &str) -> Option<String> {
    tree(&claude.join("jobs"), 1).into_iter().find_map(|dir| {
        let state: Value =
            serde_json::from_str(&fs::read_to_string(dir.join("state.json")).ok()?).ok()?;
        (state["resumeSessionId"]
            .as_str()
            .or_else(|| state["sessionId"].as_str())
            == Some(session)
            && !["done", "stopped"].contains(&text(&state["state"])))
        .then(|| name(&dir).to_owned())
    })
}

/// Every line of the JSONL file at `path` that parses; none when it cannot be read.
fn jsonl(path: &Path) -> Vec<Value> {
    String::from_utf8_lossy(&fs::read(path).unwrap_or_default())
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// The `wrong` and `stale` verdicts on `note` since its body last changed (docs/index.html#cycle-strength),
/// by path, else id.
fn judged<'line>(lines: &'line [Value], note: &Value) -> Vec<&'line Value> {
    lines
        .iter()
        .filter(|line| {
            line["kind"] == "rethink"
                && ["stale", "wrong"].contains(&text(&line["verdict"]))
                && line["at"].as_str() > when(note)
                && (line["file"] == note["path"]
                    || (note["id"].is_string() && line["id"] == note["id"]))
        })
        .collect()
}

/// A directory's bank key, the earlier sandman's `Bank::key_for`: every byte not ASCII
/// alphanumeric turned into `-`, so distinct paths can share a key.
fn key(path: &Path) -> String {
    path.as_os_str()
        .as_encoded_bytes()
        .iter()
        .map(|&byte| {
            if byte.is_ascii_alphanumeric() {
                char::from(byte)
            } else {
                '-'
            }
        })
        .collect()
}

/// The grove root (`$SANDMAN_GROVE`, else `<home>/.grove/code`), its owner and repository dirs as
/// `(bank key, label)`, longest key first, and a bank's label over them, so a worktree's bank folds
/// into its repository's (AGENTS.md `### sandman recall`).
fn labels(home: &Path, claude: &Path) -> (PathBuf, Vec<(String, String)>, impl Fn(&str) -> String) {
    let grove =
        env::var_os("SANDMAN_GROVE").map_or_else(|| home.join(".grove/code"), PathBuf::from);
    let (shown, owned) = (
        |dir: &PathBuf| dir.is_dir() && !name(dir).starts_with('.'),
        " (owner: all its repositories)",
    );
    let mut repos: Vec<(String, String)> = tree(&grove, 1)
        .into_iter()
        .filter(shown)
        .flat_map(|owner| {
            let who = name(&owner).to_owned();
            let mut dirs = vec![(key(&owner), format!("{who}{owned}"))];
            dirs.extend(
                tree(&owner, 1)
                    .iter()
                    .filter(|dir| shown(dir))
                    .map(|repo| (key(repo), format!("{who}/{}", name(repo)))),
            );
            dirs
        })
        .collect();
    repos.sort_by_key(|repo| usize::MAX.saturating_sub(repo.0.len()));
    let (homed, rooted, jobs, known) = (
        key(home),
        key(&grove),
        key(&claude.join("jobs")),
        repos.clone(),
    );
    let label = move |bank: &str| {
        let under = |dir: &str| bank.strip_prefix(dir)?.strip_prefix('-');
        let readable = bank.replace("--", "/.").replace('-', "/");
        if bank == homed {
            GENERAL[0].to_owned()
        } else if bank == rooted {
            GENERAL[1].to_owned()
        } else if let Some(topic) = bank.strip_prefix('@') {
            format!("knowledge \u{b7} {topic}")
        } else if let Some(repo) = known
            .iter()
            .find(|repo| bank == repo.0 || (!repo.1.ends_with(owned) && under(&repo.0).is_some()))
        {
            repo.1.clone()
        } else if let Some(gone) = known.iter().find_map(|repo| {
            // a repository gone from disk, under an owner still there
            Some(format!(
                "{}/{}",
                repo.1.strip_suffix(owned)?,
                under(&repo.0)?
            ))
        }) {
            gone
        } else if under(&jobs).is_some() {
            "scratch dir".to_owned()
        } else if under(&homed).is_some() {
            format!("~{}", readable.get(homed.len()..).unwrap_or_default())
        } else {
            readable
        }
    };
    (grove, repos, label)
}

/// Every line of `<root>/ledger/*/*/*.jsonl` that parses, oldest file first; a staged `.` file
/// skipped.
fn ledger(root: &Path) -> Vec<Value> {
    tree(&root.join("ledger"), 3)
        .iter()
        .filter(|file| !name(file).starts_with('.'))
        .flat_map(|file| jsonl(file))
        .collect()
}

/// The archived sessions `notes` descend from, sorted, once each: the ids on each one's
/// `sessions:` line, its `session:`, and the one a consolidate's or a dream's `source` stamp names.
fn lineage<'note, I: IntoIterator<Item = &'note Value>>(notes: I) -> BTreeSet<&'note str> {
    notes
        .into_iter()
        .flat_map(|note| {
            let source = text(&note["source"]);
            let stamped = source
                .split_whitespace()
                .next()
                .is_some_and(|word| ["consolidate", "dream"].contains(&word));
            [
                text(&note["sessions"]),
                text(&note["session"]),
                if stamped { source } else { "" },
            ]
        })
        .flat_map(str::split_whitespace)
        .filter(|word| uuid(word))
        .collect()
}

/// Where `note` is on disk: its path under the root, a memory's under `memories/`.
fn located(root: &Path, note: &Value) -> PathBuf {
    let path = text(&note["path"]);
    if path.starts_with("knowledge/") || path.starts_with("traces/") {
        root.join(path)
    } else {
        root.join("memories").join(path)
    }
}

/// This machine's id: `$SANDMAN_MACHINE`, else `<root>/machine`, created on first use by a hard
/// link from a temp file, so concurrent first runs agree on one id.
fn machine(root: &Path) -> String {
    env::var("SANDMAN_MACHINE").unwrap_or_else(|_| {
        let path = root.join("machine");
        let read = || fs::read_to_string(&path).map(|text| text.trim().to_owned());
        read().unwrap_or_else(|_| {
            let hex = random();
            let tmp = staged(&path);
            drop(
                fs::create_dir_all(root)
                    .and_then(|()| fs::write(&tmp, &hex))
                    .and_then(|()| fs::hard_link(&tmp, &path)),
            );
            drop(fs::remove_file(&tmp));
            read().unwrap_or(hex)
        })
    })
}

/// sandman's hotswap manifest (gigantic.dev/hotswap/): releases from jgeschwendt/sandman under its
/// packslip signer, `sandman setup` after each swap and the daemon the one agent kickstarted. No
/// drain step: the kickstart's SIGTERM is the daemon's own drain, inside its 2 s `ExitTimeOut`,
/// and a separate stop would race `KeepAlive`'s respawn. The smoke is the new binary's `version`,
/// then `status` until the restarted daemon answers with that same version, 30 s at most.
fn manifest() -> hotswap::Manifest {
    let owned = |words: &[&str]| words.iter().map(|&word| word.to_owned()).collect();
    hotswap::Manifest {
        agents: owned(&["dev.gigantic.sandman"]),
        bin: "sandman".into(),
        repo: "jgeschwendt/sandman".into(),
        setup: owned(&["sandman", "setup"]),
        signer: "ps1_nensfiggtc7n6uctbniwf7q4am".into(),
        smoke: vec![
            owned(&["sandman", "version"]),
            owned(&[
                "/bin/sh",
                "-c",
                r#"for _ in $(seq 30); do sandman status | grep -qF "\"version\":\"$(sandman version)\"" && exit 0; sleep 1; done; exit 1"#,
            ]),
        ],
        ..hotswap::Manifest::default()
    }
}

/// A bank's order, else the store's (docs/index.html#cycle-rewrite): its active notes, its traces
/// among them, and their characters.
fn measure(root: &Path, bank: Option<&str>) -> Value {
    let held: Vec<Value> = every(root)
        .into_iter()
        .filter(|note| bank.is_none_or(|one| note["bank"] == one))
        .collect();
    json!({"chars": held.iter().map(|note| text(&note["text"]).chars().count()).sum::<usize>(), "notes": held.len()})
}

/// A new note's id: 12 lowercase hex digits from `/dev/urandom`.
fn mint() -> String {
    random().get(..12).unwrap_or_default().to_owned()
}

/// The model a command names: the word after `--model`, else its first word.
fn model(command: &str) -> &str {
    let words = command.split_whitespace();
    let first = words.clone().next();
    words
        .skip_while(|word| *word != "--model")
        .nth(1)
        .or(first)
        .unwrap_or_default()
}

/// A path's last component, or "".
fn name(path: &Path) -> &str {
    path.file_name().and_then(OsStr::to_str).unwrap_or_default()
}

/// A note file's text: `fields` as frontmatter (a field with an empty value left out), a blank
/// line, `body` trimmed.
fn note(fields: &[(&str, &str)], body: &str) -> String {
    let head: String = fields
        .iter()
        .filter(|field| !field.1.is_empty())
        .map(|field| [field.0, ": ", field.1, "\n"].concat())
        .collect();
    format!("---\n{head}---\n\n{}\n", body.trim())
}

/// Every `<root>/<dir>/<bank>/<file>.md` but `MEMORY.md`, banks and files starting `.` or `_`
/// skipped: its frontmatter as an object, with `bank` (`@<topic>` for a knowledge page, its traces'
/// bank), `file`, `text` (the whole file), `body` (frontmatter stripped, trimmed), `path` (what
/// `show` takes: `<bank>/<file>` for a memory, `<dir>/<bank>/<file>` else) and `name` when the
/// frontmatter has none (the stem). By bank, then file.
fn notes(root: &Path, dir: &str) -> Vec<Value> {
    tree(&root.join(dir), 2)
        .iter()
        .filter_map(|path| {
            let (bank, file) = (path.parent().map_or("", name), name(path));
            let skip = [bank, file].iter().any(|part| part.starts_with(['.', '_']));
            (!skip
                && file != "MEMORY.md"
                && path.extension() == Some("md".as_ref())
                && path.is_file())
            .then(|| {
                let mut note = read(
                    json!({"name": path.file_stem().and_then(OsStr::to_str)}),
                    fs::read_to_string(path).unwrap_or_default(),
                );
                let address = format!("{dir}/{bank}/{file}");
                (note["bank"], note["file"], note["path"]) = (
                    [if dir == "knowledge" { "@" } else { "" }, bank]
                        .concat()
                        .into(),
                    file.into(),
                    address.strip_prefix("memories/").unwrap_or(&address).into(),
                );
                note
            })
        })
        .collect()
}

/// `text` with its whitespace collapsed to single spaces and cut at `max` characters.
fn one_line(text: &str, max: usize) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

/// A transcript line's parts in order: `(role, text)` per text of a user or assistant message,
/// `("tool", name and command or file)` per tool call; tool results and other lines dropped.
fn parts(line: &Value) -> Vec<(&str, String)> {
    let content = &line["message"]["content"];
    let role = line["type"].as_str().unwrap_or_default();
    content
        .as_array()
        .cloned()
        .unwrap_or_else(|| vec![json!({"text": content, "type": "text"})])
        .iter()
        .filter(|_| ["assistant", "user"].contains(&role))
        .filter_map(|block| match block["type"].as_str() {
            Some("text") => Some((role, block["text"].as_str()?.to_owned())),
            Some("tool_use") => Some((
                "tool",
                one_line(
                    &format!(
                        "{} {}",
                        block["name"].as_str().unwrap_or_default(),
                        block["input"]["command"]
                            .as_str()
                            .or_else(|| block["input"]["file_path"].as_str())
                            .unwrap_or_default()
                    ),
                    200,
                ),
            )),
            _ => None,
        })
        .collect()
}

/// A transcript's working directory: the first line's `cwd` that is neither empty nor carries a
/// control character, else `fallback`'s.
fn place<'line>(lines: &'line [Value], fallback: &'line Value) -> Option<&'line str> {
    lines
        .iter()
        .map(|line| &line["cwd"])
        .chain([fallback])
        .filter_map(Value::as_str)
        .find(|dir| !dir.is_empty() && !dir.contains(char::is_control))
}

/// Every note recall reads, muted ones dropped: the always-set (a memory's or a memory trace's
/// `user` or pinned, never knowledge), then the candidates, each in reader order.
fn pool(root: &Path) -> (Vec<Value>, Vec<Value>) {
    every(root)
        .into_iter()
        .filter(|note| note["recall"] != "mute")
        .partition(|note| {
            (note["type"] == "user" || note["recall"] == "pin")
                && !text(&note["bank"]).starts_with('@')
        })
}

/// `request` posted to `url` by curl through `sh -c` in its own process group, fed and drained
/// beside the wait, the group killed when `limit` past `begun` runs out: the reply, and what curl
/// printed on both streams. A `key` goes to curl as config on its stdin and the request on fd 3,
/// curl run with the key's variable emptied, so it is never in an argv.
fn post(
    url: &str,
    request: &str,
    key: &str,
    begun: Instant,
    limit: Duration,
) -> (io::Result<Value>, String) {
    let curl = |config: &str, data: &str| {
        format!(
            "curl -q {config}-sS --fail-with-body --max-time {} -H 'Content-Type: application/json' --data-binary @{data} {url}",
            limit.as_secs_f64()
        )
    };
    let command = if key.is_empty() {
        curl("", "-")
    } else {
        format!(
            r#"{{ printf 'header = "Authorization: Bearer %s"\n' "$SANDMAN_SYSTEMONE_KEY" | SANDMAN_SYSTEMONE_KEY= {}; }} 3<&0"#,
            curl("-K - ", "/dev/fd/3")
        )
    };
    let mut pipes = (String::new(), String::new());
    let answer = (|| {
        let (input, mut feed) = io::pipe()?;
        let mut child = Command::new("sh")
            .args(["-c", &command])
            .env("SANDMAN_SYSTEMONE_KEY", key)
            .process_group(0)
            .stdin(input)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let owned = request.to_owned();
        drop(thread::spawn(move || feed.write_all(owned.as_bytes())));
        let mut out = child.stdout.take();
        let read = thread::spawn(move || {
            let mut text = String::new();
            drop(out.as_mut().map(|pipe| pipe.read_to_string(&mut text)));
            text
        });
        let status = reap(&mut child, begun, limit)?;
        drop(
            child
                .stderr
                .as_mut()
                .map(|pipe| pipe.read_to_string(&mut pipes.1)),
        );
        pipes.0 = read.join().unwrap_or_default();
        reply(status, &pipes.0, &pipes.1)
    })();
    (answer, format!("{}{}", pipes.0, pipes.1))
}

/// The directory `path`'s memories belong to, the earlier sandman's `project::root`: a linked
/// worktree keys as the directory it sits in, a worktree nested in a checkout of its repo as that
/// checkout's project, a vanished directory beside one repo's worktrees as that repo's project;
/// a primary checkout, a submodule or a plain directory as itself.
fn project(path: &Path) -> PathBuf {
    let cwd: PathBuf = path.components().collect();
    if !cwd.exists()
        && let Some(live) = cwd.ancestors().find(|dir| dir.exists())
    {
        let mut worktrees = fs::read_dir(live)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| Some((common(&entry.path())?, entry.path())));
        if let Some((repo, sibling)) = worktrees.next()
            && worktrees.all(|(other, _)| other == repo)
        {
            return project(&sibling);
        }
    }
    let (mut found, mut outer) = (None, None);
    for dir in cwd.ancestors() {
        let dot_git = dir.join(".git");
        match common(dir) {
            Some(repo) if [None, Some(&repo)].contains(&outer.as_ref()) => {
                (found, outer) = (dir.parent().or(Some(dir)), Some(repo));
            }
            None if outer.is_some() && fs::canonicalize(&dot_git).ok() == outer => {
                return dir.to_path_buf();
            }
            // a primary checkout, a submodule or another repo's worktree owns everything below it
            _ if dot_git.exists() => break,
            _ => {}
        }
    }
    found.map_or_else(|| cwd.clone(), Path::to_path_buf)
}

/// Writes `text` to `path` by a temp file beside it and a rename, creating its directory.
fn put(path: &Path, text: &str) -> io::Result<()> {
    path.parent().map_or(Ok(()), fs::create_dir_all)?;
    fs::write(staged(path), text)?;
    fs::rename(staged(path), path)
}

/// 32 lowercase hex digits from `/dev/urandom`.
fn random() -> String {
    let mut bytes = [0u8; 16];
    drop(File::open("/dev/urandom").and_then(|mut source| source.read_exact(&mut bytes)));
    bytes.map(|byte| format!("{byte:02x}")).concat()
}

/// `note` with `whole`'s frontmatter fields set on it, then `body` (the frontmatter stripped,
/// trimmed) and `text` (`whole`).
fn read(mut note: Value, whole: String) -> Value {
    for (key, value) in front(&whole) {
        note[key] = value.into();
    }
    note["body"] = whole
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---"))
        .map_or(whole.as_str(), |(_, body)| body)
        .trim()
        .into();
    note["text"] = whole.into();
    note
}

/// Appends `line`, stamped with `at` and `machine`, to today's ledger file (`append`).
fn record(root: &Path, line: &mut Value) -> io::Result<()> {
    let at = utc(SystemTime::now());
    let day = root.join(format!(
        "ledger/{}.jsonl",
        at.get(..10).unwrap_or_default().replace('-', "/")
    ));
    line["at"] = at.into();
    line["machine"] = machine(root).into();
    day.parent().map_or(Ok(()), fs::create_dir_all)?;
    append(&day, &format!("{line}\n"))
}

/// Waits on `child`, spawned in its own process group, for its status; `limit` past `begun` or on
/// a stop signal the whole group is killed and the wait fails.
fn reap(child: &mut Child, begun: Instant, limit: Duration) -> io::Result<ExitStatus> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if STOP.get().is_some() || begun.elapsed() > limit {
            // the whole group: whatever the command started dies with it
            drop(
                Command::new("kill")
                    .args(["-KILL", &format!("-{}", child.id())])
                    .status(),
            );
            drop(child.wait());
            return Err(io::Error::other(format!(
                "timed out after {} ms",
                limit.as_millis()
            )));
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// A command's reply: `<status>: <stderr>` unless it exited 0, else the object from its stdout's
/// first `{` to its last `}`, so a fence around it passes.
fn reply(status: ExitStatus, out: &str, err: &str) -> io::Result<Value> {
    if !status.success() {
        return Err(io::Error::other(format!(
            "{status}: {}",
            one_line(err, 200)
        )));
    }
    serde_json::from_str(
        out.get(out.find('{').unwrap_or(0)..=out.rfind('}').unwrap_or(0))
            .unwrap_or_default(),
    )
    .map_err(|error| io::Error::other(format!("output: {error}")))
}

/// Ends `note` outside consolidate (decay, a graduation approved): its file moved aside to
/// `<root>/.pruned/<yyyy>/<mm>/<dd>/<HHMMSS>-<bank>-<file>`, then one `rewrite` line of `op`.
fn retire(root: &Path, note: &Value, op: &str, reason: &Value, to: &Value) -> io::Result<()> {
    let aside = dated(
        root,
        ".pruned",
        &format!("{}-{}", text(&note["bank"]), text(&note["file"])),
        &mut BTreeSet::new(),
    );
    aside.parent().map_or(Ok(()), fs::create_dir_all)?;
    fs::rename(located(root, note), aside)?;
    record(
        root,
        &mut json!({"from": [entry(note)], "kind": "rewrite", "model": null, "op": op, "reason": reason, "to": to}),
    )
}

/// One verdict on `note` (docs/index.html#cycle-rethink): `line`, holding `by`, `session`,
/// `verdict` and `why`, completed as a `rethink` line, `cue` the latest of the session's recall
/// lines that sent the note (by path, else id); a send judged also appends an example to
/// `eval/recall.jsonl`, `irrelevant` label 0, the rest 1.
fn rethink(root: &Path, lines: &[Value], note: &Value, mut line: Value) -> io::Result<()> {
    let recalled = lines.iter().rev().find(|sent| {
        sent["kind"] == "recall"
            && sent["session"] == line["session"]
            && sent["sent"].as_array().into_iter().flatten().any(|entry| {
                entry["file"] == note["path"]
                    || (note["id"].is_string() && entry["id"] == note["id"])
            })
    });
    (line["cue"], line["file"], line["id"], line["kind"]) = (
        recalled.map_or(Value::Null, |sent| sent["cue"].clone()),
        note["path"].clone(),
        note["id"].clone(),
        "rethink".into(),
    );
    record(root, &mut line)?;
    recalled.map_or(Ok(()), |sent| {
        let example = json!({
            "about": sent["about"], "at": line["at"], "cue": sent["cue"], "digest": sha256(text(&note["text"])),
            "id": note["id"], "label": u8::from(line["verdict"] != "irrelevant"), "path": note["path"],
            "session": line["session"], "verdict": line["verdict"],
        });
        fs::create_dir_all(root.join("eval"))?;
        append(&root.join("eval/recall.jsonl"), &format!("{example}\n"))
    })
}

/// A tick job's model run: `command` through `sh -c` in its own process group, flagged so a
/// session it starts is never taken, stdin `run/<job>.in` (`guidance`, else the job's in force;
/// its contract; `data`), stdout and stderr to `run/<job>.out` and `.err`; the long-lived sign-in
/// in `$CLAUDE_CODE_OAUTH_TOKEN` when its file passes, `noted` what the job's line says of it. A
/// stop signal kills the group and fails the run.
fn run(
    root: &Path,
    job: &str,
    guidance: Option<&str>,
    command: &str,
    data: &str,
    limit: Duration,
    noted: &Cell<Option<String>>,
) -> io::Result<Value> {
    let file = |ext: &str| root.join(format!("run/{job}.{ext}"));
    let input = format!(
        "{}\n{}\n{data}",
        guidance.map_or_else(|| guide(root, job), str::to_owned),
        PROMPTS
            .iter()
            .find(|prompt| prompt.0 == job)
            .map_or("", |prompt| prompt.1[1])
    );
    fs::write(file("in"), input)?;
    // read at each run: a regular file no group or other can read, its trimmed text the token
    let path = env::var_os("SANDMAN_CLAUDE_TOKEN_FILE").map_or_else(
        || Path::new(&env::var_os("HOME").unwrap_or_default()).join("Codes/claude-oauth-token"),
        PathBuf::from,
    );
    let opened = File::open(&path).and_then(|found| {
        let mode = found.metadata()?.mode();
        if mode & 0o170_077 != 0o100_000 {
            return Err(io::Error::other(format!(
                "mode {mode:o}, want a regular file of mode 600"
            )));
        }
        io::read_to_string(found).map(|token| token.trim().to_owned())
    });
    let shown = path.display();
    let (token, note) = match opened {
        Err(error) if error.kind() == io::ErrorKind::NotFound => (None, None),
        Err(error) => (None, Some(format!("refused: {shown}: {error}"))),
        Ok(token) if token.is_empty() => (None, Some(format!("ignored: {shown} is empty"))),
        Ok(token) => (Some(token), Some(shown.to_string())),
    };
    noted.set(note);
    let mut child = Command::new("sh")
        .args(["-c", command])
        .env("CLAUDE_MEMORY_PIPELINE", "1")
        .env("SANDMAN_NO_TAKE", "1")
        .envs(
            token
                .iter()
                .map(|secret| ("CLAUDE_CODE_OAUTH_TOKEN", secret)),
        )
        .process_group(0)
        .stdin(File::open(file("in"))?)
        .stdout(File::create(file("out"))?)
        .stderr(File::create(file("err"))?)
        .spawn()?;
    let status = reap(&mut child, Instant::now(), limit)?;
    let output = |ext: &str| fs::read_to_string(file(ext));
    reply(status, &output("out")?, &output("err")?)
}

/// Per titled list that has any items: `# <title>`, a `- <item>` line each, a blank line.
fn sections(lists: &[(&str, Vec<String>)]) -> String {
    lists
        .iter()
        .filter(|list| !list.1.is_empty())
        .map(|list| {
            let items: String = list
                .1
                .iter()
                .map(|item| ["- ", item, "\n"].concat())
                .collect();
            ["# ", list.0, "\n", &items, "\n"].concat()
        })
        .collect()
}

/// Per session in the ledger's `lines`: its earliest start mark (a reload re-fires it), its latest
/// end, encode and start marks (null when none), and whether it is interactive: true when a start
/// mark says so, else false when one says false, else null.
fn sessions(lines: &[Value]) -> BTreeMap<String, [Value; 5]> {
    let mut found: BTreeMap<String, [Value; 5]> = BTreeMap::new();
    for mark in lines {
        let (Some(session), Some(slot)) = (
            mark["session"].as_str(),
            ["start", "end", "encode"]
                .iter()
                .position(|kind| mark["kind"] == *kind),
        ) else {
            continue;
        };
        let held = found.entry(session.to_owned()).or_default();
        let later = |than: &Value| mark["at"].as_str() >= than["at"].as_str();
        if slot > 0 {
            if later(&held[slot]) {
                held[slot] = mark.clone();
            }
            continue;
        }
        if held[0].is_null() || !later(&held[0]) {
            held[0] = mark.clone();
        }
        if later(&held[3]) {
            held[3] = mark.clone();
        }
        if mark["interactive"].is_boolean() && held[4] != true {
            held[4] = mark["interactive"].clone();
        }
    }
    found
}

/// `$key` parsed, else `default`.
fn setting<T: FromStr>(key: &str, default: T) -> T {
    env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// SHA-256 (FIPS 180-4) of `text`, hex.
fn sha256(text: &str) -> String {
    const ROUND: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    let mut data = text.as_bytes().to_vec();
    let bits = u64::try_from(data.len())
        .unwrap_or_default()
        .wrapping_mul(8);
    data.push(0x80);
    while data.len() & 63 != 56 {
        data.push(0);
    }
    data.extend(bits.to_be_bytes());
    for block in data.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (slot, word) in w.iter_mut().zip(block.chunks_exact(4)) {
            *slot = u32::from_be_bytes(<[u8; 4]>::try_from(word).unwrap_or_default());
        }
        for i in 16..64usize {
            let (early, late) = (w[i.wrapping_sub(15)], w[i.wrapping_sub(2)]);
            w[i] = w[i.wrapping_sub(16)]
                .wrapping_add(
                    early.rotate_right(7) ^ early.rotate_right(18) ^ early.wrapping_shr(3),
                )
                .wrapping_add(w[i.wrapping_sub(7)])
                .wrapping_add(
                    late.rotate_right(17) ^ late.rotate_right(19) ^ late.wrapping_shr(10),
                );
        }
        let mut round = state;
        for (constant, word) in ROUND.iter().zip(w) {
            let [ra, rb, rc, rd, re, rf, rg, rh] = round;
            let one = rh
                .wrapping_add(re.rotate_right(6) ^ re.rotate_right(11) ^ re.rotate_right(25))
                .wrapping_add((re & rf) ^ (!re & rg))
                .wrapping_add(*constant)
                .wrapping_add(word);
            let two = (ra.rotate_right(2) ^ ra.rotate_right(13) ^ ra.rotate_right(22))
                .wrapping_add((ra & rb) ^ (ra & rc) ^ (rb & rc));
            round = [
                one.wrapping_add(two),
                ra,
                rb,
                rc,
                rd.wrapping_add(one),
                re,
                rf,
                rg,
            ];
        }
        for (held, add) in state.iter_mut().zip(round) {
            *held = held.wrapping_add(add);
        }
    }
    state.map(|word| format!("{word:08x}")).concat()
}

/// A bank's directory and its memories: a topic's (`@<topic>`) knowledge pages under
/// `knowledge/<topic>/`, else `memories/<bank>/`.
fn shelf(root: &Path, bank: &str) -> (PathBuf, Vec<Value>) {
    let (tier, at) = bank
        .strip_prefix('@')
        .map_or(("memories", bank), |topic| ("knowledge", topic));
    let held = notes(root, tier)
        .into_iter()
        .filter(|note| note["bank"] == bank)
        .collect();
    (root.join(tier).join(at), held)
}

/// A free `<dir>/<type>_<slug>.md` for a note named `name`, claimed: the bank format's slug
/// (lowercase, non-alphanumeric runs `_`, trimmed, 60 characters), `_2`, `_3`… on a clash.
fn slot(dir: &Path, kind: &str, name: &str, claimed: &mut BTreeSet<PathBuf>) -> PathBuf {
    let slug: String = name
        .split(|letter: char| !letter.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join("_")
        .to_ascii_lowercase()
        .chars()
        .take(60)
        .collect();
    let base = format!("{kind}_{slug}");
    free(
        dir,
        iter::once(format!("{base}.md")).chain((2u32..1000).map(|n| format!("{base}_{n}.md"))),
        claimed,
    )
}

/// `rows`, stably sorted by the string at `key` (a null first).
fn sorted(mut rows: Vec<Value>, key: &str) -> Vec<Value> {
    rows.sort_by(|left, right| left[key].as_str().cmp(&right[key].as_str()));
    rows
}

/// Consolidate's input for `bank` (AGENTS.md `### consolidate`): the traces whole, the memories
/// whole while their bytes fit `allowance`, the rest listed, then each of `more` that has items;
/// and the whole ones.
fn stack<'note>(
    bank: &str,
    traces: &[&Value],
    memories: &'note [Value],
    allowance: usize,
    more: &[(&str, Vec<String>)],
) -> (String, Vec<&'note Value>) {
    let mut used = 0usize;
    let (full, listed): (Vec<&Value>, Vec<&Value>) = memories.iter().partition(|memory| {
        used = used.saturating_add(text(&memory["text"]).len());
        used <= allowance
    });
    let block = |tag: &str, file: &Value| {
        format!(
            "<{tag} file=\"{}\">\n{}\n</{tag}>\n",
            text(&file["file"]),
            text(&file["text"]).trim_end()
        )
    };
    let mut input = format!(
        "# bank\n{bank}\n\n# traces\n{}",
        traces
            .iter()
            .map(|trace| block("trace", trace))
            .collect::<String>()
    );
    for (title, body) in [
        (
            "memories",
            full.iter()
                .map(|memory| block("memory", memory))
                .collect::<String>(),
        ),
        (
            "listed",
            listed
                .iter()
                .map(|memory| {
                    [
                        "- ",
                        text(&memory["file"]),
                        " \u{b7} ",
                        text(&memory["name"]),
                        " \u{b7} ",
                        text(&memory["description"]),
                        "\n",
                    ]
                    .concat()
                })
                .collect(),
        ),
    ]
    .into_iter()
    .chain(more.iter().map(|list| {
        (
            list.0,
            list.1
                .iter()
                .map(|item| ["- ", item, "\n"].concat())
                .collect(),
        )
    })) {
        if !body.is_empty() {
            input.extend(["\n# ", title, "\n", &body]);
        }
    }
    (input, full)
}

/// `.<file>.<pid>` beside `path`: a temp name every reader skips.
fn staged(path: &Path) -> PathBuf {
    path.with_file_name(format!(".{}.{}", name(path), id()))
}

/// A note's strength on day `now`, counted from the ledger's `lines`, never stored
/// (docs/index.html#cycle-strength): its used verdicts and shows, its wrong and stale verdicts since
/// its body last changed, decayed by the days since the latest of its dates, used verdicts and
/// shows (none: 0), to 3 places.
#[expect(clippy::float_arithmetic, reason = "strength is float math")]
fn strength(lines: &[Value], note: &Value, now: u64) -> f64 {
    let [used, shown, blame, half] = [
        ("SANDMAN_USED_WEIGHT", 3.0f64),
        ("SANDMAN_SHOWN_WEIGHT", 1.0f64),
        ("SANDMAN_JUDGED_WEIGHT", 4.0f64),
        ("SANDMAN_HALF_LIFE_DAYS", 30.0f64),
    ]
    .map(|(key, default)| setting(key, default));
    let about: Vec<&Value> = lines
        .iter()
        .filter(|line| {
            line["file"] == note["path"] || (note["id"].is_string() && line["id"] == note["id"])
        })
        .collect();
    let (read, acted) = (
        |line: &Value| line["kind"] == "show",
        |line: &Value| line["kind"] == "rethink" && line["verdict"] == "used",
    );
    let count = |items: usize| u32::try_from(items).map_or(f64::MAX, f64::from);
    let tally =
        |pick: &dyn Fn(&Value) -> bool| count(about.iter().filter(|line| pick(line)).count());
    let idle = about
        .iter()
        .filter(|line| read(line) || acted(line))
        .filter_map(|line| line["at"].as_str())
        .chain(
            [note["created"].as_str(), note["updated"].as_str()]
                .into_iter()
                .flatten(),
        )
        .max()
        .and_then(|at| day(at.get(..10)?))
        .map_or(0, |then| now.saturating_sub(then));
    let score = used.mul_add(
        tally(&acted),
        shown.mul_add(
            tally(&read),
            blame.mul_add(-count(judged(lines, note).len()), 1.0),
        ),
    ) * (-u32::try_from(idle).map_or(f64::MAX, f64::from) / half).exp2();
    (score * 1000.0f64).round() / 1000.0f64
}

/// The string `value` holds, else "".
fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

/// The local date, `YYYY-MM-DD`: `$SANDMAN_TODAY` (the suite's fixed day), else `date +%F`, in the
/// machine's time zone (`$TZ`, else `/etc/localtime`).
fn today() -> String {
    env::var("SANDMAN_TODAY").unwrap_or_else(|_| {
        Command::new("date")
            .arg("+%F")
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
            .unwrap_or_default()
    })
}

/// Recall's tokens: `text`'s lowercase alphanumeric runs, a one-character run or a `STOPWORDS` word
/// dropped, a final `s` dropped past four characters (a crude plural fold).
fn toks(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|letter: char| !letter.is_ascii_lowercase() && !letter.is_ascii_digit())
        .filter(|word| word.len() > 1 && !STOPWORDS.split(' ').any(|stop| stop == *word))
        .map(|word| {
            word.strip_suffix('s')
                .filter(|_| word.len() > 4)
                .unwrap_or(word)
                .to_owned()
        })
        .collect()
}

/// Where a note of `kind` files, `topic` given or not: a knowledge type's slug topic (lowercase
/// letters, digits and `-`, a letter or digit first), `""` for a memory type (its topic ignored);
/// none for another type, or a knowledge type without a slug.
fn topic<'slug>(kind: &str, topic: Option<&'slug str>) -> Option<&'slug str> {
    let slug = |letter: u8| letter.is_ascii_lowercase() || letter.is_ascii_digit();
    if KNOWN.contains(&kind) {
        topic.filter(|given| {
            given.bytes().next().is_some_and(slug)
                && given.bytes().all(|letter| slug(letter) || letter == b'-')
        })
    } else {
        KINDS.contains(&kind).then_some("")
    }
}

/// `values` summed as Python 3.12's `sum` does, with Neumaier's compensation, so a norm and a
/// cosine match the reference's to the bit.
#[expect(clippy::float_arithmetic, reason = "summation is float math")]
fn total<I: IntoIterator<Item = f64>>(values: I) -> f64 {
    let (sum, carry) = values
        .into_iter()
        .fold((0.0f64, 0.0f64), |(sum, carry), value| {
            let next = sum + value;
            let lost = if sum.abs() >= value.abs() {
                (sum - next) + value
            } else {
                (value - next) + sum
            };
            (next, carry + lost)
        });
    if carry != 0.0f64 && carry.is_finite() {
        sum + carry
    } else {
        sum
    }
}

/// When `session`'s live transcript last changed, when it has one.
fn touched(claude: &Path, session: &str) -> Option<SystemTime> {
    transcript(claude, session)?
        .metadata()
        .ok()?
        .modified()
        .ok()
}

/// `<claude>/projects/*/<session>.jsonl`, the first project's that holds one.
fn transcript(claude: &Path, session: &str) -> Option<PathBuf> {
    tree(&claude.join("projects"), 1)
        .into_iter()
        .map(|dir| dir.join(format!("{session}.jsonl")))
        .find(|path| path.is_file())
}

/// When a note last changed: its `updated`, else its `created`.
fn when(note: &Value) -> Option<&str> {
    note["updated"]
        .as_str()
        .or_else(|| note["created"].as_str())
}

/// Every path exactly `depth` levels under `path`, sorted; an unreadable level yields none.
fn tree(path: &Path, depth: usize) -> Vec<PathBuf> {
    let Some(next) = depth.checked_sub(1) else {
        return vec![path.to_owned()];
    };
    let mut kids: Vec<PathBuf> = fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .collect();
    kids.sort();
    kids.iter().flat_map(|kid| tree(kid, next)).collect()
}

/// `time` as RFC 3339 UTC to the second (Howard Hinnant's `civil_from_days`).
#[expect(
    clippy::arithmetic_side_effects,
    clippy::integer_division,
    clippy::integer_division_remainder_used,
    reason = "calendar math on unsigned seconds since the epoch: nothing goes negative or overflows"
)]
fn utc(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let days = secs / 86_400 + 719_468;
    let doe = days % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    format!(
        "{:04}-{month:02}-{:02}T{:02}:{:02}:{:02}Z",
        days / 146_097 * 400 + yoe + u64::from(month <= 2),
        doy - (153 * mp + 2) / 5 + 1,
        secs / 3600 % 24,
        secs / 60 % 60,
        secs % 60
    )
}

/// Whether `text` is a 36-character session id.
fn uuid(text: &str) -> bool {
    text.len() == 36
        && text
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
}

/// The embedding cache `<root>/embeddings.jsonl` by hash, the last line of each.
fn vectors(root: &Path) -> BTreeMap<String, Value> {
    jsonl(&root.join("embeddings.jsonl"))
        .into_iter()
        .map(|line| (text(&line["h"]).to_owned(), line))
        .collect()
}

/// The words of `lines` but `-`, sorted, once each: a note's `from` ids, its `sessions:`.
fn words<'line, I: IntoIterator<Item = &'line str>>(lines: I) -> BTreeSet<&'line str> {
    lines
        .into_iter()
        .flat_map(str::split_whitespace)
        .filter(|word| *word != "-")
        .collect()
}

fn main() -> ExitCode {
    let home = PathBuf::from(env::var_os("HOME").unwrap_or_default());
    let root = env::var_os("SANDMAN_ROOT").map_or_else(|| home.join(".sandman"), PathBuf::from);
    let claude =
        env::var_os("CLAUDE_CONFIG_DIR").map_or_else(|| home.join(".claude"), PathBuf::from);
    let args: Vec<String> = env::args().collect();
    // the name it was invoked as, so a command printed for an agent runs this binary
    let me = name(Path::new(args.first().map_or("", String::as_str)));
    let flag = |name: &str| args.iter().skip_while(|arg| *arg != name).nth(1).cloned();
    let pull = flag("--session");
    if !pull.as_deref().is_none_or(uuid) {
        eprintln!("sandman: --session takes a session id");
        return ExitCode::from(2);
    }
    let (verb, cued) = (
        args.get(1).map(String::as_str).unwrap_or_default(),
        flag("--cue"),
    );
    // mark, take and the seed (a recall without a cue) read one object on stdin
    let seed = verb == "recall" && cued.is_none();
    let reads = seed || ["mark", "take"].contains(&verb);
    let object: Map<String, Value> = reads
        .then(|| {
            let mut text = String::new();
            drop(io::stdin().read_to_string(&mut text));
            serde_json::from_str(&text).ok()
        })
        .flatten()
        .unwrap_or_default();
    let got = |key: &str| object.get(key).cloned().unwrap_or_default();
    let (asserted, why) = (got("session"), got("reason"));
    let sid = text(&asserted);
    let marking = verb == "mark";
    // a guard mark, a denied tool call, carries its rationale
    let given_kind = object.get("kind").map_or("", text);
    let fit = |kind: &str| {
        ["start", "end"].contains(&kind) || (kind == "guard" && got("rationale").is_string())
    };
    if reads && !(uuid(sid) && (!marking || fit(given_kind))) {
        let kinds = marking.then_some(r#""kind":"start"|"end"|"guard","#);
        eprintln!(
            r#"sandman {verb}: stdin is not {{{}"session":"<uuid>",...}}"#,
            kinds.unwrap_or_default()
        );
        return ExitCode::from(2);
    }
    // remember's and defer's `--about`: canonical when it is a directory
    let regarding = flag("--about").map(|given| {
        fs::canonicalize(&given)
            .ok()
            .filter(|dir| dir.is_dir())
            .map_or(given, |dir| dir.to_string_lossy().into_owned())
    });
    let pipeline = env::var("CLAUDE_MEMORY_PIPELINE").as_deref() == Ok("1");
    // a start, end or guard mark: mark's line; a start the seed's first act
    let marked = |kind: &str| {
        let mut line = if kind == "guard" {
            json!({
                "kind": kind, "rationale": one_line(text(&got("rationale")), 1000), "session": asserted,
                "tool": got("tool").as_str(),
            })
        } else {
            json!({
                "cwd": got("cwd"), "interactive": got("interactive"), "kind": kind,
                "reason": why, "resume": got("resume"), "session": asserted, "surface": got("surface"),
            })
        };
        record(&root, &mut line)
    };
    match verb {
        "mark" | "take" => {
            let skip =
                env::var_os("SANDMAN_NO_TAKE").is_some_and(|value| !value.is_empty()) || pipeline;
            let done = if marking {
                marked(given_kind)
            } else if why == "resume"
                || !ledger(&root)
                    .iter()
                    .any(|mark| mark["session"] == sid && mark["kind"] == "start")
            {
                Ok(())
            } else {
                let decided = skip.then(|| json!({"outcome": "skipped"})).or_else(|| {
                    job(&claude, sid)
                        .filter(|_| why != "clear")
                        .map(|job| json!({"job": job, "outcome": "held"}))
                });
                end(&root, &claude, sid, &got("cwd"), &why, decided).map(drop)
            };
            match done {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("sandman {verb}: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        "check" => {
            // `--fix` first repairs what is mechanical (AGENTS.md `### sandman check`), each note by
            // temp + rename, then says so in the ledger: an empty or `-` `from:` or `sessions:`
            // dropped, else its words sorted and once; an `origin:` none of `ORIGINS` dropped; a
            // minted `id:` first for a note without a valid one of its own (an earlier note's is
            // taken); a note left without a source given `origin:` by its stamp's first word
            let mend = |whole: &str, taken: &mut BTreeSet<String>| -> String {
                let (head, rest) = whole
                    .strip_prefix("---\n")
                    .and_then(|rest| rest.split_once("\n---"))
                    .map_or_else(
                        || ("", format!("\n\n{whole}")),
                        |(head, rest)| (head, rest.to_owned()),
                    );
                let mut lines: Vec<String> = head
                    .lines()
                    .filter_map(|line| {
                        let (key, value) = line.split_once(':').unwrap_or_default();
                        match key.trim() {
                            listed @ ("from" | "sessions") => {
                                let kept = words([value]);
                                (!kept.is_empty()).then(|| {
                                    format!("{listed}: {}", Vec::from_iter(kept).join(" "))
                                })
                            }
                            "origin" => ORIGINS.contains(&value.trim()).then(|| line.to_owned()),
                            _ => Some(line.to_owned()),
                        }
                    })
                    .collect();
                let joined = |kept: &[String]| {
                    format!(
                        "---\n{}---{rest}",
                        kept.iter()
                            .map(|line| [line, "\n"].concat())
                            .collect::<String>()
                    )
                };
                let note = read(json!({}), joined(&lines));
                let id = text(&note["id"]).to_owned();
                if !(hexid(&id) && taken.insert(id)) {
                    let minted = mint();
                    lines.retain(|line| {
                        line.split_once(':')
                            .is_none_or(|(key, _)| key.trim() != "id")
                    });
                    lines.insert(0, format!("id: {minted}"));
                    let _: bool = taken.insert(minted);
                }
                if lineage([&note]).is_empty() && note["from"].is_null() && note["origin"].is_null()
                {
                    let origin = match text(&note["source"]).split_whitespace().next() {
                        Some("remember") => "hand",
                        Some("consolidate" | "dream" | "reconsolidate" | "reflect") => "routine",
                        _ => "import",
                    };
                    let at = lines
                        .iter()
                        .position(|line| {
                            line.starts_with("sessions:") || line.starts_with("source:")
                        })
                        .unwrap_or(lines.len());
                    lines.insert(at, format!("origin: {origin}"));
                }
                joined(&lines)
            };
            let fix = || -> io::Result<()> {
                let (mut taken, mut fixed) = (BTreeSet::new(), 0u32);
                for note in every(&root) {
                    let (whole, mended) =
                        (text(&note["text"]), mend(text(&note["text"]), &mut taken));
                    if mended != whole {
                        put(&located(&root, &note), &mended)?;
                        fixed = fixed.saturating_add(1);
                    }
                }
                println!("check: fixed {fixed}");
                record(&root, &mut json!({"fixed": fixed, "kind": "check"}))
            };
            // the invariants, read only: per note in reader order, `<path>: <n>: <detail>`
            let audit = || -> io::Result<(usize, Vec<String>)> {
                let _: Option<()> = args
                    .iter()
                    .any(|arg| arg == "--fix")
                    .then(fix)
                    .transpose()?;
                let (all, mut ids, lines) = (every(&root), BTreeMap::new(), ledger(&root));
                let ends = ended(&lines);
                let over: BTreeSet<&str> =
                    ends.iter().filter_map(|end| end.2["id"].as_str()).collect();
                let mut found: Vec<String> = all
                    .iter()
                    .flat_map(|note| {
                        let (path, field) = (text(&note["path"]), |key: &str| {
                            note.get(key).and_then(Value::as_str)
                        });
                        let id = field("id").unwrap_or_default();
                        let twin = ids.get(id).copied();
                        if twin.is_none() {
                            let _: Option<&str> = ids.insert(id, path);
                        }
                        let listed = |key: &str| -> Vec<&str> {
                            field(key).unwrap_or_default().split_whitespace().collect()
                        };
                        let unsorted = |key: &str| {
                            listed(key)
                                .windows(2)
                                .any(|pair| matches!(pair, [left, right] if left >= right))
                                .then(|| format!("{key} is not sorted once"))
                        };
                        let origin = field("origin").filter(|value| !["", "-"].contains(value));
                        [
                            (1u8, field("id").is_none().then(|| "no id".to_owned())),
                            (
                                1,
                                (field("id").is_some() && !hexid(id))
                                    .then(|| format!("id {id} is not 12 lowercase hex")),
                            ),
                            (
                                1,
                                twin.filter(|_| hexid(id))
                                    .map(|first| format!("id {id} is also {first}'s")),
                            ),
                        ]
                        .into_iter()
                        .chain(["from", "origin", "sessions"].map(|key| {
                            let bare = field(key).filter(|value| ["", "-"].contains(value));
                            (2, bare.map(|value| format!("{key} is {value:?}")))
                        }))
                        .chain([
                            (
                                2,
                                origin
                                    .filter(|value| !ORIGINS.contains(value))
                                    .map(|value| {
                                        format!("origin {value} is not hand, import or routine")
                                    }),
                            ),
                            (
                                2,
                                (lineage([note]).is_empty()
                                    && words(field("from")).is_empty()
                                    && !origin.is_some_and(|value| ORIGINS.contains(&value)))
                                .then(|| "no source".to_owned()),
                            ),
                            (
                                3,
                                listed("from")
                                    .into_iter()
                                    .find(|word| !hexid(word))
                                    .map(|word| format!("from: {word} is not an id")),
                            ),
                            (3, unsorted("from")),
                            (
                                3,
                                listed("sessions")
                                    .into_iter()
                                    .find(|word| !uuid(word))
                                    .map(|word| format!("sessions: {word} is not a session id")),
                            ),
                            (3, unsorted("sessions")),
                            (
                                3,
                                field("strength")
                                    .map(|_| "strength is counted, never stored".to_owned()),
                            ),
                            (
                                4,
                                words(field("from"))
                                    .into_iter()
                                    .find(|parent| !over.contains(parent))
                                    .map(|parent| format!("from {parent} was never ended")),
                            ),
                            (4, over.contains(id).then(|| format!("id {id} was ended"))),
                        ])
                        .filter_map(|(rule, detail)| Some(format!("{path}: {rule}: {}", detail?)))
                        .collect::<Vec<_>>()
                    })
                    .collect();
                // then each rewrite line: its shape, an id ended once, its `to` still or later accounted for
                let active: BTreeSet<&str> =
                    all.iter().filter_map(|note| note["id"].as_str()).collect();
                let mut counted = BTreeSet::new();
                let twice: Vec<(usize, &str)> = ends
                    .iter()
                    .filter_map(|&(at, _, entry)| {
                        let id = text(&entry["id"]);
                        (!counted.insert(id)).then_some((at, id))
                    })
                    .collect();
                for (at, line) in lines
                    .iter()
                    .enumerate()
                    .filter(|pair| pair.1["kind"] == "rewrite")
                {
                    let (maybe, to) = (
                        |value: &Value| value.is_null() || value.is_string(),
                        &line["to"],
                    );
                    let shaped = line["from"].as_array().is_some_and(|entries| {
                        entries.iter().all(|entry| {
                            entry["digest"].is_string()
                                && maybe(&entry["id"])
                                && entry["path"].is_string()
                                && entry["sessions"]
                                    .as_array()
                                    .is_some_and(|held| held.iter().all(Value::is_string))
                        })
                    }) && OPS.contains(&text(&line["op"]))
                        && maybe(&line["model"])
                        && maybe(&line["reason"])
                        && (to.is_null()
                            || (to["id"].is_string() && to["path"].is_string())
                            || to["artifact"].is_string());
                    let lost = to["id"].as_str().filter(|into| {
                        !active.contains(into)
                            && !ends
                                .iter()
                                .any(|&(later, _, entry)| later > at && entry["id"] == *into)
                    });
                    found.extend(
                        [
                            (!shaped).then(|| "not a rewrite line".to_owned()),
                            lost.map(|into| format!("to {into} is neither active nor ended later")),
                        ]
                        .into_iter()
                        .chain(
                            twice
                                .iter()
                                .filter(|pair| pair.0 == at)
                                .map(|pair| Some(format!("id {} ended again", pair.1))),
                        )
                        .flatten()
                        .map(|detail| format!("ledger {}: 5: {detail}", text(&line["at"]))),
                    );
                }
                Ok((all.len(), found))
            };
            audit().map_or_else(
                |error| {
                    eprintln!("sandman check: {error}");
                    ExitCode::FAILURE
                },
                |(count, found)| {
                    if found.is_empty() {
                        println!("check: {count} notes: clean");
                    } else {
                        println!("{}", found.join("\n"));
                    }
                    ExitCode::from(u8::from(!found.is_empty()))
                },
            )
        }
        "chart" => {
            let (lines, now) = (ledger(&root), day(&today()).unwrap_or_default());
            // a memory or a knowledge page, the page's topic in place of a bank
            let shelved = |dir: &str| -> Vec<Value> {
                notes(&root, dir)
                .iter()
                .map(|note| {
                    let source = text(&note["source"]);
                    let kind = source
                        .split_whitespace()
                        .next()
                        .filter(|word| ["consolidate", "dream", "reflect", "remember"].contains(word))
                        .unwrap_or("other");
                    let mut row = json!({
                        "created": note["created"], "file": note["file"], "from": words([text(&note["from"])]),
                        "id": note["id"], "name": note["name"], "origin": note["origin"],
                        "session": source.split_whitespace().find(|word| ["consolidate", "dream"].contains(&kind) && uuid(word)),
                        "sessions": lineage([note]), "source": kind, "state": "active", "strength": strength(&lines, note, now), "type": text(&note["type"]),
                    });
                    let bank = text(&note["bank"]);
                    match bank.strip_prefix('@') {
                        Some(topic) => row["topic"] = topic.into(),
                        None => row["bank"] = bank.into(),
                    }
                    row
                })
                .collect()
            };
            let (knowledge, memories) = (shelved("knowledge"), shelved("memories"));
            let traces: Vec<Value> = notes(&root, "traces")
                .iter()
                .map(|note| {
                    json!({
                        "bank": note["bank"], "created": note["created"], "file": note["file"], "from": words([text(&note["from"])]),
                        "id": note["id"], "name": note["name"], "origin": note["origin"], "session": note["session"],
                        "sessions": lineage([note]), "state": "active", "strength": strength(&lines, note, now), "type": text(&note["type"]),
                    })
                })
                .collect();
            let mut taken: BTreeMap<String, Value> = BTreeMap::new();
            for path in tree(&root.join(".archive/claude"), 4) {
                let Some((head, session)) = name(&path)
                    .strip_suffix(".jsonl")
                    .and_then(|stem| stem.split_at_checked(stem.len().checked_sub(36)?))
                else {
                    continue;
                };
                let day: Vec<&str> = path.ancestors().skip(1).take(3).map(name).collect();
                let (&[dd, mm, yyyy], true) = (
                    day.as_slice(),
                    head.len() > 7
                        && head.ends_with('-')
                        && head.get(6..7) == Some("-")
                        && head.bytes().take(6).all(|byte| byte.is_ascii_digit())
                        && uuid(session)
                        && path.is_file(),
                ) else {
                    continue;
                };
                let pointer =
                    fs::read_to_string(root.join(format!("memories/.recent/{session}.json")))
                        .ok()
                        .and_then(|text| serde_json::from_str(&text).ok())
                        .filter(Value::is_object)
                        .unwrap_or_default();
                let state = if pointer.is_null() {
                    "archived"
                } else if pointer["dreamed"].is_string() {
                    "dreamed"
                } else {
                    "waiting"
                };
                let clock =
                    [head.get(..2), head.get(2..4), head.get(4..6)].map(Option::unwrap_or_default);
                drop(taken.insert(session.to_owned(), json!({
                    "bytes": fs::metadata(&path).map_or(0, |meta| meta.len()), "cwd": pointer["cwd"],
                    "dreamed": pointer["dreamed"],
                    "highlights": pointer["highlights"].as_array().map_or(0, Vec::len),
                    "memories": memories.iter().chain(&knowledge).filter(|row| row["sessions"].as_array().into_iter().flatten().any(|id| *id == session)).count(),
                    "session": session, "state": state,
                    "taken": format!("{yyyy}-{mm}-{dd}T{}Z", clock.join(":")), "title": pointer["title"],
                })));
            }
            // the notes rewrite lines ended, and each edge from a parent to the note it went into
            let ends = ended(&lines);
            let gone: Vec<Value> = ends
                .iter()
                .map(|&(_, line, entry)| json!({"at": line["at"], "id": entry["id"], "into": line["to"]["id"], "op": line["op"], "path": entry["path"], "sessions": entry["sessions"], "state": "ended"}))
                .collect();
            let edges: Vec<Value> = ends
                .iter()
                .filter(|end| end.1["to"]["id"].is_string())
                .map(|&(_, line, entry)| json!({"at": line["at"], "from": entry["id"], "op": line["op"], "to": line["to"]["id"]}))
                .collect();
            let guards = lines
                .iter()
                .filter(|line| line["kind"] == "guard")
                .map(|line| json!({"at": line["at"], "rationale": line["rationale"], "session": line["session"], "tool": line["tool"]}))
                .collect();
            let active = sessions(&lines)
                .into_iter()
                .map(|(session, [start, end, encode, last, interactive])| {
                    let from = if start.is_null() { &end } else { &start };
                    // an end before the latest start was resumed past: running again
                    let ended = |field: &str| {
                        end[field]
                            .as_str()
                            .filter(|_| last["at"].as_str() <= end["at"].as_str())
                    };
                    json!({
                        "cwd": from["cwd"], "encoded": encode["outcome"], "end": ended("at"), "interactive": interactive,
                        "machine": from["machine"], "outcome": ended("outcome"),
                        "seen": touched(&claude, &session).map(utc), "session": session, "start": start["at"],
                    })
                })
                .collect();
            let intended = intentions(&root)
                .iter()
                .map(|held| {
                    json!({
                        "about": held["about"], "after": held["after"], "closed": held["closed"], "created": held["created"],
                        "due": held["due"], "id": held["id"], "session": held["session"], "state": held["state"], "what": held["body"],
                    })
                })
                .collect();
            // a closed stdout (`| head`) ends the chart quietly
            drop(writeln!(
                io::stdout().lock(),
                "{}",
                json!({
                    "active": sorted(active, "start"), "at": utc(SystemTime::now()), "ended": sorted(gone, "at"), "guards": sorted(guards, "at"),
                    "intentions": sorted(intended, "created"),
                    "knowledge": knowledge, "lineage": sorted(edges, "at"),
                    "machine": machine(&root), "memories": memories, "taken": sorted(taken.into_values().collect(), "taken"), "traces": traces,
                })
            ));
            ExitCode::SUCCESS
        }
        "defer" | "done" | "drop" => {
            // prospective memory: an intention written, pushed or closed; nothing acts on one
            let change = || -> Result<String, (u8, String)> {
                let deferring = verb == "defer";
                let given = if deferring {
                    flag("--id")
                } else {
                    Some(args.get(2).cloned().unwrap_or_default())
                };
                let what = args
                    .get(2)
                    .filter(|said| !said.starts_with("--"))
                    .map_or("", |said| said.trim());
                // a time is a date from the local today, a weekday the next one after it
                // (1970-01-01 was a Thursday); any other words are an event's
                let when = flag("--when").and_then(|asked| {
                    let (spoken, now) = (
                        asked.trim().to_lowercase(),
                        day(&today()).unwrap_or_default(),
                    );
                    let ahead = match spoken.as_str() {
                        "" => return None,
                        "today" => Some(0),
                        "tomorrow" => Some(1),
                        _ => "sunday monday tuesday wednesday thursday friday saturday"
                            .split(' ')
                            .position(|name| name == spoken)
                            .map(|at| {
                                u64::try_from(at)
                                    .unwrap_or_default()
                                    .saturating_add(13)
                                    .saturating_sub(now.saturating_add(4).rem_euclid(7))
                                    .rem_euclid(7)
                                    .saturating_add(1)
                            })
                            .or_else(|| {
                                let count = spoken.strip_prefix('+')?;
                                let (span, per) = count
                                    .strip_suffix('d')
                                    .map(|days| (days, 1))
                                    .or_else(|| Some((count.strip_suffix('w')?, 7)))?;
                                Some(u64::from(span.parse::<u16>().ok()?).saturating_mul(per))
                            }),
                    };
                    Some(
                        ahead
                            .map(|days| now.saturating_add(days))
                            .or_else(|| day(&spoken))
                            .map_or_else(
                                || ("after", one_line(&asked, 400)),
                                |at| ("due", date(at)),
                            ),
                    )
                });
                if deferring && (when.is_none() || (given.is_none() && what.is_empty())) {
                    return Err((
                        2,
                        "sandman defer: takes \"<what>\" first, or --id <id>, and --when today|tomorrow|<weekday>|YYYY-MM-DD|+<n>d|+<n>w|<event>".to_owned(),
                    ));
                }
                let hits: Vec<Value> = intentions(&root)
                    .into_iter()
                    .filter(|held| {
                        given.as_deref().is_some_and(|asked| {
                            !asked.is_empty() && text(&held["id"]).starts_with(asked)
                        })
                    })
                    .collect();
                let found = hits.first().filter(|_| hits.len() == 1);
                if let Some(asked) = given.as_deref().filter(|_| found.is_none()) {
                    return Err((
                        1,
                        format!(
                            "sandman {verb}: {asked} is not one intention's id ({} match)",
                            hits.len()
                        ),
                    ));
                }
                let (now, (field, value), noted) = (
                    utc(SystemTime::now()),
                    when.unwrap_or_default(),
                    flag("--note").unwrap_or_default(),
                );
                let id = found.map_or_else(
                    || random().get(..8).unwrap_or_default().to_owned(),
                    |hit| text(&hit["id"]).to_owned(),
                );
                // the store's fields in their order: a push files it anew and reopens it, a close
                // stamps it
                let held = found.cloned().unwrap_or_else(
                    || json!({"about": regarding, "created": now, "session": pull}),
                );
                let mut fields = Vec::from(
                    [
                        "due", "after", "created", "about", "session", "graduate", "artifact",
                        "state", "closed", "note",
                    ]
                    .map(|key| (key, text(&held[key]))),
                );
                let state = ["open", "done", "dropped"]
                    [usize::from(!deferring).saturating_add(usize::from(verb == "drop"))];
                let other = ["due", "after"][usize::from(field == "due")];
                let closed = ["", now.as_str()][usize::from(!deferring)];
                let changes = [
                    (field, value.as_str()),
                    (other, ""),
                    ("state", state),
                    ("closed", closed),
                    ("note", noted.as_str()),
                ];
                amend(
                    &mut fields,
                    changes
                        .get([0, 2][usize::from(!deferring)]..)
                        .unwrap_or_default(),
                );
                let failed = |error: io::Error| (1, format!("sandman {verb}: {error}"));
                // done on a graduation's proposal first ends its note, into the artifact
                let graduated = found
                    .filter(|_| verb == "done")
                    .and_then(|hit| {
                        let wanted = hit["graduate"].as_str()?;
                        let ending = every(&root).into_iter().find(|one| one["id"] == wanted)?;
                        let artifact = text(&hit["artifact"]);
                        let to = json!({"artifact": artifact});
                        Some(
                            retire(&root, &ending, "graduate", &Value::Null, &to).map(|()| {
                                format!(
                                    "\ngraduated: {} \u{2192} {artifact}",
                                    text(&ending["path"])
                                )
                            }),
                        )
                    })
                    .transpose()
                    .map_err(failed)?
                    .unwrap_or_default();
                put(
                    &root.join(format!("intentions/{id}.md")),
                    &note(&fields, found.map_or(what, |hit| text(&hit["body"]))),
                )
                .map_err(failed)?;
                let kind = ["push", verb][usize::from(!deferring || found.is_none())];
                let mut line = json!({"id": id, "kind": kind, "session": pull});
                for (key, said) in [(field, &value), ("note", &noted)] {
                    if !said.is_empty() {
                        line[key] = said.as_str().into();
                    }
                }
                drop(record(&root, &mut line));
                Ok(match (deferring, field) {
                    (false, _) => format!("{id} {state}{graduated}"),
                    (true, "due") => format!("{id} {value}"),
                    _ => format!("{id} waiting: {value}"),
                })
            };
            change().map_or_else(
                |(code, error)| {
                    eprintln!("{error}");
                    ExitCode::from(code)
                },
                |printed| {
                    println!("{printed}");
                    ExitCode::SUCCESS
                },
            )
        }
        "due" => {
            let (_, _, label) = labels(&home, &claude);
            let all =
                args.iter().any(|arg| arg == "--all") && flag("--all").as_deref() != Some("false");
            let shown = agenda(&root, all, usize::MAX, me, label);
            print!(
                "{}",
                Some(shown)
                    .filter(|listed| !listed.is_empty())
                    .unwrap_or_else(|| "nothing due\n".to_owned())
            );
            ExitCode::SUCCESS
        }
        "forget" => {
            // erased for real, unlike everything else here: no `.pruned/` copy, and no line naming it
            let session = args.get(2).map_or("", String::as_str);
            let erase = || -> io::Result<[usize; 2]> {
                let mut done = [0usize; 2];
                // a JSONL file, each line as `keep` leaves it (none: dropped), rewritten under an
                // exclusive `flock`: an appender waiting on it finds the file replaced and opens the
                // new one; 1 when it was
                let purge =
                    |path: &Path, keep: &dyn Fn(&str) -> Option<String>| -> io::Result<usize> {
                        let file = match File::open(path) {
                            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
                            opened => opened?,
                        };
                        file.lock()?;
                        let whole = io::read_to_string(&file)?;
                        let kept: String = whole.split_inclusive('\n').filter_map(keep).collect();
                        let changed = kept != whole;
                        if changed {
                            put(path, &kept)?;
                        }
                        Ok(usize::from(changed))
                    };
                let lines = ledger(&root);
                let unnamed = |line: &str| (!line.contains(session)).then(|| line.to_owned());
                // a rewrite line stays, the session off its `from[].sessions`, so the notes it ended
                // still hold invariants 4–5; any other line naming it goes, and so does one still
                // naming it once rewritten
                let ledgered = |line: &str| {
                    unnamed(line).or_else(|| {
                        let mut row = serde_json::from_str::<Value>(line)
                            .ok()
                            .filter(|row| row["kind"] == "rewrite")?;
                        for held in row["from"]
                            .as_array_mut()
                            .into_iter()
                            .flatten()
                            .filter_map(|entry| entry["sessions"].as_array_mut())
                        {
                            held.retain(|id| id != session);
                        }
                        unnamed(&format!("{row}\n"))
                    })
                };
                // first, so a take, a reconcile or an encode racing this finds no start mark to act on
                for day in tree(&root.join("ledger"), 3) {
                    done[1] = done[1].saturating_add(purge(&day, &ledgered)?);
                }
                // encode's run files hold it when, of every encode line and its own ends, the latest
                // names it
                let encoding = lines
                    .iter()
                    .rev()
                    .find(|line| {
                        line["kind"] == "encode"
                            || (line["kind"] == "end" && line["session"] == session)
                    })
                    .is_some_and(|line| line["session"] == session);
                let naming =
                    |path: &Path| fs::read_to_string(path).is_ok_and(|held| held.contains(session));
                let mut gone: Vec<PathBuf> = tree(&claude.join("projects"), 1)
                    .iter()
                    .flat_map(|dir| [dir.join(format!("{session}.jsonl")), dir.join(session)])
                    .chain(
                        tree(&root.join(".archive/claude"), 4)
                            .into_iter()
                            .filter(|path| name(path).contains(session)),
                    )
                    .chain([root.join(format!("memories/.recent/{session}.json"))])
                    .chain(tree(&root.join("run"), 1).into_iter().filter(|path| {
                        [".in", ".out", ".err"]
                            .iter()
                            .any(|ext| name(path).ends_with(ext))
                            && ((encoding && name(path).starts_with("encode.")) || naming(path))
                    }))
                    .collect();
                // a note or a pruned copy whose `session:` is it goes, the recall paths kept for the
                // embedding cache; one naming it otherwise (a merge's source, an intention's link) is
                // rewritten without it
                let (mut paths, mut scrub) = (BTreeSet::new(), Vec::new());
                for note in every(&root) {
                    let file = located(&root, &note);
                    match (
                        note["session"] == session,
                        text(&note["text"]).contains(session),
                    ) {
                        (true, _) => {
                            let _: bool = paths.insert(text(&note["path"]).to_owned());
                            gone.push(file);
                        }
                        (false, true) => scrub.push(file),
                        (false, false) => {}
                    }
                }
                let pruned = tree(&root.join(".pruned"), 4);
                for path in &pruned {
                    let held = fs::read_to_string(path).unwrap_or_default();
                    match (
                        front(&held).contains(&("session", session)),
                        held.contains(session),
                    ) {
                        (true, _) => gone.push(path.clone()),
                        (false, true) => scrub.push(path.clone()),
                        (false, false) => {}
                    }
                }
                scrub.extend(
                    tree(&root.join("intentions"), 1)
                        .into_iter()
                        .filter(|path| naming(path)),
                );
                for path in &gone {
                    let Ok(meta) = fs::symlink_metadata(path) else {
                        continue;
                    };
                    if meta.is_dir() {
                        fs::remove_dir_all(path)?;
                    } else {
                        fs::remove_file(path)?;
                    }
                    done[0] = done[0].saturating_add(1);
                }
                // a reply's ops on a trace whose copy went, gone with it
                for path in pruned
                    .iter()
                    .filter(|path| name(path).ends_with("-ops.json"))
                {
                    let stem = name(path).trim_end_matches("ops.json");
                    let mut reply: Value =
                        serde_json::from_str(&fs::read_to_string(path)?).unwrap_or_default();
                    let ops: Vec<Value> = reply["ops"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|op| {
                            !gone.contains(
                                &path.with_file_name(format!("{stem}{}", text(&op["trace"]))),
                            )
                        })
                        .cloned()
                        .collect();
                    if ops.len() < reply["ops"].as_array().map_or(0, Vec::len) {
                        reply["ops"] = ops.into();
                        put(path, &reply.to_string())?;
                        done[1] = done[1].saturating_add(1);
                    }
                }
                for path in &scrub {
                    let held = fs::read_to_string(path)?;
                    // off a `sessions:` line, the line gone once empty; `-` in a stamp
                    let scrubbed: String = held
                        .replace(&format!("session: {session}\n"), "")
                        .split_inclusive('\n')
                        .filter_map(|line| {
                            line.strip_prefix("sessions: ").map_or_else(
                                || Some(line.to_owned()),
                                |ids| {
                                    let kept: Vec<&str> = ids
                                        .split_whitespace()
                                        .filter(|id| *id != session)
                                        .collect();
                                    (!kept.is_empty())
                                        .then(|| format!("sessions: {}\n", kept.join(" ")))
                                },
                            )
                        })
                        .collect();
                    put(path, &scrubbed.replace(session, "-"))?;
                    done[1] = done[1].saturating_add(1);
                }
                let cached = |line: &str| {
                    (!serde_json::from_str::<Value>(line)
                        .is_ok_and(|row| paths.contains(text(&row["path"]))))
                    .then(|| line.to_owned())
                };
                done[1] = done[1].saturating_add(purge(&root.join("embeddings.jsonl"), &cached)?);
                for log in ["eval/recall.jsonl", "run/serve.log", "run/serve.log.1"] {
                    done[1] = done[1].saturating_add(purge(&root.join(log), &unnamed)?);
                }
                Ok(done)
            };
            let erased = Some(session)
                .filter(|given| uuid(given))
                .ok_or_else(|| (2, "sandman forget: takes a session id".to_owned()))
                .and_then(|_| erase().map_err(|error| (1, format!("sandman forget: {error}"))));
            erased.map_or_else(
                |(code, error)| {
                    eprintln!("{error}");
                    ExitCode::from(code)
                },
                |[removed, rewritten]| {
                    if removed == 0 && rewritten == 0 {
                        println!("nothing to forget");
                    } else {
                        println!("forgot: {removed} removed, {rewritten} rewritten");
                    }
                    ExitCode::SUCCESS
                },
            )
        }
        "mcp" => {
            // the calling session: Claude Code's, in every stdio server's env, else this process's
            let session = env::var("CLAUDE_CODE_SESSION_ID")
                .ok()
                .filter(|given| uuid(given))
                .unwrap_or_else(|| {
                    random()
                        .chars()
                        .enumerate()
                        .flat_map(|(at, digit)| {
                            [8, 12, 16, 20]
                                .contains(&at)
                                .then_some('-')
                                .into_iter()
                                .chain([digit])
                        })
                        .collect()
                });
            let string = |about: &str| json!({"description": about, "type": "string"});
            let sessioned = string("this session's id, as the seed gives it; else the server's");
            let closing = |named: &str, how: &str| {
                json!({"description": format!("Close a deferred intention {how}, by the id `due` or the seed gives."),
                 "inputSchema": {"properties": {"id": string("the intention's id, or its start"), "note": string("a line on how it ended"), "session": sessioned}, "required": ["id"], "type": "object"},
                 "name": named})
            };
            let tools = json!([
                {"description": "Defer work at once when the person puts it off (later, tomorrow, after X), in their words: it is surfaced to the session it comes due in, never started on its own. With `id`, push that one to a new when.",
                 "inputSchema": {"properties": {"about": string("the repository path, when it is in one"), "id": string("an intention's id, to push it to a new when"), "session": sessioned, "what": string("the person's words"), "when": string("today, tomorrow, a weekday, YYYY-MM-DD, +3d, +2w, or the event's words: after grove ships")}, "required": ["what", "when"], "type": "object"},
                 "name": "defer"},
                closing("done", "once it is finished"),
                closing("drop", "the person no longer wants"),
                {"description": "List deferred work that is due, and every one waiting on an event; `all` adds the upcoming. Raise it with the person; never start it unasked.",
                 "inputSchema": {"properties": {"all": {"description": "add the upcoming ones", "type": "boolean"}, "session": sessioned}, "required": [], "type": "object"},
                 "name": "due"},
                {"description": "Pull memory from earlier sessions before work in a repository, before a choice the person may already have ruled on, and when a correction may already be recorded; not every turn. Each line is a pointer: read it with `show` before acting on it; a `~` marks one remembered but not yet consolidated.",
                 "inputSchema": {"properties": {"about": string("the repository path, when the work is in one"), "budget": {"description": "characters, 4000 by default", "type": "number"}, "cue": string("what you are about to do"), "session": sessioned}, "required": ["cue"], "type": "object"},
                 "name": "recall"},
                {"description": "Write a memory at once when the person states a preference, corrects you, or decides something a later session needs: their words, one fact per call. Knowledge true beyond this project is type `knowledge`, the steps of a task done more than once type `playbook`, each with a topic.",
                 "inputSchema": {"properties": {"about": string("the repository path, when it holds in one repository"), "body": string("the person's words"), "session": sessioned, "topic": string("a knowledge or playbook's topic, a slug: launchd, llama-cpp"), "type": string("user, feedback, project or reference (by repository), knowledge or playbook (by topic); feedback by default, knowledge with a topic")}, "required": ["body"], "type": "object"},
                 "name": "remember"},
                {"description": "Judge a note a recall sent, once you know: `used` when you acted on it, `wrong` or `stale` when it is untrue or out of date, `irrelevant` when it was not about what you were doing.",
                 "inputSchema": {"properties": {"path": string("the path the recall line gave"), "session": sessioned, "verdict": string("used, wrong, stale or irrelevant"), "why": string("a line on why")}, "required": ["path", "verdict"], "type": "object"},
                 "name": "rethink"},
                {"description": "Read one memory in full, by the path a recall line gives, before acting on it.",
                 "inputSchema": {"properties": {"path": string("the path a recall line gives"), "session": sessioned}, "required": ["path"], "type": "object"},
                 "name": "show"},
            ]);
            let exe = env::current_exe().unwrap_or_default();
            // a reply per request, none for a notification
            let respond = |said: String| {
                let request: Value = serde_json::from_str(&said).unwrap_or_default();
                let (params, known) = (
                    &request["params"],
                    ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"],
                );
                let called = text(&params["name"]);
                let answer = match request["method"].as_str() {
                    _ if !request.is_object() => Err((-32_700i32, "Parse error".to_owned())),
                    // a notification: nothing to answer
                    _ if request.get("id").is_none() => return None,
                    Some("initialize") => {
                        let asked = text(&params["protocolVersion"]);
                        Ok(json!({
                            "capabilities": {"tools": {}},
                            "protocolVersion": known.into_iter().find(|version| *version == asked).unwrap_or(known[0]),
                            "serverInfo": {"name": "sandman", "version": env!("CARGO_PKG_VERSION")},
                        }))
                    }
                    Some("ping") => Ok(json!({})),
                    Some("tools/list") => Ok(json!({"tools": tools})),
                    Some("tools/call") => tools
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|tool| tool["name"] == called)
                        .ok_or_else(|| (-32_602i32, format!("Unknown tool: {called}")))
                        .map(|tool| {
                            // the verb's own argv: the required argument positional (recall's
                            // `--cue`), every other given one a flag, the session always
                            let schema = &tool["inputSchema"];
                            let required = text(&schema["required"][0]);
                            let mut line = vec![called.to_owned()];
                            let keys = schema["properties"].as_object().into_iter().flat_map(Map::keys);
                            for key in iter::once(required).chain(keys.map(String::as_str).filter(|key| *key != required)) {
                                // a verdict is its own flag: `--used`
                                let value = params["arguments"]
                                    .get(key)
                                    .map(|given| given.as_str().map_or_else(|| given.to_string(), str::to_owned))
                                    .map(|given| if key == "verdict" { format!("--{given}") } else { given })
                                    .or_else(|| (key == required).then(String::new))
                                    .or_else(|| (key == "session").then(|| session.clone()));
                                let named = ((key != required && key != "verdict") || called == "recall").then(|| format!("--{key}"));
                                line.extend(value.into_iter().flat_map(|given| named.clone().into_iter().chain([given])));
                            }
                            let out = Command::new(&exe).args(&line).stdin(Stdio::null()).output().ok();
                            let failed = !out.as_ref().is_some_and(|ran| ran.status.success());
                            let shown = out.map(|ran| if failed { ran.stderr } else { ran.stdout });
                            json!({
                                "content": [{"text": String::from_utf8_lossy(&shown.unwrap_or_default()), "type": "text"}],
                                "isError": failed,
                            })
                        }),
                    _ => Err((-32_601i32, "Method not found".to_owned())),
                };
                let mut reply = answer.map_or_else(
                    |(code, message)| json!({"error": {"code": code, "message": message}}),
                    |result| json!({"result": result}),
                );
                (reply["id"], reply["jsonrpc"]) = (request["id"].clone(), "2.0".into());
                Some(reply)
            };
            for reply in io::stdin()
                .lock()
                .lines()
                .map_while(Result::ok)
                .filter_map(respond)
            {
                drop(writeln!(io::stdout().lock(), "{reply}"));
            }
            ExitCode::SUCCESS
        }
        "recall" => {
            let session = seed.then(|| sid.to_owned()).or_else(|| pull.clone());
            let cue = cued.unwrap_or_else(|| text(&got("prompt")).to_owned());
            // first, so a slow or failed seed never loses the start; a headless run is never marked
            drop((seed && got("interactive") == true).then(|| marked("start")));
            if seed && pipeline {
                return ExitCode::SUCCESS;
            }
            let (budget, millis) = if seed {
                (
                    setting("SANDMAN_SEED_BUDGET", 9000),
                    setting("SANDMAN_SEED_MS", 4000),
                )
            } else {
                (
                    flag("--budget")
                        .and_then(|chars| chars.parse().ok())
                        .unwrap_or(4000),
                    setting("SANDMAN_RECALL_MS", 10_000),
                )
            };
            // the bound covers the whole pass: every embedding and every scorer tried
            let (begun, limit) = (Instant::now(), Duration::from_millis(millis));
            // the always-set comes without a vote and never leaves the machine; muted, nothing
            let (always, candidates) = pool(&root);
            let path = |memory: &Value| text(&memory["path"]).to_owned();
            // what this session's earlier recalls sent is not sent again
            let earlier: Vec<Value> = ledger(&root)
                .into_iter()
                .filter(|line| {
                    line["kind"] == "recall"
                        && session.as_deref().is_some_and(|id| line["session"] == id)
                })
                .collect();
            let before: BTreeSet<&str> = earlier
                .iter()
                .flat_map(|line| line["sent"].as_array().into_iter().flatten())
                .filter_map(|sent| sent["file"].as_str())
                .collect();
            let (grove, repos, label) = labels(&home, &claude);
            // the repository the work is in: the pull's `--about`, the seed's cwd when it is
            // inside a repository under the grove root
            let given = flag("--about")
                .map(PathBuf::from)
                .or_else(|| {
                    let cwd = PathBuf::from(text(&got("cwd")));
                    cwd.strip_prefix(&grove)
                        .is_ok_and(|rest| rest.components().count() > 1)
                        .then_some(cwd)
                })
                .map(|dir| label(&key(&project(&dir))));
            let state = given
                .as_ref()
                .map_or_else(|| cue.clone(), |repo| format!("repository: {repo}\n{cue}"));
            // else the first repository, longest key first, whose name the cue says
            let lower = cue.to_lowercase();
            let spoken: BTreeSet<&str> = lower
                .split(|letter: char| !letter.is_ascii_alphanumeric() && letter != '.')
                .collect();
            let about = given.clone().or_else(|| {
                repos.iter().find_map(|repo| {
                    let (_, named) = repo.1.split_once('/')?;
                    spoken
                        .contains(named.to_lowercase().as_str())
                        .then(|| repo.1.clone())
                })
            });
            // the gate, free: a cue of steering words alone asks for nothing and calls no model
            let ask = toks(&cue)
                .iter()
                .any(|word| !STEER.split(' ').any(|steer| steer == word));
            let secret = env::var("TYPESAFE_API_KEY")
                .or_else(|_| fs::read_to_string(home.join(".config/typesafe/api_key")))
                .map(|key| key.trim().to_owned())
                .unwrap_or_default();
            let now = || {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |since| {
                        u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
                    })
            };
            let (down, tried) = (
                root.join("jev-down.json"),
                setting("SANDMAN_SCORERS", "pplx,kev,jev".to_owned()),
            );
            // the pass, the reference's (docs/recall.html#proposed): BM25 and cosine ranks fused,
            // the fused top 10 to one decision model, the two orders fused again and cut; each
            // line `(its score, its candidate)`, at most so many, the path taken, the scorer, the
            // embeddings' part, the tokens spent and what failed
            #[expect(
                clippy::float_arithmetic,
                reason = "BM25, cosine and reciprocal-rank fusion are float math"
            )]
            let pass = || {
                let mut errors: Vec<(&str, String)> = Vec::new();
                let count = |items: usize| f64::from(u32::try_from(items).unwrap_or(u32::MAX));
                let reciprocal = |rank: usize| 1.0f64 / count(rank.saturating_add(61));
                // descending, a tie in the order given
                let order = |scores: &[f64], mut among: Vec<usize>| {
                    among.sort_by(|left, right| {
                        scores[*right]
                            .partial_cmp(&scores[*left])
                            .unwrap_or(Ordering::Equal)
                    });
                    among
                };
                let labelled: Vec<String> = candidates
                    .iter()
                    .map(|memory| label(text(&memory["bank"])))
                    .collect();
                // BM25 (k1 1.2, b .75) of the head, the body and the file's stem for the cue's
                // distinct tokens and the given repository's, plus 4 in the about's bank, else 1
                // in home's or the grove root's, unless the memory claims its own importance
                let docs: Vec<Vec<String>> = candidates
                    .iter()
                    .zip(&labelled)
                    .map(|(memory, repo)| {
                        let file = text(&memory["file"]);
                        toks(&format!(
                            "{}\n{} {}",
                            head(memory, repo),
                            text(&memory["body"]),
                            file.strip_suffix(".md").unwrap_or(file)
                        ))
                    })
                    .collect();
                let size = count(docs.len());
                let mean = count(docs.iter().map(Vec::len).sum()) / size;
                let idf: Vec<(String, f64)> =
                    toks(&format!("{cue} {}", given.as_deref().unwrap_or_default()))
                        .into_iter()
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .map(|word| {
                            let df = count(docs.iter().filter(|doc| doc.contains(&word)).count());
                            let weight = ((size - df + 0.5) / (df + 0.5)).ln_1p();
                            (word, weight)
                        })
                        .collect();
                let lexical: Vec<f64> = docs
                    .iter()
                    .zip(candidates.iter().zip(&labelled))
                    .map(|(doc, (memory, repo))| {
                        let told =
                            format!("{} {}", text(&memory["name"]), text(&memory["description"]))
                                .to_lowercase();
                        let ends = |tail: &str| {
                            !tail.starts_with(|letter: char| {
                                letter.is_alphanumeric() || letter == '_'
                            })
                        };
                        let claims = CLAIM.iter().any(|phrase| told.contains(phrase))
                            || ["every request", "any request", "all request"].iter().any(
                                |phrase| {
                                    told.match_indices(phrase).any(|(at, _)| {
                                        let tail = told
                                            .get(at.saturating_add(phrase.len())..)
                                            .unwrap_or_default();
                                        ends(tail) || tail.strip_prefix('s').is_some_and(ends)
                                    })
                                },
                            );
                        let bonus = if claims {
                            0.0f64
                        } else if about.as_ref() == Some(repo) {
                            4.0f64
                        } else if GENERAL.contains(&repo.as_str()) {
                            1.0f64
                        } else {
                            0.0f64
                        };
                        let length = count(doc.len());
                        idf.iter()
                            .map(|pair| {
                                (
                                    pair.1,
                                    count(doc.iter().filter(|token| **token == pair.0).count()),
                                )
                            })
                            .filter(|pair| pair.1 > 0.0f64)
                            .fold(0.0f64, |sum, (weight, tf)| {
                                sum + weight * tf * 2.2f64
                                    / 1.2f64.mul_add(0.75f64.mul_add(length / mean, 0.25f64), tf)
                            })
                            + bonus
                    })
                    .collect();
                let by_word = order(&lexical, (0..candidates.len()).collect());
                // the cosine rank: the query embedded, each candidate's vector from the cache,
                // at most `$SANDMAN_RECALL_EMBED` missing ones (the lexically best) embedded and
                // appended; one still missing takes part in the lexical rank only
                let texts: Vec<String> = candidates
                    .iter()
                    .zip(&labelled)
                    .map(|(memory, repo)| excerpt(memory, repo))
                    .collect();
                let keys: Vec<String> = texts.iter().map(|said| hash(said)).collect();
                let cache = vectors(&root);
                let floats = |vector: &Value| -> Vec<f64> {
                    vector
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_f64)
                        .collect()
                };
                let mut held: Vec<Option<Vec<f64>>> = keys
                    .iter()
                    .map(|hashed| cache.get(hashed).map(|line| floats(&line["v"])))
                    .collect();
                let query = embed(&[format!("{}{state}", EMBED[1])], begun, limit)
                    .map(|mut found| found.pop().unwrap_or_default());
                let mut fail = |scorer, error: io::Error| {
                    errors.push((scorer, one_line(&error.to_string(), 400)));
                };
                let missing: Vec<usize> = by_word
                    .iter()
                    .copied()
                    .filter(|&index| held[index].is_none() && query.is_ok())
                    .take(setting("SANDMAN_RECALL_EMBED", 8))
                    .collect();
                let inline: Vec<String> =
                    missing.iter().map(|&index| texts[index].clone()).collect();
                if let Err(error) = if missing.is_empty() {
                    Ok(Vec::new())
                } else {
                    embed(&inline, begun, limit)
                }
                    .and_then(|found| {
                        let rows: Vec<String> = missing
                            .iter()
                            .zip(found)
                            .map(|(&index, vector)| {
                                let row = json!({"h": keys[index], "path": path(&candidates[index]), "v": vector});
                                held[index] = Some(vector);
                                format!("{row}\n")
                            })
                            .collect();
                        append(&root.join("embeddings.jsonl"), &rows.concat())
                    })
                {
                    fail("embed", error);
                }
                let fused = match query {
                    Ok(vector) => {
                        let cosine: Vec<f64> = held
                            .iter()
                            .map(|found| {
                                found.as_ref().map_or(0.0f64, |memory| {
                                    total(
                                        vector.iter().zip(memory).map(|(mine, other)| mine * other),
                                    )
                                })
                            })
                            .collect();
                        let by_meaning = order(
                            &cosine,
                            (0..candidates.len())
                                .filter(|&index| held[index].is_some())
                                .collect(),
                        );
                        let mut fused = vec![0.0f64; candidates.len()];
                        for ranking in [&by_word, &by_meaning] {
                            for (rank, &index) in ranking.iter().enumerate() {
                                fused[index] += reciprocal(rank);
                            }
                        }
                        Some(fused)
                    }
                    Err(error) => {
                        fail("embed", error);
                        None
                    }
                };
                // a tie in the fused order keeps the lexical order
                let first = fused
                    .as_ref()
                    .map_or_else(|| by_word.clone(), |scores| order(scores, by_word.clone()));
                // the decision: the first order's top 10 in one request, keyed by rank, to each
                // scorer in `$SANDMAN_SCORERS` until one answers; Jev skipped while it is down
                let shortlist: Vec<usize> = first.iter().copied().take(10).collect();
                let questions: Map<String, Value> = shortlist
                    .iter()
                    .enumerate()
                    .map(|(rank, &index)| {
                        let memory = &candidates[index];
                        let instructions = format!(
                            "Memory ({}): {} \u{2014} {}. {QUESTION}",
                            labelled[index],
                            text(&memory["name"]),
                            one_line(text(&memory["description"]), 160)
                        );
                        (
                            rank.to_string(),
                            json!({"instructions": instructions, "type": "noul"}),
                        )
                    })
                    .collect();
                let held_down =
                    serde_json::from_str::<Value>(&fs::read_to_string(&down).unwrap_or_default())
                        .ok()
                        .and_then(|stamp| stamp["until"].as_u64())
                        .is_some_and(|until| until > now());
                let ollama = setting("SANDMAN_OLLAMA", "http://127.0.0.1:11434".to_owned());
                let decided = tried
                    .split(',')
                    .filter(|scorer| !(*scorer == "jev" && held_down))
                    .find_map(|scorer| {
                        let (url, model) = match scorer {
                            "pplx" => (format!("{ollama}/v1/systemone"), "pplx-decider-flash:4b"),
                            "kev" => ("http://127.0.0.1:1703/v1/systemone".to_owned(), "kev-latest"),
                            "jev" => ("https://api.typesafe.ai/v1/systemone".to_owned(), "jev-latest"),
                            _ => return None,
                        };
                        let key = if scorer == "jev" { secret.as_str() } else { "" };
                        let request = json!({"model": model, "questions": questions, "state": state});
                        let (answer, replied) = if scorer == "jev" && secret.is_empty() {
                            (
                                Err(io::Error::other(
                                    "no API key: set TYPESAFE_API_KEY or write it to ~/.config/typesafe/api_key",
                                )),
                                String::new(),
                            )
                        } else {
                            post(&url, &request.to_string(), key, begun, limit)
                        };
                        let scored = answer.and_then(|reply| {
                            let nouls = (0..shortlist.len()).map(|rank| {
                                reply["answers"][rank.to_string()]["noul"]
                                    .as_f64()
                                    .ok_or_else(|| {
                                        io::Error::other(format!("reply: no noul for question {rank}"))
                                    })
                            });
                            Ok((
                                nouls.collect::<io::Result<Vec<f64>>>()?,
                                reply["usage"]["input_tokens"].as_u64().unwrap_or(0),
                            ))
                        });
                        scored
                            .map_err(|error| {
                                // out of credit or refused: down for 6 h, else for 60 s
                                let refused = ["billing_error", "error: 401", "error: 402", "error: 403"]
                                    .iter()
                                    .any(|marker| replied.contains(marker));
                                let expiry = setting(
                                    ["SANDMAN_JEV_DOWN_MS", "SANDMAN_JEV_HELD_MS"][usize::from(refused)],
                                    [60_000, 21_600_000][usize::from(refused)],
                                );
                                let until = json!({"error": one_line(&error.to_string(), 400), "until": now().saturating_add(expiry)});
                                drop((scorer == "jev").then(|| put(&down, &until.to_string())));
                                fail(scorer, error);
                            })
                            .ok()
                            .map(|found| (scorer.to_owned(), found))
                    });
                let embedded = fused.is_some();
                let (lines, max, route, scorer, tokens) = match (decided, fused) {
                    // the decision's order and the first order fused within the 10, a tie
                    // newer first, then the decision's order; kept at 2/66 or more, at most
                    // `$SANDMAN_RECALL_CAP` (6)
                    (Some((scorer, (nouls, tokens))), _) => {
                        let positions = order(&nouls, (0..nouls.len()).collect());
                        let mut place = vec![0usize; nouls.len()];
                        for (rank, &at) in positions.iter().enumerate() {
                            place[at] = rank;
                        }
                        let finals: Vec<f64> = (0..nouls.len())
                            .map(|at| reciprocal(place[at]) + reciprocal(at))
                            .collect();
                        let mut dated = positions;
                        dated.sort_by(|left, right| {
                            when(&candidates[shortlist[*right]])
                                .cmp(&when(&candidates[shortlist[*left]]))
                        });
                        let lines = order(&finals, dated)
                            .into_iter()
                            .filter(|&at| finals[at] >= 2.0f64 / 66.0f64)
                            .map(|at| (finals[at] * 61.0f64 / 2.0f64, shortlist[at]))
                            .collect();
                        (
                            lines,
                            setting("SANDMAN_RECALL_CAP", 6),
                            scorer.clone(),
                            Some(scorer),
                            tokens,
                        )
                    }
                    // no decision model: the reference's hybrid cut on the first fusion
                    (None, Some(scores)) => {
                        let top = first.first().map_or(0.0f64, |&index| scores[index]);
                        let lines = first
                            .iter()
                            .filter(|&&index| {
                                scores[index] >= 0.028_438_886_647_841_874f64
                                    && scores[index] >= top - 0.006_272_091_157_751_175f64
                            })
                            .map(|&index| (scores[index] * 61.0f64 / 2.0f64, index))
                            .collect();
                        (lines, 8, "hybrid, no decision model".to_owned(), None, 0)
                    }
                    // neither: the lexical rank, each over the top one, a tie newer first
                    (None, None) => {
                        let top = by_word.first().map_or(0.0f64, |&index| lexical[index]);
                        let mut dated = by_word;
                        dated.sort_by(|left, right| {
                            when(&candidates[*right]).cmp(&when(&candidates[*left]))
                        });
                        let lines = order(&lexical, dated)
                            .into_iter()
                            .map(|index| (lexical[index] / top.max(f64::MIN_POSITIVE), index))
                            .collect();
                        (lines, 5, "lexical".to_owned(), None, 0)
                    }
                };
                (lines, max, route, scorer, embedded, tokens, errors)
            };
            let (ranked, max, route, scorer, embedded, tokens, errors) =
                if ask && !candidates.is_empty() {
                    let (lines, max, route, scorer, embedded, tokens, errors) = pass();
                    (lines, max, Some(route), scorer, embedded, tokens, errors)
                } else {
                    (Vec::new(), usize::MAX, None, None, false, 0, Vec::new())
                };
            let fits = |listed: usize, used: usize| listed < max && used <= budget;
            let failing = errors
                .iter()
                .map(|error| error.0)
                .collect::<Vec<_>>()
                .join(" and ");
            let head = |sent: usize| {
                format!(
                    "recall \u{b7} {} \u{b7} {sent} of {}{}\n",
                    one_line(&cue, 60),
                    candidates.len(),
                    route.as_ref().map_or_else(String::new, |taken| {
                        if failing.is_empty() {
                            format!(" \u{b7} {taken}")
                        } else {
                            format!(" \u{b7} {taken}: {failing} failed")
                        }
                    })
                )
            };
            let failed = (!errors.is_empty()).then(|| {
                errors
                    .iter()
                    .map(|error| format!("{}: {}", error.0, error.1))
                    .collect::<Vec<_>>()
                    .join("; ")
            });
            let mut line = json!({
                "ask": ask, "considered": candidates.len(), "cue": one_line(&cue, 200), "cut": 0u8,
                "embedded": embedded, "kind": "recall", "ms": begun.elapsed().as_millis(),
                "scorer": scorer, "sent": [], "session": session, "tokens": tokens,
            });
            for (field, value) in [("about", &about), ("error", &failed)] {
                if let Some(set) = value.as_ref() {
                    line[field] = set.as_str().into();
                }
            }
            let foot = &*format!("read one in full: {me} show <path>\n");
            // the seed opens with its instructions, what is due and the always-set, counted first
            let opening = if seed {
                format!(
                    "{}\n\n{}always \u{b7} about the person, or pinned\n",
                    SEED.replace("<id>", sid).replace("<me>", me),
                    agenda(&root, false, 10, me, &label)
                )
            } else {
                String::new()
            };
            // one line a memory while the cut rule holds; what went into the ledger line
            let mut rows = || {
                let mut used = [opening.as_str(), &head(candidates.len()), foot]
                    .concat()
                    .chars()
                    .count();
                let (mut listed, mut carried, mut digest, mut sent, mut cut) =
                    (String::new(), Vec::new(), String::new(), Vec::new(), 0usize);
                for (score, memory) in always
                    .iter()
                    .filter(|_| seed)
                    .map(|memory| (None, memory))
                    .chain(
                        ranked
                            .iter()
                            .map(|row| (Some(format!("{:.2}", row.0)), &candidates[row.1]))
                            .filter(|row| !before.contains(path(row.1).as_str())),
                    )
                {
                    let row = format!(
                        "{} {}{} ({} \u{b7} {}) \u{2014} {}\n",
                        score
                            .as_deref()
                            .map_or("  \u{b7}", |figure| figure.trim_start_matches('0')),
                        // a trace is provisional until consolidation keeps it
                        if path(memory).starts_with("traces/") {
                            "~"
                        } else {
                            ""
                        },
                        text(&memory["name"]),
                        text(&memory["type"]),
                        path(memory),
                        one_line(text(&memory["description"]), 160)
                    );
                    used = used.saturating_add(row.chars().count());
                    if !fits(sent.len(), used) {
                        cut = cut.saturating_add(1);
                        continue;
                    }
                    if let Some(figure) = score {
                        digest.push_str(&row);
                        sent.push(json!({"file": path(memory), "id": memory["id"], "score": figure.parse::<f64>().unwrap_or_default()}));
                    } else {
                        listed.push_str(&row);
                        carried.push(path(memory));
                    }
                }
                let count = sent.len();
                (line["cut"], line["sent"]) = (cut.into(), sent.into());
                if seed {
                    (line["always"], line["seed"]) = (carried.into(), true.into());
                }
                (listed, digest, count)
            };
            let (listed, digest, sent) = rows();
            eprintln!(
                "{}recall: scorer={} embedded={embedded} ask={ask} about={} ms={} tokens={tokens} considered={} sent={sent} cut={}",
                failed
                    .as_ref()
                    .map_or_else(String::new, |error| format!("sandman recall: {error}\n")),
                line["scorer"],
                line["about"],
                line["ms"],
                line["considered"],
                line["cut"]
            );
            // a pull's lines, else `nothing recalled` and why when the gate closed; a seed's
            // prompt: its lines, nothing without a pass
            let tail = if seed && route.is_none() {
                foot.to_owned()
            } else if seed || sent > 0 {
                [head(sent).as_str(), &digest, foot].concat()
            } else {
                let closed = if ask { "" } else { ": no specific ask" };
                format!("{}nothing recalled{closed}\n", head(0))
            };
            print!("{opening}{listed}{tail}");
            drop(record(&root, &mut line));
            ExitCode::SUCCESS
        }
        "remember" => {
            let body = args.get(2).map_or("", |said| said.trim());
            let asked = flag("--topic");
            let kind = flag("--type").unwrap_or_else(|| {
                ["feedback", "knowledge"][usize::from(asked.is_some())].to_owned()
            });
            let Some(shelved) = topic(&kind, asked.as_deref()).filter(|_| !body.is_empty()) else {
                eprintln!(
                    "sandman remember: takes a non-empty \"<body>\" first and --type user|feedback|project|reference, or knowledge|playbook with --topic <slug>"
                );
                return ExitCode::from(2);
            };
            // homed as encode homes a trace: the directory --about names, else the cwd, else home;
            // a knowledge type by its topic
            let bank = if shelved.is_empty() {
                key(&project(
                    &regarding
                        .as_ref()
                        .map(PathBuf::from)
                        .filter(|dir| dir.is_dir())
                        .or_else(|| env::current_dir().ok())
                        .unwrap_or_else(|| home.clone()),
                ))
            } else {
                format!("@{shelved}")
            };
            let title = flag("--name").unwrap_or_else(|| {
                body.split_whitespace()
                    .take(8)
                    .collect::<Vec<_>>()
                    .join(" ")
            });
            let line = flag("--description")
                .unwrap_or_else(|| one_line(body.lines().next().unwrap_or_default(), 160));
            let file = slot(
                &root.join("traces").join(&bank),
                &kind,
                &title,
                &mut BTreeSet::new(),
            );
            let (now, session, id) = (utc(SystemTime::now()), pull.as_deref(), mint());
            let contents = note(
                &[
                    ("id", &id),
                    ("name", one_line(&title, 200).as_str()),
                    ("description", one_line(&line, 400).as_str()),
                    ("type", &kind),
                    ("created", &now),
                    (
                        "about",
                        &one_line(regarding.as_deref().unwrap_or_default(), 400),
                    ),
                    ("topic", shelved),
                    ("session", session.unwrap_or_default()),
                    ("origin", ["hand", ""][usize::from(session.is_some())]),
                    (
                        "source",
                        &format!("remember {now} \u{b7} {}", session.unwrap_or("-")),
                    ),
                ],
                body,
            );
            if let Err(error) = put(&file, &contents) {
                eprintln!("sandman remember: {error}");
                return ExitCode::FAILURE;
            }
            let path = format!("traces/{bank}/{}", name(&file));
            drop(record(
                &root,
                &mut json!({"file": path, "id": id, "kind": "remember", "session": pull, "type": kind}),
            ));
            println!("{me} show {path}");
            ExitCode::SUCCESS
        }
        "rethink" => {
            let wanted = args.get(2).cloned().unwrap_or_default();
            let given: Vec<&str> = VERDICTS
                .into_iter()
                .filter(|verdict| args.contains(&format!("--{verdict}")))
                .collect();
            let [verdict] = given[..] else {
                eprintln!(
                    "sandman rethink: takes <path> first and one of --irrelevant, --used, --wrong, --stale"
                );
                return ExitCode::from(2);
            };
            let Some(memory) = every(&root)
                .into_iter()
                .find(|note| note["path"] == wanted.as_str())
            else {
                eprintln!(
                    "sandman rethink: no memory {wanted} (want [traces/]<bank>/<file>.md or knowledge/<topic>/<file>.md under {})",
                    root.display()
                );
                return ExitCode::FAILURE;
            };
            let line =
                json!({"by": "agent", "session": pull, "verdict": verdict, "why": flag("--why")});
            if let Err(error) = rethink(&root, &ledger(&root), &memory, line) {
                eprintln!("sandman rethink: {error}");
                return ExitCode::FAILURE;
            }
            println!("rethink: {verdict} {wanted}");
            ExitCode::SUCCESS
        }
        "serve" => {
            let rundir = root.join("run");
            let (log, sock, tmp) = (
                rundir.join("serve.log"),
                rundir.join("serve.sock"),
                rundir.join("serve.new"),
            );
            let started = || -> io::Result<_> {
                if sock.as_os_str().len() > 103 {
                    return Err(io::Error::other(format!(
                        "{} is over a unix socket's 103 bytes; shorten SANDMAN_ROOT",
                        sock.display()
                    )));
                }
                fs::create_dir_all(&root)?;
                drop(DirBuilder::new().mode(0o700).create(&rundir));
                // append, not create: a second instance must not even truncate the lock file
                let lock = OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(rundir.join("serve.lock"))?;
                match lock.try_lock() {
                    Err(TryLockError::WouldBlock) => {
                        return Err(io::Error::other("already running"));
                    }
                    other => other?,
                }
                drop(fs::remove_file(&tmp));
                let listener = UnixListener::bind(&tmp)?;
                let dir = fs::metadata(&rundir)?;
                if dir.uid() != fs::metadata(&tmp)?.uid() || dir.mode() & 0o077 != 0 {
                    fs::remove_file(&tmp)?;
                    return Err(io::Error::other(format!(
                        "{} must be this user's, mode 0700",
                        rundir.display()
                    )));
                }
                fs::rename(&tmp, &sock)?;
                fs::write(rundir.join("serve.pid"), id().to_string())?;
                Ok((lock, listener, Signals::new([SIGHUP, SIGINT, SIGTERM])?))
            };
            let (_lock, listener, mut signals) = match started() {
                Ok(held) => held,
                Err(error) => {
                    eprintln!("sandman serve: {error}");
                    return ExitCode::FAILURE;
                }
            };
            let cap = setting("SANDMAN_LOG_CAP", 1u64 << 20);
            let write = move |mut line: Value| {
                line["at"] = utc(SystemTime::now()).into();
                if fs::metadata(&log).is_ok_and(|meta| meta.len() >= cap) {
                    drop(fs::rename(&log, log.with_extension("log.1")));
                }
                drop(append(&log, &format!("{line}\n")));
            };
            let hook = write.clone();
            panic::set_hook(Box::new(move |info| {
                hook(json!({"event": "panic", "reason": info.to_string()}));
            }));
            let wake = sock.clone();
            drop(thread::spawn(move || {
                for signal in signals.forever() {
                    STOP.set(
                        signal_name(signal)
                            .unwrap_or_default()
                            .trim_start_matches("SIG"),
                    )
                    .unwrap_or_default();
                    drop(UnixStream::connect(&wake));
                }
            }));
            let (this, settle, data) = (machine(&root), write.clone(), root.clone());
            let ms = |key: &str, default: u64| Duration::from_millis(setting(key, default));
            let (tick, quiet, calm) = (
                ms("SANDMAN_TICK_MS", 30_000),
                ms("SANDMAN_QUIET_MS", 120_000),
                ms("SANDMAN_CONSOLIDATE_QUIET_MS", 600_000),
            );
            let (grace, idle) = (
                ms("SANDMAN_GRACE_MS", 600_000),
                ms("SANDMAN_IDLE_MS", 604_800_000),
            );
            let ago =
                |span: Duration| utc(SystemTime::now().checked_sub(span).unwrap_or(UNIX_EPOCH));
            let spent = |time: SystemTime| time.elapsed().unwrap_or_default();
            let (encode_limit, consolidate_limit) = (
                ms("SANDMAN_ENCODE_MS", 300_000),
                ms("SANDMAN_CONSOLIDATE_MS", 600_000),
            );
            let (budget, allowance) = (
                setting("SANDMAN_DIGEST_BYTES", 300_000usize),
                setting("SANDMAN_CONSOLIDATE_BYTES", 300_000usize),
            );
            let (cadence, reflect_limit) = (
                ms("SANDMAN_REFLECT_MS", 604_800_000),
                ms("SANDMAN_REFLECTOR_MS", 600_000),
            );
            let (minimum, replays, ceiling, evidence) = (
                setting("SANDMAN_REFLECT_MIN", 5usize),
                setting("SANDMAN_REFLECT_REPLAY", 3usize),
                setting("SANDMAN_GUIDANCE_BYTES", 8000usize),
                setting("SANDMAN_REFLECT_BYTES", 300_000usize),
            );
            let command = |key: &str| env::var(key).unwrap_or_else(|_| CLAUDE.to_owned());
            let (encoder, consolidator, reflector) = (
                command("SANDMAN_ENCODER"),
                command("SANDMAN_CONSOLIDATOR"),
                command("SANDMAN_REFLECTOR"),
            );
            let logged = move |event: &str, key: &str, value: &str, written: io::Result<Value>| {
                let mut line = written.unwrap_or_else(|error| json!({"reason": error.to_string()}));
                (line["event"], line[key]) = (event.into(), value.into());
                settle(line);
            };
            let ticker = thread::spawn(move || {
                while STOP.get().is_none() {
                    thread::park_timeout(tick);
                    let lines = ledger(&data);
                    let story = sessions(&lines);
                    // this machine's sessions started before the grace with no end since, no job
                    // naming them: without a transcript gone, idle past the window taken; at most
                    // 20 gone and one taken a tick
                    let (mut left, born) = ([20u8, 1], ago(grace));
                    for (session, marks) in &story {
                        let (start, at) = (&marks[0], marks[3]["at"].as_str());
                        if start["machine"] != this
                            || at <= marks[1]["at"].as_str()
                            || at > Some(born.as_str())
                        {
                            continue;
                        }
                        let live = touched(&claude, session);
                        let which = usize::from(live.is_some());
                        if left[which] == 0
                            || live.is_some_and(|time| spent(time) <= idle)
                            || job(&claude, session).is_some()
                            || STOP.get().is_some()
                        {
                            continue;
                        }
                        left[which] = left[which].saturating_sub(1);
                        let reason = ["no end recorded", "idle"][which];
                        logged(
                            "reconcile",
                            "session",
                            session,
                            end(&data, &claude, session, &start["cwd"], &reason.into(), None),
                        );
                    }
                    // the embedding cache: rewritten to its live lines once past twice their size,
                    // then what recall would embed and it lacks, in reader order, at most
                    // `$SANDMAN_EMBED_TICK` a tick
                    {
                        let (_, _, label) = labels(&home, &claude);
                        let (cache, file) = (vectors(&data), data.join("embeddings.jsonl"));
                        let wanted: Vec<(String, String, String)> = pool(&data)
                            .1
                            .iter()
                            .map(|memory| {
                                let excerpted = excerpt(memory, &label(text(&memory["bank"])));
                                (
                                    hash(&excerpted),
                                    text(&memory["path"]).to_owned(),
                                    excerpted,
                                )
                            })
                            .collect();
                        let live: String = wanted
                            .iter()
                            .map(|row| row.0.as_str())
                            .collect::<BTreeSet<_>>()
                            .into_iter()
                            .filter_map(|hashed| cache.get(hashed))
                            .flat_map(|line| [line.to_string(), "\n".to_owned()])
                            .collect();
                        if STOP.get().is_none()
                            && fs::metadata(&file).is_ok_and(|meta| {
                                meta.len()
                                    > u64::try_from(live.len())
                                        .unwrap_or(u64::MAX)
                                        .saturating_mul(2)
                            })
                        {
                            drop(put(&file, &live));
                        }
                        let missing: Vec<&(String, String, String)> = wanted
                            .iter()
                            .filter(|row| !cache.contains_key(&row.0))
                            .take(setting("SANDMAN_EMBED_TICK", 128))
                            .collect();
                        if !missing.is_empty() && STOP.get().is_none() {
                            let texts: Vec<String> =
                                missing.iter().map(|row| row.2.clone()).collect();
                            let done = embed(&texts, Instant::now(), Duration::from_secs(60))
                                .and_then(|found| {
                                    let rows: Vec<String> = missing
                                        .iter()
                                        .zip(found)
                                        .map(|(row, vector)| {
                                            format!(
                                                "{}\n",
                                                json!({"h": row.0, "path": row.1, "v": vector})
                                            )
                                        })
                                        .collect();
                                    append(&file, &rows.concat())
                                        .map(|()| json!({"embedded": rows.len()}))
                                });
                            logged("embed", "event", "embed", done);
                        }
                    }
                    // this machine's sessions whose latest end has `outcome`, no start since
                    let ended = |marks: &[Value; 5], outcome: &str| {
                        marks[1]["outcome"] == outcome
                            && marks[1]["machine"] == this
                            && marks[3]["at"].as_str() <= marks[1]["at"].as_str()
                    };
                    // a held end whose job is gone and whose transcript has been quiet for the
                    // window is taken
                    for (session, marks) in &story {
                        let held = &marks[1];
                        if !ended(marks, "held")
                            || job(&claude, session).is_some()
                            || touched(&claude, session).is_some_and(|time| spent(time) < quiet)
                            || STOP.get().is_some()
                        {
                            continue;
                        }
                        logged(
                            "take",
                            "session",
                            session,
                            end(
                                &data,
                                &claude,
                                session,
                                &held["cwd"],
                                &"job gone".into(),
                                None,
                            ),
                        );
                    }
                    let oldest = story
                        .values()
                        .filter(|marks| {
                            ended(marks, "taken")
                                && marks[2]["at"].as_str() < marks[1]["at"].as_str()
                        })
                        .min_by_key(|marks| marks[1]["at"].as_str());
                    let cutoff = ago(calm);
                    let (labile, filed) = (
                        notes(&data, "traces"),
                        [notes(&data, "memories"), notes(&data, "knowledge")].concat(),
                    );
                    // per bank: when each trace was created and each wrong or stale verdict its
                    // notes still answer to was given, the verdicts flagged
                    let mut banks: BTreeMap<&str, Vec<(&str, bool)>> = BTreeMap::new();
                    for trace in &labile {
                        let bank = banks.entry(text(&trace["bank"])).or_default();
                        bank.push((text(&trace["created"]), false));
                    }
                    for held in &filed {
                        for verdict in judged(&lines, held) {
                            let bank = banks.entry(text(&held["bank"])).or_default();
                            bank.push((text(&verdict["at"]), true));
                        }
                    }
                    // quiet: nothing newer than the window; then a trace newer than its last
                    // failure, or a verdict newer than its last consolidate
                    let ripe = banks
                        .into_iter()
                        .filter(|pair| {
                            let latest = |verdicts: bool| {
                                let after = lines
                                    .iter()
                                    .filter(|line| {
                                        line["kind"] == "consolidate"
                                            && line["bank"] == pair.0
                                            && (verdicts || line["outcome"] == "failed")
                                    })
                                    .filter_map(|line| line["at"].as_str())
                                    .max();
                                after
                                    < pair
                                        .1
                                        .iter()
                                        .filter(|time| time.1 == verdicts)
                                        .map(|time| time.0)
                                        .max()
                            };
                            pair.1.iter().map(|time| time.0).max() <= Some(cutoff.as_str())
                                && (latest(false) || latest(true))
                        })
                        .min_by_key(|pair| pair.1.iter().map(|time| time.0).min())
                        .map(|pair| pair.0);
                    // the latest reflect line of this machine, and the model runs since
                    let since = lines
                        .iter()
                        .rposition(|line| line["kind"] == "reflect" && line["machine"] == this);
                    let last = since
                        .and_then(|at| lines.get(at))
                        .cloned()
                        .unwrap_or_default();
                    let after = lines
                        .get(since.map_or(0, |at| at.saturating_add(1))..)
                        .unwrap_or_default();
                    let ran = |line: &&Value| {
                        line["machine"] == this
                            && (line["kind"] == "consolidate"
                                || (line["kind"] == "encode"
                                    && (line["outcome"] == "encoded"
                                        || line["outcome"] == "failed")))
                    };
                    // an adoption whose job's next two runs both failed is rolled back
                    let revert = last["history"]
                        .as_object()
                        .filter(|_| last["outcome"] == "adopted")
                        .into_iter()
                        .flat_map(Map::keys)
                        .find(|job| {
                            let next: Vec<&Value> = after
                                .iter()
                                .filter(ran)
                                .filter(|line| line["kind"] == job.as_str())
                                .take(2)
                                .collect();
                            next.len() == 2 && next.iter().all(|line| line["outcome"] == "failed")
                        })
                        .cloned();
                    let due = last["at"]
                        .as_str()
                        .is_none_or(|at| at <= ago(cadence).as_str())
                        && after.iter().filter(ran).count() >= minimum;
                    let noted = Cell::new(None);
                    // a stop that woke the tick, or landed since its last job: none starts
                    if STOP.get().is_some() {
                        continue;
                    }
                    // one job a tick: a rollback; else the oldest of this machine's episodes taken
                    // and not encoded since; else the quiet bank whose oldest trace is oldest; else,
                    // when due, the weekly review of the guidance
                    let (kind, key, value, mut zero, done) = if let Some(job) = revert {
                        let back = || -> io::Result<Value> {
                            let earlier =
                                fs::read_to_string(data.join(text(&last["history"][&job])))?;
                            Ok(json!({
                                "history": adopt(&data, &[(job.as_str(), earlier)])?,
                                "outcome": "reverted", "reason": "its next two runs failed",
                            }))
                        };
                        (
                            "reflect",
                            "kind",
                            "reflect".to_owned(),
                            json!({"history": {}}),
                            back(),
                        )
                    } else if let Some(marks) = oldest {
                        let (closing, interactive) = (&marks[1], &marks[4]);
                        let session = text(&closing["session"]);
                        let archived = Path::new(text(&closing["archived"]));
                        let encode = || -> io::Result<Value> {
                            if *interactive != true {
                                return Ok(json!({"outcome": "skipped", "traces": 0}));
                            }
                            if !archived.is_file() {
                                return Ok(json!({"outcome": "gone", "traces": 0}));
                            }
                            let (digest, cwd, recalled) = episode(&data, &lines, closing, budget);
                            let reply = run(
                                &data,
                                "encode",
                                None,
                                &encoder,
                                &digest,
                                encode_limit,
                                &noted,
                            )?;
                            let traces = reply["traces"].as_array();
                            if let Some(bad) = traces.map_or(Some(&reply), |list| {
                                list.iter().find(|trace| {
                                    topic(text(&trace["type"]), trace["topic"].as_str()).is_none()
                                        || ["body", "description", "name"]
                                            .iter()
                                            .any(|field| !trace[field].is_string())
                                        || !(trace["about"].is_null() || trace["about"].is_string())
                                })
                            }) {
                                return Err(io::Error::other(format!(
                                    "output: not a trace: {}",
                                    one_line(&bad.to_string(), 200)
                                )));
                            }
                            // at most one verdict per note `# recalled` lists, none on another
                            let (listed, mut judged, none): (BTreeMap<&str, &Value>, _, _) = (
                                recalled
                                    .iter()
                                    .map(|note| (text(&note["path"]), note))
                                    .collect(),
                                BTreeSet::new(),
                                Vec::new(),
                            );
                            let verdicts = if reply["verdicts"].is_null() {
                                Some(&none)
                            } else {
                                reply["verdicts"].as_array()
                            };
                            if let Some(bad) = verdicts.map_or_else(
                                || Some(&reply["verdicts"]),
                                |list| {
                                    list.iter().find(|verdict| {
                                        let file = text(&verdict["file"]);
                                        !(listed.contains_key(file)
                                            && VERDICTS.contains(&text(&verdict["verdict"]))
                                            && (verdict["why"].is_null()
                                                || verdict["why"].is_string())
                                            && judged.insert(file))
                                    })
                                },
                            ) {
                                return Err(io::Error::other(format!(
                                    "output: not a verdict: {}",
                                    one_line(&bad.to_string(), 200)
                                )));
                            }
                            let (now, mut ids) = (utc(SystemTime::now()), Vec::new());
                            for trace in traces.into_iter().flatten() {
                                let (kind, name, about) = (
                                    text(&trace["type"]),
                                    text(&trace["name"]),
                                    text(&trace["about"]),
                                );
                                let shelved =
                                    topic(kind, trace["topic"].as_str()).unwrap_or_default();
                                // a knowledge type's topic, else the bank of the repository the
                                // trace is about, else the cwd's
                                let bank = if shelved.is_empty() {
                                    key(&project(
                                        &Some(PathBuf::from(about))
                                            .filter(|dir| dir.is_absolute() && dir.is_dir())
                                            .or_else(|| cwd.as_ref().map(PathBuf::from))
                                            .unwrap_or_else(|| home.clone()),
                                    ))
                                } else {
                                    format!("@{shelved}")
                                };
                                let file = slot(
                                    &data.join("traces").join(bank),
                                    kind,
                                    name,
                                    &mut BTreeSet::new(),
                                );
                                ids.push(mint());
                                let contents = note(
                                    &[
                                        ("id", ids.last().map_or("", String::as_str)),
                                        ("name", one_line(name, 200).as_str()),
                                        (
                                            "description",
                                            one_line(text(&trace["description"]), 400).as_str(),
                                        ),
                                        ("type", kind),
                                        ("created", now.as_str()),
                                        ("about", one_line(about, 400).as_str()),
                                        ("topic", shelved),
                                        ("session", session),
                                        ("archived", archived.to_str().unwrap_or_default()),
                                        (
                                            "source",
                                            format!(
                                                "encode {now} \u{b7} {session} \u{b7} model={}",
                                                model(&encoder)
                                            )
                                            .as_str(),
                                        ),
                                    ],
                                    text(&trace["body"]),
                                );
                                put(&file, &contents)?;
                            }
                            verdicts
                                .into_iter()
                                .flatten()
                                .filter_map(|verdict| {
                                    Some((*listed.get(text(&verdict["file"]))?, verdict))
                                })
                                .try_for_each(|(note, verdict)| {
                                    rethink(
                                        &data,
                                        &lines,
                                        note,
                                        json!({"by": "encode", "session": session, "verdict": verdict["verdict"], "why": verdict["why"]}),
                                    )
                                })
                                .map(|()| json!({"ids": ids, "outcome": "encoded", "traces": ids.len()}))
                        };
                        (
                            "encode",
                            "session",
                            session.to_owned(),
                            json!({"traces": 0u8}),
                            encode(),
                        )
                    } else if let Some(bank) = ripe {
                        // a topic's traces fold into its knowledge pages, a bank's into its memories
                        let inbound: Vec<&Value> = labile
                            .iter()
                            .filter(|trace| trace["bank"] == bank)
                            .collect();
                        let (dir, memories) = shelf(&data, bank);
                        let (traces, subject) = (
                            data.join("traces").join(bank),
                            bank.strip_prefix('@').unwrap_or_default(),
                        );
                        let consolidate = || -> io::Result<Value> {
                            let before = measure(&data, Some(bank));
                            // the wrong and stale verdicts its memories still answer to
                            let blamed: BTreeMap<&str, Vec<&Value>> = memories
                                .iter()
                                .map(|memory| (text(&memory["file"]), judged(&lines, memory)))
                                .filter(|pair| !pair.1.is_empty())
                                .collect();
                            let charges = blamed
                                .iter()
                                .flat_map(|(file, verdicts)| {
                                    verdicts.iter().map(move |verdict| {
                                        format!(
                                            "{file} \u{b7} {} \u{b7} {}",
                                            text(&verdict["verdict"]),
                                            verdict["why"].as_str().map_or_else(
                                                || "-".to_owned(),
                                                |said| one_line(said, 400)
                                            )
                                        )
                                    })
                                })
                                .collect();
                            // the notes of other banks and topics nearest one sent, by the cosine of
                            // their cached embeddings, at most 5
                            let (_, _, label) = labels(&home, &claude);
                            let cache = vectors(&data);
                            let vector = |note: &Value| -> Option<Vec<f64>> {
                                let hashed = hash(&excerpt(note, &label(text(&note["bank"]))));
                                Some(
                                    cache.get(&hashed)?["v"]
                                        .as_array()?
                                        .iter()
                                        .filter_map(Value::as_f64)
                                        .collect(),
                                )
                            };
                            let ours: Vec<Vec<f64>> = inbound
                                .iter()
                                .copied()
                                .chain(&memories)
                                .filter_map(vector)
                                .collect();
                            #[expect(clippy::float_arithmetic, reason = "a cosine is float math")]
                            let mut near: Vec<(f64, String)> = pool(&data)
                                .1
                                .iter()
                                .filter(|note| note["bank"] != bank)
                                .filter_map(|note| {
                                    let theirs = vector(note)?;
                                    let best = ours
                                        .iter()
                                        .map(|one| {
                                            total(
                                                one.iter()
                                                    .zip(&theirs)
                                                    .map(|(mine, other)| mine * other),
                                            )
                                        })
                                        .fold(f64::MIN, f64::max);
                                    (best >= setting("SANDMAN_NEAR", 0.8f64)).then(|| {
                                        (
                                            best,
                                            format!(
                                                "{} \u{b7} {} \u{b7} {}",
                                                text(&note["path"]),
                                                text(&note["name"]),
                                                one_line(text(&note["description"]), 160)
                                            ),
                                        )
                                    })
                                })
                                .collect();
                            near.sort_by(|one, two| two.0.total_cmp(&one.0));
                            let (input, full) = stack(
                                bank,
                                &inbound,
                                &memories,
                                allowance,
                                &[
                                    ("judged", charges),
                                    (
                                        "elsewhere",
                                        near.into_iter().take(5).map(|pair| pair.1).collect(),
                                    ),
                                ],
                            );
                            let reply = run(
                                &data,
                                "consolidate",
                                None,
                                &consolidator,
                                &input,
                                consolidate_limit,
                                &noted,
                            )?;
                            let fed = |found: &[&Value]| -> BTreeMap<String, Value> {
                                found
                                    .iter()
                                    .map(|one| (text(&one["file"]).to_owned(), (*one).clone()))
                                    .collect()
                            };
                            let (given, held_memories) = (fed(&inbound), fed(&full));
                            // every trace named once, a memory at most once, only whole ones; a
                            // revise only on a judged one
                            let ops = reply["ops"].as_array();
                            let mut named = BTreeSet::new();
                            if let Some(bad) = ops.map_or(Some(&reply), |list| {
                                list.iter().find(|op| {
                                    let (trace, memory) =
                                        (op["trace"].as_str(), op["memory"].as_str());
                                    let parents =
                                        op.get("memories").map(|files| -> Option<Vec<&str>> {
                                            files.as_array()?.iter().map(Value::as_str).collect()
                                        });
                                    let has = |fields: &[&str]| {
                                        fields.iter().all(|field| op[field].is_string())
                                    };
                                    let typed = [KINDS.as_slice(), &KNOWN]
                                        [usize::from(!subject.is_empty())]
                                    .contains(&text(&op["type"]));
                                    let shape = (
                                        trace.is_some(),
                                        memory.is_some(),
                                        parents.as_ref().map(|files| files.as_ref().map(Vec::len)),
                                    );
                                    !(match (text(&op["op"]), shape) {
                                        ("keep", (true, false, None))
                                        | ("merge", (false, false, Some(Some(2..)))) => {
                                            typed && has(&["body", "description", "name"])
                                        }
                                        ("merge", (true, true, None)) => {
                                            has(&["body", "description"])
                                        }
                                        ("graduate", (false, false, Some(Some(1..)))) => {
                                            topic(text(&op["type"]), op["topic"].as_str())
                                                .is_some_and(|slug| !slug.is_empty())
                                                && has(&["body", "description", "name", "reason"])
                                        }
                                        ("prune", (true, false, None) | (false, true, None)) => {
                                            has(&["reason"])
                                        }
                                        ("rehome", (false, true, None)) => {
                                            // another bank of its kind, already on disk
                                            let target = text(&op["bank"]);
                                            let tier = target.strip_prefix('@').unwrap_or(target);
                                            target != bank
                                                && target.starts_with('@') == bank.starts_with('@')
                                                && !tier.is_empty()
                                                && !tier.contains('/')
                                                && !tier.starts_with(['.', '_'])
                                                && shelf(&data, target).0.is_dir()
                                        }
                                        ("retitle", (false, true, None)) => {
                                            has(&["description", "name"])
                                        }
                                        ("revise", (false, true, None)) => {
                                            has(&["body", "description"])
                                                && memory
                                                    .is_some_and(|file| blamed.contains_key(file))
                                        }
                                        _ => false,
                                    } && trace.is_none_or(|file| {
                                        given.contains_key(file)
                                            && named.insert(("trace", file.to_owned()))
                                    }) && memory
                                        .into_iter()
                                        .chain(parents.flatten().into_iter().flatten())
                                        .all(|file| {
                                            held_memories.contains_key(file)
                                                && named.insert(("memory", file.to_owned()))
                                        }))
                                })
                            }) {
                                return Err(io::Error::other(format!(
                                    "output: not an op: {}",
                                    one_line(&bad.to_string(), 200)
                                )));
                            }
                            if let Some(file) = given
                                .keys()
                                .find(|file| !named.contains(&("trace", (*file).clone())))
                            {
                                return Err(io::Error::other(format!(
                                    "output: trace not named: {file}"
                                )));
                            }
                            // stage every write, then move aside what goes, then rename the writes in
                            let now = utc(SystemTime::now());
                            let mut claimed = BTreeSet::new();
                            let (mut aside, mut writes) = (
                                Vec::new(),
                                vec![(
                                    dated(
                                        &data,
                                        ".pruned",
                                        &format!("{bank}-ops.json"),
                                        &mut claimed,
                                    ),
                                    format!("{reply}\n"),
                                )],
                            );
                            let mut rewrites = Vec::new();
                            for op in ops.into_iter().flatten() {
                                let trace = op["trace"].as_str().and_then(|file| given.get(file));
                                let memory = op["memory"]
                                    .as_str()
                                    .and_then(|file| held_memories.get(file));
                                // a merge's parents, each with an id: one minted for one without
                                let parents: Vec<Value> = op["memories"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter_map(|file| held_memories.get(text(file)))
                                    .map(|held| {
                                        let mut parent = held.clone();
                                        parent["id"] = parent["id"]
                                            .as_str()
                                            .map_or_else(mint, str::to_owned)
                                            .into();
                                        parent
                                    })
                                    .collect();
                                let inputs: Vec<(&Path, &Value)> = trace
                                    .map(|found| (traces.as_path(), found))
                                    .into_iter()
                                    .chain(
                                        memory
                                            .into_iter()
                                            .chain(&parents)
                                            .map(|found| (dir.as_path(), found)),
                                    )
                                    .collect();
                                for &(tier, found) in &inputs {
                                    let file = text(&found["file"]);
                                    aside.push((
                                        tier.join(file),
                                        dated(
                                            &data,
                                            ".pruned",
                                            &format!("{bank}-{file}"),
                                            &mut claimed,
                                        ),
                                    ));
                                }
                                let at =
                                    |field: &str| trace.map_or("", |found| text(&found[field]));
                                let of =
                                    |field: &str| memory.map_or("", |found| text(&found[field]));
                                let one = |field: &str, max: usize| one_line(text(&op[field]), max);
                                let stamp = format!(
                                    "consolidate {now} \u{b7} {} \u{b7} model={}",
                                    Some(at("session"))
                                        .filter(|session| !session.is_empty())
                                        .unwrap_or("-"),
                                    model(&consolidator)
                                );
                                // the result continues the note: the trace's id on a keep, the
                                // memory's otherwise, descended from every session and note its
                                // inputs were, its origin kept only without either; a merge of
                                // notes starts a new one, from its parents alone
                                let fresh = !parents.is_empty();
                                let values = || inputs.iter().map(|pair| pair.1);
                                let sessions =
                                    Vec::from_iter(lineage(values().filter(|_| !fresh))).join(" ");
                                let from =
                                    Vec::from_iter(words(values().map(|part| {
                                        text(&part[if fresh { "id" } else { "from" }])
                                    })))
                                    .join(" ");
                                let origin = memory
                                    .into_iter()
                                    .chain(trace)
                                    .find_map(|part| part["origin"].as_str())
                                    .filter(|_| sessions.is_empty() && from.is_empty())
                                    .unwrap_or_default();
                                let id = memory
                                    .or(trace)
                                    .and_then(|part| part["id"].as_str())
                                    .map_or_else(mint, str::to_owned);
                                let (kind, name, description) =
                                    (text(&op["type"]), one("name", 200), one("description", 400));
                                match (text(&op["op"]), fresh) {
                                    ("keep", _) => {
                                        writes.push((
                                            slot(&dir, kind, text(&op["name"]), &mut claimed),
                                            note(
                                                &[
                                                    ("id", id.as_str()),
                                                    ("name", name.as_str()),
                                                    ("description", description.as_str()),
                                                    ("type", kind),
                                                    ("created", at("created")),
                                                    ("about", at("about")),
                                                    ("topic", at("topic")),
                                                    ("session", at("session")),
                                                    ("archived", at("archived")),
                                                    ("from", from.as_str()),
                                                    ("origin", origin),
                                                    ("sessions", sessions.as_str()),
                                                    ("source", stamp.as_str()),
                                                ],
                                                text(&op["body"]),
                                            ),
                                        ));
                                    }
                                    (made, true) => {
                                        // a graduation's trace goes to its topic's traces
                                        let (into, slugged) = if made == "graduate" {
                                            let slug = text(&op["topic"]);
                                            (data.join("traces").join(format!("@{slug}")), slug)
                                        } else {
                                            (dir.clone(), subject)
                                        };
                                        writes.push((
                                            slot(&into, kind, text(&op["name"]), &mut claimed),
                                            note(
                                                &[
                                                    ("id", id.as_str()),
                                                    ("name", name.as_str()),
                                                    ("description", description.as_str()),
                                                    ("type", kind),
                                                    ("created", now.as_str()),
                                                    ("topic", slugged),
                                                    ("from", from.as_str()),
                                                    ("source", stamp.as_str()),
                                                ],
                                                text(&op["body"]),
                                            ),
                                        ));
                                    }
                                    ("prune", _) => {}
                                    (continuing, _) => {
                                        // merged, revised, retitled or rehomed: the memory continuing,
                                        // its keys kept in order
                                        let mut fields = front(of("text"));
                                        fields.retain(|field| {
                                            !["from", "id", "origin", "sessions"].contains(&field.0)
                                        });
                                        let source = fields
                                            .iter()
                                            .position(|field| field.0 == "source")
                                            .unwrap_or(fields.len());
                                        drop(fields.splice(
                                            source..source,
                                            [
                                                ("from", from.as_str()),
                                                ("origin", origin),
                                                ("sessions", sessions.as_str()),
                                            ],
                                        ));
                                        fields.insert(0, ("id", id.as_str()));
                                        // a retitle under its new name, a rehome in its new bank,
                                        // both with the body as it was; else rewritten in place
                                        let target = text(&op["bank"]);
                                        let (amended, into, body): (&[(&str, &str)], _, _) =
                                            match continuing {
                                                "retitle" => (
                                                    &[
                                                        ("description", &description),
                                                        ("name", &name),
                                                    ],
                                                    slot(&dir, of("type"), &name, &mut claimed),
                                                    of("body"),
                                                ),
                                                "rehome" => (
                                                    &[(
                                                        "topic",
                                                        target
                                                            .strip_prefix('@')
                                                            .unwrap_or_default(),
                                                    )],
                                                    slot(
                                                        &shelf(&data, target).0,
                                                        of("type"),
                                                        of("name"),
                                                        &mut claimed,
                                                    ),
                                                    of("body"),
                                                ),
                                                _ => (
                                                    &[
                                                        ("description", &description),
                                                        ("source", &stamp),
                                                        ("updated", &now),
                                                    ],
                                                    dir.join(of("file")),
                                                    text(&op["body"]),
                                                ),
                                            };
                                        amend(&mut fields, amended);
                                        writes.push((into, note(&fields, body)));
                                    }
                                }
                                // a line per op, each input as it stood and where it went, if anywhere
                                let landed = writes.last().filter(|_| op["op"] != "prune").map(|pending| {
                                    let under = pending.0.strip_prefix(&data).unwrap_or(&pending.0).to_string_lossy();
                                    json!({"id": id, "path": under.strip_prefix("memories/").unwrap_or(&under)})
                                });
                                rewrites.push(json!({
                                    "from": values().map(entry).collect::<Vec<_>>(),
                                    "kind": "rewrite", "model": model(&consolidator), "op": op["op"], "reason": op["reason"], "to": landed,
                                }));
                            }
                            if let Err(error) = writes.iter().try_for_each(|pending| {
                                pending.0.parent().map_or(Ok(()), fs::create_dir_all)?;
                                fs::write(staged(&pending.0), &pending.1)
                            }) {
                                for pending in &writes {
                                    drop(fs::remove_file(staged(&pending.0)));
                                }
                                return Err(error);
                            }
                            for (from, to) in aside {
                                to.parent().map_or(Ok(()), fs::create_dir_all)?;
                                fs::rename(from, to)?;
                            }
                            for pending in &writes {
                                fs::rename(staged(&pending.0), &pending.0)?;
                            }
                            for mut line in rewrites {
                                drop(record(&data, &mut line));
                            }
                            let count = |pick: &dyn Fn(&Value) -> bool| {
                                ops.into_iter().flatten().filter(|done| pick(done)).count()
                            };
                            let tally = |op: &str| count(&|done| done["op"] == op);
                            Ok(json!({
                                "kept": tally("keep"), "merged": tally("merge"),
                                "order": {"after": measure(&data, Some(bank)), "before": before, "merges": count(&|done| done["op"] == "merge" && done["memories"].is_array())},
                                "outcome": "consolidated", "pruned": tally("prune"),
                            }))
                        };
                        (
                            "consolidate",
                            "bank",
                            bank.to_owned(),
                            json!({"kept": 0u8, "merged": 0u8, "pruned": 0u8}),
                            consolidate(),
                        )
                    } else if due {
                        // decay first, with no model run: each note under the floor but one about
                        // the person, pinned or muted, or with a verdict consolidate still answers
                        let decay = || -> io::Result<usize> {
                            let (floor, now) = (
                                setting("SANDMAN_DECAY_FLOOR", 0.1f64),
                                day(&today()).unwrap_or_default(),
                            );
                            let mut pruned = 0usize;
                            for note in
                                [notes(&data, "memories"), notes(&data, "knowledge")].concat()
                            {
                                let force = strength(&lines, &note, now);
                                if force < floor
                                    && note["type"] != "user"
                                    && !["mute", "pin"].contains(&text(&note["recall"]))
                                    && judged(&lines, &note).is_empty()
                                {
                                    let cause =
                                        json!(format!("decay: strength {force} under {floor}"));
                                    retire(&data, &note, "prune", &cause, &Value::Null)?;
                                    pruned = pruned.saturating_add(1);
                                }
                            }
                            Ok(pruned)
                        };
                        let review = || -> io::Result<Value> {
                            let mine = |kind: &str| -> Vec<&Value> {
                                after
                                    .iter()
                                    .filter(|line| line["kind"] == kind && line["machine"] == this)
                                    .collect()
                            };
                            let pruned = tree(&data.join(".pruned"), 4);
                            let replies: Vec<&PathBuf> = pruned
                                .iter()
                                .filter(|path| name(path).ends_with("-ops.json"))
                                .collect();
                            // `<HHMMSS>-<bank>-ops.json`: the bank
                            let bank_of = |path: &Path| {
                                name(path)
                                    .get(7..)
                                    .and_then(|rest| rest.strip_suffix("-ops.json"))
                                    .unwrap_or_default()
                                    .to_owned()
                            };
                            let ops_of = |path: &Path| {
                                serde_json::from_str::<Value>(
                                    &fs::read_to_string(path).unwrap_or_default(),
                                )
                                .unwrap_or_default()["ops"]
                                    .as_array()
                                    .cloned()
                                    .unwrap_or_default()
                            };
                            // each op since, its body cut
                            let ops: Vec<(String, Value)> = replies
                                .iter()
                                .filter(|path| {
                                    fs::metadata(path)
                                        .and_then(|meta| meta.modified())
                                        .is_ok_and(|time| utc(time).as_str() > text(&last["at"]))
                                })
                                .flat_map(|path| {
                                    let bank = bank_of(path);
                                    ops_of(path).into_iter().map(move |mut op| {
                                        drop(
                                            op.as_object_mut().map(|fields| fields.remove("body")),
                                        );
                                        (bank.clone(), op)
                                    })
                                })
                                .collect();
                            let merged: Vec<String> = ops
                                .iter()
                                .filter(|pair| pair.1["op"] == "merge")
                                .map(|pair| format!("{}/{}", pair.0, text(&pair.1["memory"])))
                                .collect();
                            let again: BTreeSet<String> = merged
                                .iter()
                                .map(|file| {
                                    (file, merged.iter().filter(|other| *other == file).count())
                                })
                                .filter(|pair| pair.1 > 1)
                                .map(|pair| format!("{} \u{b7} merged {} times", pair.0, pair.1))
                                .collect();
                            // what an agent had to remember by hand: the trace, live or moved aside
                            let remembered: Vec<String> = mine("remember")
                                .into_iter()
                                .map(|line| {
                                    let file = text(&line["file"]);
                                    let (bank, stem) = file
                                        .strip_prefix("traces/")
                                        .and_then(|rest| rest.split_once('/'))
                                        .unwrap_or_default();
                                    let aside = format!("-{bank}-{stem}");
                                    let found = iter::once(data.join(file))
                                        .chain(
                                            pruned
                                                .iter()
                                                .filter(|path| name(path).ends_with(&aside))
                                                .cloned(),
                                        )
                                        .find_map(|path| fs::read_to_string(path).ok())
                                        .unwrap_or_default();
                                    let trace = read(json!({}), found);
                                    format!(
                                        "{} \u{2014} {} ({file})",
                                        text(&trace["name"]),
                                        text(&trace["description"])
                                    )
                                })
                                .collect();
                            // recall's feedback: sent again and again, never read in the session
                            let shown: BTreeSet<(&str, &str)> = mine("show")
                                .into_iter()
                                .map(|line| (text(&line["session"]), text(&line["file"])))
                                .collect();
                            let sends: Vec<(&str, &str)> =
                                mine("recall")
                                    .into_iter()
                                    .flat_map(|line| {
                                        line["sent"].as_array().into_iter().flatten().map(
                                            move |one| (text(&line["session"]), text(&one["file"])),
                                        )
                                    })
                                    .collect();
                            let unread: BTreeSet<String> = sends
                                .iter()
                                .filter(|pair| {
                                    !sends
                                        .iter()
                                        .any(|other| other.1 == pair.1 && shown.contains(other))
                                })
                                .map(|pair| {
                                    (
                                        pair.1,
                                        sends.iter().filter(|other| other.1 == pair.1).count(),
                                    )
                                })
                                .filter(|pair| pair.1 >= 3)
                                .map(|pair| {
                                    format!("{} \u{b7} sent {} times, never shown", pair.0, pair.1)
                                })
                                .collect();
                            // the ladder's evidence, per note with an id that no proposal holds open
                            // and none was dropped for since its body last changed: used in so many
                            // sessions and never blamed (rung 2), or relearned by so many sessions'
                            // traces merged into it since it was written (rung 3)
                            let held = intentions(&data);
                            let (mut standing, mut relearned) = (BTreeMap::new(), BTreeMap::new());
                            for note in [notes(&data, "memories"), notes(&data, "knowledge")]
                                .concat()
                                .into_iter()
                                .filter(|note| {
                                    note["id"].is_string()
                                        && !held.iter().any(|one| {
                                            one["graduate"] == note["id"]
                                                && (one["state"] != "dropped"
                                                    || one["closed"].as_str() > when(note))
                                        })
                                })
                            {
                                let verdicts: Vec<&Value> = lines
                                    .iter()
                                    .filter(|line| {
                                        line["kind"] == "rethink"
                                            && (line["file"] == note["path"]
                                                || line["id"] == note["id"])
                                    })
                                    .collect();
                                let used: BTreeSet<&str> = verdicts
                                    .iter()
                                    .filter(|line| line["verdict"] == "used")
                                    .filter_map(|line| line["session"].as_str())
                                    .collect();
                                let rejoined: BTreeSet<&str> = lines
                                    .iter()
                                    .filter(|line| {
                                        line["kind"] == "rewrite"
                                            && line["op"] == "merge"
                                            && line["to"]["id"] == note["id"]
                                            && line["at"].as_str() > note["created"].as_str()
                                    })
                                    .flat_map(|line| line["from"].as_array().into_iter().flatten())
                                    .filter(|from| text(&from["path"]).starts_with("traces/"))
                                    .flat_map(|from| {
                                        from["sessions"].as_array().into_iter().flatten()
                                    })
                                    .filter_map(Value::as_str)
                                    .collect();
                                let listing = |count: usize, how: &str| {
                                    format!(
                                        "{} \u{b7} {} \u{b7} {} \u{2014} {} \u{b7} {how} in {count} sessions",
                                        text(&note["id"]),
                                        text(&note["path"]),
                                        text(&note["name"]),
                                        one_line(text(&note["description"]), 160)
                                    )
                                };
                                let id = text(&note["id"]).to_owned();
                                if used.len() >= setting("SANDMAN_GUIDE_SESSIONS", 5)
                                    && !verdicts.iter().any(|line| {
                                        ["stale", "wrong"].contains(&text(&line["verdict"]))
                                    })
                                {
                                    drop(standing.insert(
                                        id.clone(),
                                        (listing(used.len(), "used"), note.clone()),
                                    ));
                                }
                                if rejoined.len() >= setting("SANDMAN_MECHANISM_SESSIONS", 3) {
                                    drop(relearned.insert(
                                        id,
                                        (listing(rejoined.len(), "relearned"), note.clone()),
                                    ));
                                }
                            }
                            let input = format!(
                                "<guidance job=\"encode\">\n{}\n</guidance>\n<guidance job=\"consolidate\">\n{}\n</guidance>\n\n# since\n{}\n\n{}",
                                guide(&data, "encode").trim_end(),
                                guide(&data, "consolidate").trim_end(),
                                last["at"].as_str().unwrap_or("the start"),
                                cut(
                                    sections(&[
                                        (
                                            "runs",
                                            after
                                                .iter()
                                                .filter(ran)
                                                .map(Value::to_string)
                                                .collect()
                                        ),
                                        (
                                            "ops",
                                            ops.iter()
                                                .map(|pair| format!("{}: {}", pair.0, pair.1))
                                                .collect()
                                        ),
                                        ("merged again", again.into_iter().collect()),
                                        ("remembered by hand", remembered),
                                        ("sent, never read", unread.into_iter().collect()),
                                        (
                                            "standing",
                                            standing.values().map(|pair| pair.0.clone()).collect()
                                        ),
                                        (
                                            "relearned",
                                            relearned.values().map(|pair| pair.0.clone()).collect()
                                        ),
                                    ]),
                                    evidence
                                )
                            );
                            let reply = run(
                                &data,
                                "reflect",
                                None,
                                &reflector,
                                &input,
                                reflect_limit,
                                &noted,
                            )?;
                            let jobs = ["consolidate", "encode"];
                            // a proposal per note the evidence lists on its rung, at most once
                            let (mut proposed, rungs) = (BTreeSet::new(), [&standing, &relearned]);
                            let fits = |one: &Value| {
                                let rung = rungs
                                    .get(usize::from(one["rung"] == 3u8))
                                    .filter(|_| one["rung"] == 2u8 || one["rung"] == 3u8);
                                ["artifact", "line", "note", "why"]
                                    .iter()
                                    .all(|field| one[field].is_string())
                                    && (one["replaces"].is_null() || one["replaces"].is_string())
                                    && rung.is_some_and(|listed| {
                                        listed.contains_key(text(&one["note"]))
                                    })
                                    && proposed.insert(text(&one["note"]).to_owned())
                            };
                            if !reply["reason"].is_string()
                                || jobs
                                    .iter()
                                    .any(|job| !(reply[job].is_null() || reply[job].is_string()))
                                || !(reply["propose"].is_null()
                                    || reply["propose"]
                                        .as_array()
                                        .is_some_and(|list| list.iter().all(fits)))
                            {
                                return Err(io::Error::other(format!(
                                    "output: not a proposal: {}",
                                    one_line(&reply.to_string(), 200)
                                )));
                            }
                            if let Some(job) =
                                jobs.iter().find(|job| text(&reply[**job]).len() > ceiling)
                            {
                                return Err(io::Error::other(format!(
                                    "output: {job} guidance over {ceiling} bytes"
                                )));
                            }
                            // each proposal replayed: the old and the new on the same recent
                            // inputs, writing nothing, one judge per input under labels in a random
                            // order; adopted unless the old wins one
                            let (mut judged, mut adopted) = (Map::new(), Vec::new());
                            for job in jobs {
                                let Some(new) = reply[job].as_str() else {
                                    continue;
                                };
                                let (model_command, limit, inputs): (&str, Duration, Vec<String>) =
                                    if job == "encode" {
                                        (
                                            &encoder,
                                            encode_limit,
                                            lines
                                                .iter()
                                                .rev()
                                                .filter(|line| {
                                                    line["kind"] == "encode"
                                                        && line["outcome"] == "encoded"
                                                        && line["machine"] == this
                                                })
                                                .filter_map(|line| {
                                                    story.get(text(&line["session"]))
                                                })
                                                .filter(|marks| {
                                                    Path::new(text(&marks[1]["archived"])).is_file()
                                                })
                                                .take(replays)
                                                .map(|marks| {
                                                    episode(&data, &lines, &marks[1], budget).0
                                                })
                                                .collect(),
                                        )
                                    } else {
                                        // a past consolidate rebuilt: the traces it consumed and the
                                        // memories it changed as they stood, beside the bank's others
                                        // now, less those its keeps wrote
                                        (
                                            &consolidator,
                                            consolidate_limit,
                                            replies
                                                .iter()
                                                .rev()
                                                .take(replays)
                                                .map(|path| {
                                                    let stem = name(path)
                                                        .strip_suffix("ops.json")
                                                        .unwrap_or_default();
                                                    let applied = ops_of(path);
                                                    let aside = |field: &str| -> Vec<Value> {
                                                        applied
                                                            .iter()
                                                            .filter_map(|op| op[field].as_str())
                                                            .filter_map(|file| {
                                                                let whole = fs::read_to_string(
                                                                    path.with_file_name(format!(
                                                                        "{stem}{file}"
                                                                    )),
                                                                )
                                                                .ok()?;
                                                                Some(read(
                                                                    json!({"file": file}),
                                                                    whole,
                                                                ))
                                                            })
                                                            .collect()
                                                    };
                                                    let (consumed, mut stood) =
                                                        (aside("trace"), aside("memory"));
                                                    let bank = bank_of(path);
                                                    let others: Vec<Value> = shelf(&data, &bank)
                                                        .1
                                                        .into_iter()
                                                        .filter(|memory| {
                                                            !stood.iter().any(|earlier| {
                                                                earlier["file"] == memory["file"]
                                                            }) && !consumed.iter().any(|trace| {
                                                                trace["created"]
                                                                    == memory["created"]
                                                                    && trace["session"]
                                                                        == memory["session"]
                                                            })
                                                        })
                                                        .collect();
                                                    stood.extend(others);
                                                    stack(
                                                        &bank,
                                                        &consumed.iter().collect::<Vec<_>>(),
                                                        &sorted(stood, "file"),
                                                        allowance,
                                                        &[],
                                                    )
                                                    .0
                                                })
                                                .collect(),
                                        )
                                    };
                                // new, same, old
                                let mut tally = [0u8; 3];
                                let judge = |asked: &str| {
                                    run(
                                        &data,
                                        "judge",
                                        None,
                                        &reflector,
                                        asked,
                                        reflect_limit,
                                        &noted,
                                    )
                                };
                                for replayed in &inputs {
                                    let ask = |given: Option<&str>| {
                                        run(
                                            &data,
                                            job,
                                            given,
                                            model_command,
                                            replayed,
                                            limit,
                                            &noted,
                                        )
                                    };
                                    let pair = [ask(None)?, ask(Some(new))?];
                                    let flip = usize::from(random().as_str() > "8");
                                    let asked = format!(
                                        "# job\n{job}\n\n# input\n{replayed}\n# a\n{}\n\n# b\n{}\n",
                                        pair[flip],
                                        pair[flip ^ 1]
                                    );
                                    let verdict = judge(&asked)?;
                                    let pick = ["a", "b", "same"]
                                        .iter()
                                        .position(|label| verdict["better"] == *label)
                                        .ok_or_else(|| {
                                            io::Error::other(format!(
                                                "output: not a verdict: {}",
                                                one_line(&verdict.to_string(), 200)
                                            ))
                                        })?;
                                    let at = [[2usize, 0, 1], [0, 2, 1]][flip][pick];
                                    tally[at] = tally[at].saturating_add(1);
                                }
                                drop(judged.insert(
                                    job.to_owned(),
                                    json!({"new": tally[0], "old": tally[2], "same": tally[1]}),
                                ));
                                if !inputs.is_empty() && tally[2] == 0 {
                                    adopted.push((job, new.to_owned()));
                                }
                            }
                            let history = adopt(&data, &adopted)?;
                            // each proposal an intention due today, about the artifact's directory;
                            // a rung-2 line that replaces none and says nothing of its cost dropped
                            let (now, deadline) = (utc(SystemTime::now()), today());
                            let mut written = 0usize;
                            for (one, subject) in reply["propose"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter(|one| {
                                    one["rung"] == 3u8
                                        || one["replaces"].is_string()
                                        || !text(&one["why"]).trim().is_empty()
                                })
                                .filter_map(|one| {
                                    let rung = rungs[usize::from(one["rung"] == 3u8)];
                                    Some((one, &rung.get(text(&one["note"]))?.1))
                                })
                            {
                                let (artifact, minted) = (text(&one["artifact"]), random());
                                let about = Path::new(artifact).parent().map(Path::to_string_lossy);
                                let tail = |field: &str, lead: &str| {
                                    Some(text(&one[field]).trim())
                                        .filter(|said| !said.is_empty())
                                        .map_or_else(String::new, |said| format!("{lead}{said}"))
                                };
                                let what = format!(
                                    "Graduate {} (rung {}) into {artifact}: {}{}{}",
                                    text(&subject["name"]),
                                    one["rung"],
                                    text(&one["line"]),
                                    tail("replaces", ", replacing: "),
                                    tail("why", " \u{2014} ")
                                );
                                let id = minted.get(..8).unwrap_or_default();
                                let proposal = note(
                                    &[
                                        ("due", &deadline),
                                        ("created", &now),
                                        ("about", &about.unwrap_or_default()),
                                        ("graduate", text(&subject["id"])),
                                        ("artifact", artifact),
                                        ("state", "open"),
                                    ],
                                    &what,
                                );
                                put(&data.join(format!("intentions/{id}.md")), &proposal)?;
                                drop(record(
                                    &data,
                                    &mut json!({"due": deadline, "graduate": subject["id"], "id": id, "kind": "defer", "session": null}),
                                ));
                                written = written.saturating_add(1);
                            }
                            Ok(json!({
                                "outcome": (["kept", "adopted"][usize::from(!history.is_empty())]),
                                "proposed": written, "reason": one_line(text(&reply["reason"]), 400), "replay": judged, "history": history,
                            }))
                        };
                        // the review's line carries the store's order before decay and after the pass
                        let reflect = || -> io::Result<Value> {
                            let before = measure(&data, None);
                            let pruned = decay()?;
                            let mut line = review().or_else(|error| match STOP.get() {
                                Some(_) => Err(error),
                                None => Ok(json!({"error": one_line(&error.to_string(), 400), "history": {}, "outcome": "failed"})),
                            })?;
                            (line["order"], line["pruned"]) = (
                                json!({"after": measure(&data, None), "before": before, "merges": 0u8}),
                                pruned.into(),
                            );
                            Ok(line)
                        };
                        (
                            "reflect",
                            "kind",
                            "reflect".to_owned(),
                            json!({"history": {}}),
                            reflect(),
                        )
                    } else {
                        continue;
                    };
                    let mut line = match done {
                        Ok(line) => line,
                        // stopped: the group killed, no line; the job runs again after the next start
                        Err(_) if STOP.get().is_some() => continue,
                        Err(error) => {
                            (zero["error"], zero["outcome"]) =
                                (one_line(&error.to_string(), 400).into(), "failed".into());
                            zero
                        }
                    };
                    (line["kind"], line[key]) = (kind.into(), value.as_str().into());
                    if let Some(token) = noted.take() {
                        line["token"] = token.into();
                    }
                    logged(kind, key, &value, record(&data, &mut line).map(|()| line));
                }
            });
            let health = json!({
                "machine": machine(&root), "ok": true, "pid": id(), "since": utc(SystemTime::now()), "state": "ready", "version": VERSION,
            });
            write(json!({"event": "start", "pid": id(), "version": VERSION}));
            // the ready line: whoever spawned the daemon reads it, so nothing polls
            println!("{health}");
            let reason = loop {
                let (mut conn, _) = match listener.accept().and_then(|pair| {
                    STOP.get().map_or(Ok(pair), |signal| {
                        Err(io::Error::other(format!("signal {signal}")))
                    })
                }) {
                    Ok(pair) => pair,
                    Err(reason) => break reason.to_string(),
                };
                let mut line = String::new();
                if conn
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .and_then(|()| BufReader::new(&conn).read_line(&mut line))
                    .is_err()
                {
                    continue;
                }
                let reply = match serde_json::from_str::<Value>(&line).unwrap_or_default()["op"]
                    .as_str()
                {
                    Some("health") => health.clone(),
                    Some("shutdown") => {
                        drop(listener);
                        drop(writeln!(conn, "{}", json!({"ok": true})));
                        break "shutdown request".to_owned();
                    }
                    op => json!({"code": op.map_or("bad_request", |_| "unknown_op"), "ok": false}),
                };
                drop(writeln!(conn, "{reply}"));
            };
            // the tick finishes the job in hand and leaves its loop, starting no other
            STOP.set("").unwrap_or_default();
            ticker.thread().unpark();
            drop(ticker.join());
            let pid = rundir.join("serve.pid");
            if fs::read_to_string(&pid).is_ok_and(|text| text == id().to_string()) {
                drop(fs::remove_file(&pid));
                drop(fs::remove_file(&sock));
            }
            write(json!({"event": "exit", "reason": reason}));
            ExitCode::SUCCESS
        }
        "setup" => {
            // hotswap's link through its `current`, never this executable: a checkout's setup
            // wires the installed release
            let exe = home.join(".local/bin/sandman");
            let plist = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>EnvironmentVariables</key>
  <dict><key>PATH</key><string>{}</string><key>SANDMAN_ROOT</key><string>{}</string></dict>
  <key>ExitTimeOut</key>
  <integer>2</integer>
  <key>KeepAlive</key>
  <true/>
  <key>Label</key>
  <string>dev.gigantic.sandman</string>
  <key>ProgramArguments</key>
  <array><string>{}</string><string>serve</string></array>
  <key>RunAtLoad</key>
  <true/>
  <key>StandardErrorPath</key>
  <string>{}</string>
  <key>ThrottleInterval</key>
  <integer>10</integer>
</dict>
</plist>
"#,
                // launchd's own PATH holds no `claude`: the encoder finds it on the caller's
                env::var("PATH").unwrap_or_default(),
                root.display(),
                exe.display(),
                // beside run/, not in it: launchd creates the parent of this path 0744, which serve refuses
                root.join("serve.err").display()
            );
            let label = "dev.gigantic.sandman";
            let agents = home.join("Library/LaunchAgents");
            let install = || -> io::Result<()> {
                if args.iter().any(|arg| arg == "--print") {
                    print!("{plist}");
                    return Ok(());
                }
                let uid = Command::new("id").arg("-u").output()?.stdout;
                let gui = format!("gui/{}", String::from_utf8_lossy(&uid).trim());
                // resolved from PATH, as curl is; none (Linux) fails before anything is written
                let call = |words: &[&str]| {
                    Command::new("launchctl").args(words).output().map_err(|error| {
                        io::Error::other(format!("launchctl: {error} (setup installs a launchd agent, macOS only; --print prints the plist)"))
                    })
                };
                let must = |words: &[&str]| {
                    let out = call(words)?;
                    let line = words.join(" ");
                    if !out.status.success() {
                        let cause = one_line(&String::from_utf8_lossy(&out.stderr), 200);
                        return Err(io::Error::other(format!("launchctl {line}: {cause}")));
                    }
                    println!("launchctl {line}");
                    Ok(())
                };
                let loaded = |job: &str| {
                    Ok::<bool, io::Error>(
                        call(&["print", &format!("{gui}/{job}")])?.status.success(),
                    )
                };
                // a bootout returns before launchd lets go of the job, and a bootstrap before then fails
                let limit = Duration::from_millis(setting("SANDMAN_BOOTOUT_MS", 30_000));
                let unload = |job: &str| -> io::Result<()> {
                    if loaded(job)? {
                        must(&["bootout", &format!("{gui}/{job}")])?;
                    }
                    let begun = Instant::now();
                    while loaded(job)? {
                        if begun.elapsed() > limit {
                            return Err(io::Error::other(format!(
                                "{job} still loaded {} ms after its bootout",
                                limit.as_millis()
                            )));
                        }
                        thread::sleep(Duration::from_millis(100));
                    }
                    Ok(())
                };
                let remove = |job: &str| -> io::Result<()> {
                    unload(job)?;
                    let path = agents.join(format!("{job}.plist"));
                    if path.exists() {
                        fs::remove_file(&path)?;
                        println!("removed {}", path.display());
                    }
                    Ok(())
                };
                if args.iter().any(|arg| arg == "--remove") {
                    // first: it refuses while a swap runs, before anything is booted out
                    hotswap::forget("sandman").map_err(io::Error::other)?;
                    remove(label)?;
                    if fs::remove_file(&exe).is_ok() {
                        println!("removed {}", exe.display());
                    }
                    return Ok(());
                }
                for job in RETIRED {
                    remove(job)?;
                }
                // the release into hotswap's layout, the link onto its `current`, and the upgrade
                // agent, hotswap's alone
                let linked = hotswap::ensure("sandman", &manifest()).map_err(io::Error::other)?;
                println!(
                    "linked {} \u{2192} {}",
                    linked.display(),
                    fs::canonicalize(&linked)?.display()
                );
                let path = agents.join(format!("{label}.plist"));
                let same = fs::read_to_string(&path).is_ok_and(|held| held == plist);
                put(&path, &plist)?;
                println!("wrote {}", path.display());
                // the same plist loaded: a swap is one kickstart; else it is loaded afresh
                if same && loaded(label)? {
                    return must(&["kickstart", "-k", &format!("{gui}/{label}")]);
                }
                unload(label)?;
                must(&["bootstrap", &gui, &path.to_string_lossy()])
            };
            install().map_or_else(
                |error| {
                    eprintln!("sandman setup: {error}");
                    ExitCode::FAILURE
                },
                |()| ExitCode::SUCCESS,
            )
        }
        "upgrade" => u8::try_from(hotswap::upgrade(
            "sandman",
            &manifest(),
            args.get(2..).unwrap_or_default(),
        ))
        .map_or(ExitCode::FAILURE, ExitCode::from),
        "show" => {
            let wanted = args.get(2).cloned().unwrap_or_default();
            let Some(memory) = every(&root)
                .into_iter()
                .find(|note| note["path"] == wanted.as_str())
            else {
                eprintln!(
                    "sandman show: no memory {wanted} (want [traces/]<bank>/<file>.md or knowledge/<topic>/<file>.md under {})",
                    root.display()
                );
                return ExitCode::FAILURE;
            };
            println!(
                "{} ({}{}) \u{b7} {}\n{}",
                text(&memory["name"]),
                text(&memory["type"]),
                if wanted.starts_with("traces/") {
                    " \u{b7} trace"
                } else {
                    ""
                },
                when(&memory).unwrap_or("undated"),
                text(&memory["body"])
            );
            drop(record(
                &root,
                &mut json!({"file": wanted, "id": memory["id"], "kind": "show", "session": pull}),
            ));
            ExitCode::SUCCESS
        }
        "status" => {
            let reply = UnixStream::connect(root.join("run/serve.sock"))
                .and_then(|mut stream| {
                    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
                    writeln!(stream, r#"{{"op":"health"}}"#)?;
                    let mut line = String::new();
                    BufReader::new(stream).read_line(&mut line).map(|_| line)
                })
                .ok()
                .and_then(|line| serde_json::from_str::<Value>(&line).ok())
                .filter(|health| health["ok"] == true);
            let Some(health) = reply else {
                println!("{}", json!({"code": "down", "ok": false}));
                return ExitCode::FAILURE;
            };
            if health["version"] != VERSION {
                eprintln!(
                    "sandman status: the daemon is {}, this client {VERSION}; swap it: {me} setup",
                    health["version"].as_str().unwrap_or_default()
                );
            }
            println!("{health}");
            ExitCode::SUCCESS
        }
        "version" => {
            println!("{VERSION}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!(
                "usage: sandman mark < {{\"kind\":\"start\"|\"end\"|\"guard\",\"session\":\"<uuid>\",...}}\n       sandman take < {{\"session\":\"<uuid>\",...}}\n       sandman recall < {{\"session\":\"<uuid>\",...}} | --cue \"<text>\" [--about <path>] [--session <uuid>] [--budget <chars>]\n       sandman remember \"<body>\" [--type <type>] [--topic <topic>] [--name <name>] [--description <line>] [--about <path>] [--session <uuid>]\n       sandman show <path> [--session <uuid>]\n       sandman rethink <path> --irrelevant|--used|--wrong|--stale [--why \"<text>\"] [--session <uuid>]\n       sandman defer \"<what>\" --when <when> [--about <path>] [--session <uuid>] | --id <id> --when <when>\n       sandman done|drop <id> [--note \"<text>\"] [--session <uuid>]\n       sandman due [--all]\n       sandman forget <session>\n       sandman setup [--print | --remove] | upgrade [--check | --back | --retry]\n       sandman check [--fix] | chart | mcp | serve | status | version"
            );
            ExitCode::from(2)
        }
    }
}
