//! `sandman`, rewritten from scratch: `mark` appends a session's start or end to the day's ledger
//! file, `take` archives a marked session at its end, `chart` prints what the data root holds as one JSON object, `serve` is the daemon,
//! `status` asks it for its health, `setup` prints its launchd plist.

use serde_json::{Map, Value, json};
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use signal_hook::low_level::signal_name;
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsStr;
use std::fs::{self, DirBuilder, File, OpenOptions, TryLockError};
use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
use std::iter;
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::panic;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, id};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Held by the tick for each take; the drain takes it after setting `STOP`, so the take in hand
/// finishes and no other starts.
static BUSY: Mutex<()> = Mutex::new(());

/// The stop signal's name (`TERM`, `INT`, `HUP`), set by the signal thread before it wakes `accept`.
static STOP: OnceLock<&str> = OnceLock::new();

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
            let at = utc(SystemTime::now());
            let day = root.join(format!(
                ".archive/claude/{}",
                at.get(..10).unwrap_or_default().replace('-', "/")
            ));
            let base = format!(
                "{}-{}",
                at.get(11..19).unwrap_or_default().replace(':', ""),
                path.strip_prefix(claude)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('/', "-")
            );
            // a same-second clash takes `<HHMMSS>-<n>-…`, never the earlier archive
            let name = iter::once(base.clone())
                .chain((2u32..1000).map(|n| base.replacen('-', &format!("-{n}-"), 1)))
                .find(|name| !day.join(name).exists())
                .unwrap_or_default();
            let (archived, ended) = (day.join(&name), utc(fs::metadata(&path)?.modified()?));
            fs::create_dir_all(&day)?;
            fs::rename(&path, &archived)?;
            let (sidecar, beside) = (
                path.with_extension(""),
                day.join(format!("{}-dir", name.trim_end_matches(".jsonl"))),
            );
            if sidecar.is_dir() {
                fs::rename(sidecar, beside)?;
            }
            let lines: Vec<Value> = String::from_utf8_lossy(&fs::read(&archived)?)
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect();
            let title = lines
                .iter()
                .filter(|line| line["type"] == "user")
                .filter_map(|line| {
                    let content = &line["message"]["content"];
                    content.as_str().or_else(|| {
                        content
                            .as_array()?
                            .iter()
                            .find(|block| block["type"] == "text")?["text"]
                            .as_str()
                    })
                })
                .map(|said| one_line(said, 80))
                .find(|title| !title.is_empty())
                .unwrap_or_else(|| session.to_owned());
            let mut highlights: Vec<String> = Vec::new();
            // only commands the session ran: an assistant's tool_use, a user's `!` line
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
                                            chars =
                                                chars.as_str().get(1..).unwrap_or_default().chars();
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
            let found = lines
                .iter()
                .map(|line| &line["cwd"])
                .chain([cwd])
                .filter_map(Value::as_str)
                .find(|dir| !dir.is_empty() && !dir.contains(char::is_control));
            let recent = root.join("memories/.recent");
            let tmp = recent.join(format!(".{session}.json.{}", id()));
            fs::create_dir_all(&recent)?;
            let pointer = format!(
                "{{\"archived\":{},\"cwd\":{},\"ended\":{},\"title\":{},\"highlights\":{}}}\n",
                json!(archived.to_string_lossy()),
                json!(found),
                json!(ended),
                json!(title),
                json!(highlights.split_off(highlights.len().saturating_sub(20)))
            );
            fs::write(&tmp, pointer)?;
            fs::rename(tmp, recent.join(format!("{session}.json")))?;
            json!({"archived": archived.to_string_lossy(), "outcome": "taken"})
        }
    };
    line["cwd"] = cwd.clone();
    line["kind"] = "end".into();
    line["reason"] = reason.clone();
    line["session"] = session.into();
    record(root, &mut line).map(|()| line)
}

/// The background job under `<claude>/jobs/` that names `session`: its `resumeSessionId`, else
/// its `sessionId` (a `/clear` moves a job on to a new id and leaves the old one in `sessionId`).
fn job(claude: &Path, session: &str) -> Option<String> {
    tree(&claude.join("jobs"), 1).into_iter().find_map(|dir| {
        let state: Value =
            serde_json::from_str(&fs::read_to_string(dir.join("state.json")).ok()?).ok()?;
        (state["resumeSessionId"]
            .as_str()
            .or_else(|| state["sessionId"].as_str())
            == Some(session))
        .then(|| name(&dir).to_owned())
    })
}

/// Every line of `<root>/ledger/*/*/*.jsonl` that parses, oldest file first.
fn ledger(root: &Path) -> Vec<Value> {
    tree(&root.join("ledger"), 3)
        .iter()
        .flat_map(|file| {
            fs::read_to_string(file)
                .unwrap_or_default()
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// This machine's id: `$SANDMAN_MACHINE`, else `<root>/machine`, created on first use by a hard
/// link from a temp file, so concurrent first runs agree on one id.
fn machine(root: &Path) -> String {
    env::var("SANDMAN_MACHINE").unwrap_or_else(|_| {
        let path = root.join("machine");
        let read = || fs::read_to_string(&path).map(|text| text.trim().to_owned());
        read().unwrap_or_else(|_| {
            let mut bytes = [0u8; 16];
            drop(File::open("/dev/urandom").and_then(|mut random| random.read_exact(&mut bytes)));
            let hex = bytes.map(|byte| format!("{byte:02x}")).concat();
            let tmp = root.join(format!(".machine.{}", id()));
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

/// A path's last component, or "".
fn name(path: &Path) -> &str {
    path.file_name().and_then(OsStr::to_str).unwrap_or_default()
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

/// Appends `line`, stamped with `at` and `machine`, to today's ledger file with `O_APPEND`.
fn record(root: &Path, line: &mut Value) -> io::Result<()> {
    let at = utc(SystemTime::now());
    let day = root.join(format!(
        "ledger/{}.jsonl",
        at.get(..10).unwrap_or_default().replace('-', "/")
    ));
    line["at"] = at.into();
    line["machine"] = machine(root).into();
    day.parent().map_or(Ok(()), fs::create_dir_all)?;
    OpenOptions::new()
        .append(true)
        .create(true)
        .open(&day)?
        .write_all(format!("{line}\n").as_bytes())
}

/// `rows`, stably sorted by the string at `key` (a null first).
fn sorted(mut rows: Vec<Value>, key: &str) -> Vec<Value> {
    rows.sort_by(|left, right| left[key].as_str().cmp(&right[key].as_str()));
    rows
}

/// `<claude>/projects/*/<session>.jsonl`, the first project's that holds one.
fn transcript(claude: &Path, session: &str) -> Option<PathBuf> {
    tree(&claude.join("projects"), 1)
        .into_iter()
        .map(|dir| dir.join(format!("{session}.jsonl")))
        .find(|path| path.is_file())
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

fn main() -> ExitCode {
    let home = PathBuf::from(env::var_os("HOME").unwrap_or_default());
    let root = env::var_os("SANDMAN_ROOT").map_or_else(|| home.join(".sandman"), PathBuf::from);
    let claude =
        env::var_os("CLAUDE_CONFIG_DIR").map_or_else(|| home.join(".claude"), PathBuf::from);
    match env::args().nth(1).as_deref() {
        Some(verb @ ("mark" | "take")) => {
            let mut text = String::new();
            drop(io::stdin().read_to_string(&mut text));
            let input: Map<String, Value> = serde_json::from_str(&text).unwrap_or_default();
            let field = |key: &str| input.get(key).cloned().unwrap_or_default();
            let (session, reason) = (field("session"), field("reason"));
            let Some(id) = session.as_str().filter(|id| uuid(id)).filter(|_| {
                verb == "take" || matches!(field("kind").as_str(), Some("start" | "end"))
            }) else {
                eprintln!(
                    r#"sandman {verb}: stdin is not {{{}"session":"<uuid>",...}}"#,
                    if verb == "mark" {
                        r#""kind":"start"|"end","#
                    } else {
                        ""
                    }
                );
                return ExitCode::from(2);
            };
            let skip = env::var_os("SANDMAN_NO_TAKE").is_some_and(|value| !value.is_empty())
                || env::var("CLAUDE_MEMORY_PIPELINE").as_deref() == Ok("1");
            let done = if verb == "mark" {
                record(
                    &root,
                    &mut json!({
                        "cwd": field("cwd"), "interactive": field("interactive"), "kind": field("kind"),
                        "reason": reason, "resume": field("resume"), "session": session, "surface": field("surface"),
                    }),
                )
            } else if reason == "resume"
                || !ledger(&root)
                    .iter()
                    .any(|mark| mark["session"] == id && mark["kind"] == "start")
            {
                Ok(())
            } else {
                let decided = skip.then(|| json!({"outcome": "skipped"})).or_else(|| {
                    job(&claude, id)
                        .filter(|_| reason != "clear")
                        .map(|job| json!({"job": job, "outcome": "held"}))
                });
                end(&root, &claude, id, &field("cwd"), &reason, decided).map(drop)
            };
            match done {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("sandman {verb}: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("chart") => {
            let mut marks: BTreeMap<String, (Value, Value)> = BTreeMap::new();
            for mark in ledger(&root) {
                if let (Some(session), Some(kind)) =
                    (mark["session"].as_str(), mark["kind"].as_str())
                {
                    let pair = marks.entry(session.to_owned()).or_default();
                    let slot = if kind == "end" {
                        &mut pair.1
                    } else {
                        &mut pair.0
                    };
                    // the earliest start (a reload re-fires it), the latest end
                    if slot.is_null()
                        || (mark["at"].as_str() < slot["at"].as_str()) == (kind != "end")
                    {
                        *slot = mark;
                    }
                }
            }
            let memories: Vec<Value> = tree(&root.join("memories"), 2)
                .iter()
                .filter_map(|path| {
                    let (bank, file) = (path.parent().map_or("", name), name(path));
                    let skip = [bank, file].iter().any(|part| part.starts_with(['.', '_']));
                    (!skip && file != "MEMORY.md" && path.extension() == Some("md".as_ref()) && path.is_file())
                        .then_some((path, bank, file))
                })
                .map(|(path, bank, file)| {
                    let text = fs::read_to_string(path).unwrap_or_default();
                    let front: BTreeMap<&str, &str> = text
                        .strip_prefix("---\n")
                        .and_then(|rest| rest.split_once("\n---"))
                        .map_or("", |(head, _)| head)
                        .lines()
                        .filter_map(|line| line.split_once(':'))
                        .map(|(key, value)| (key.trim(), value.trim()))
                        .collect();
                    let source = front.get("source").copied().unwrap_or_default();
                    let kind = source
                        .split_whitespace()
                        .next()
                        .filter(|word| ["dream", "reflect", "remember"].contains(word))
                        .unwrap_or("other");
                    json!({
                        "bank": bank, "created": front.get("created"), "file": file,
                        "name": front.get("name").copied().unwrap_or_else(|| path.file_stem().and_then(OsStr::to_str).unwrap_or_default()),
                        "session": source.split_whitespace().find(|word| kind == "dream" && uuid(word)),
                        "source": kind, "type": front.get("type").copied().unwrap_or_default(),
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
                    "memories": memories.iter().filter(|memory| memory["session"] == session).count(),
                    "session": session, "state": state,
                    "taken": format!("{yyyy}-{mm}-{dd}T{}Z", clock.join(":")), "title": pointer["title"],
                })));
            }
            let active = marks
                .into_iter()
                .map(|(session, (start, end))| {
                    let from = if start.is_null() { &end } else { &start };
                    json!({
                        "cwd": from["cwd"], "end": end["at"], "machine": from["machine"],
                        "outcome": end["outcome"], "session": session, "start": start["at"],
                    })
                })
                .collect();
            // a closed stdout (`| head`) ends the chart quietly
            drop(writeln!(
                io::stdout().lock(),
                "{}",
                json!({
                    "active": sorted(active, "start"), "at": utc(SystemTime::now()), "machine": machine(&root),
                    "memories": memories, "taken": sorted(taken.into_values().collect(), "taken"),
                })
            ));
            ExitCode::SUCCESS
        }
        Some("serve") => {
            let run = root.join("run");
            let (log, sock, tmp) = (
                run.join("serve.log"),
                run.join("serve.sock"),
                run.join("serve.new"),
            );
            let started = || -> io::Result<_> {
                if sock.as_os_str().len() > 103 {
                    return Err(io::Error::other(format!(
                        "{} is over a unix socket's 103 bytes; shorten SANDMAN_ROOT",
                        sock.display()
                    )));
                }
                fs::create_dir_all(&root)?;
                drop(DirBuilder::new().mode(0o700).create(&run));
                // append, not create: a second instance must not even truncate the lock file
                let lock = OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(run.join("serve.lock"))?;
                match lock.try_lock() {
                    Err(TryLockError::WouldBlock) => {
                        return Err(io::Error::other("already running"));
                    }
                    other => other?,
                }
                drop(fs::remove_file(&tmp));
                let listener = UnixListener::bind(&tmp)?;
                let dir = fs::metadata(&run)?;
                if dir.uid() != fs::metadata(&tmp)?.uid() || dir.mode() & 0o077 != 0 {
                    fs::remove_file(&tmp)?;
                    return Err(io::Error::other(format!(
                        "{} must be this user's, mode 0700",
                        run.display()
                    )));
                }
                fs::rename(&tmp, &sock)?;
                fs::write(run.join("serve.pid"), id().to_string())?;
                Ok((lock, listener, Signals::new([SIGHUP, SIGINT, SIGTERM])?))
            };
            let (_lock, listener, mut signals) = match started() {
                Ok(held) => held,
                Err(error) => {
                    eprintln!("sandman serve: {error}");
                    return ExitCode::FAILURE;
                }
            };
            let number = |key: &str, default: u64| {
                env::var(key)
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(default)
            };
            let cap = number("SANDMAN_LOG_CAP", 1 << 20);
            let write = move |mut line: Value| {
                line["at"] = utc(SystemTime::now()).into();
                if fs::metadata(&log).is_ok_and(|meta| meta.len() >= cap) {
                    drop(fs::rename(&log, log.with_extension("log.1")));
                }
                drop(
                    OpenOptions::new()
                        .append(true)
                        .create(true)
                        .open(&log)
                        .and_then(|mut file| writeln!(file, "{line}")),
                );
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
            let tick = Duration::from_millis(number("SANDMAN_TICK_MS", 30_000));
            let quiet = Duration::from_millis(number("SANDMAN_QUIET_MS", 120_000));
            drop(thread::spawn(move || {
                while STOP.get().is_none() {
                    thread::sleep(tick);
                    // the latest line per session; a held end whose job is gone and whose
                    // transcript has been quiet for the window is taken
                    let latest: BTreeMap<String, Value> = ledger(&data)
                        .into_iter()
                        .filter_map(|entry| Some((entry["session"].as_str()?.to_owned(), entry)))
                        .collect();
                    for (session, held) in latest {
                        let _busy = BUSY.lock().unwrap_or_else(PoisonError::into_inner);
                        if held["kind"] != "end"
                            || held["outcome"] != "held"
                            || held["machine"] != this
                            || job(&claude, &session).is_some()
                            || transcript(&claude, &session)
                                .and_then(|path| {
                                    path.metadata().and_then(|meta| meta.modified()).ok()
                                })
                                .is_some_and(|time| time.elapsed().unwrap_or_default() < quiet)
                            || STOP.get().is_some()
                        {
                            continue;
                        }
                        let mut line = end(
                            &data,
                            &claude,
                            &session,
                            &held["cwd"],
                            &"job gone".into(),
                            None,
                        )
                        .unwrap_or_else(
                            |error| json!({"reason": error.to_string(), "session": session}),
                        );
                        line["event"] = "take".into();
                        settle(line);
                    }
                }
            }));
            let version = env!("CARGO_PKG_VERSION");
            let health = json!({
                "machine": machine(&root), "ok": true, "pid": id(), "since": utc(SystemTime::now()), "state": "ready", "version": version,
            });
            write(json!({"event": "start", "pid": id(), "version": version}));
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
            // the take in hand finishes; the tick starts no other
            STOP.set("").unwrap_or_default();
            let _busy = BUSY.lock().unwrap_or_else(PoisonError::into_inner);
            let pid = run.join("serve.pid");
            if fs::read_to_string(&pid).is_ok_and(|text| text == id().to_string()) {
                drop(fs::remove_file(&pid));
                drop(fs::remove_file(&sock));
            }
            write(json!({"event": "exit", "reason": reason}));
            ExitCode::SUCCESS
        }
        Some("setup") => {
            let exe = env::current_exe().unwrap_or_default();
            print!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>EnvironmentVariables</key>
  <dict><key>SANDMAN_ROOT</key><string>{}</string></dict>
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
                root.display(),
                exe.display(),
                // beside run/, not in it: launchd creates the parent of this path 0744, which serve refuses
                root.join("serve.err").display()
            );
            ExitCode::SUCCESS
        }
        Some("status") => {
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
            let version = env!("CARGO_PKG_VERSION");
            if health["version"] != version {
                eprintln!(
                    "sandman status: the daemon is {}, this client {version}; swap it: launchctl kickstart -k gui/$(id -u)/dev.gigantic.sandman",
                    health["version"].as_str().unwrap_or_default()
                );
            }
            println!("{health}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!(
                "usage: sandman mark < {{\"kind\":\"start\"|\"end\",\"session\":\"<uuid>\",...}}\n       sandman take < {{\"session\":\"<uuid>\",...}}\n       sandman chart | serve | setup | status"
            );
            ExitCode::from(2)
        }
    }
}
