//! `sandman`, rewritten from scratch: `mark` appends a session's start or end to the day's marks
//! file, `chart` prints what the data root holds as one JSON object.

use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{ExitCode, id};
use std::time::{SystemTime, UNIX_EPOCH};

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

/// Now as RFC 3339 UTC to the second (Howard Hinnant's `civil_from_days`).
#[expect(
    clippy::arithmetic_side_effects,
    clippy::integer_division,
    clippy::integer_division_remainder_used,
    reason = "calendar math on unsigned seconds since the epoch: nothing goes negative or overflows"
)]
fn now() -> String {
    let secs = SystemTime::now()
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

/// `rows`, stably sorted by the string at `key` (a null first).
fn sorted(mut rows: Vec<Value>, key: &str) -> Vec<Value> {
    rows.sort_by(|left, right| left[key].as_str().cmp(&right[key].as_str()));
    rows
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

/// Whether `text` is a 36-character session id.
fn uuid(text: &str) -> bool {
    text.len() == 36
        && text
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
}

fn main() -> ExitCode {
    let root = env::var_os("SANDMAN_ROOT").map_or_else(
        || PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".sandman"),
        PathBuf::from,
    );
    match env::args().nth(1).as_deref() {
        Some("mark") => {
            let mut text = String::new();
            drop(io::stdin().read_to_string(&mut text));
            let input: Map<String, Value> = serde_json::from_str(&text).unwrap_or_default();
            let field = |key: &str| input.get(key).cloned().unwrap_or_default();
            if !matches!(field("kind").as_str(), Some("start" | "end"))
                || !field("session").as_str().is_some_and(uuid)
            {
                eprintln!(
                    r#"sandman mark: stdin is not {{"kind":"start"|"end","session":"<uuid>",...}}"#
                );
                return ExitCode::from(2);
            }
            let at = now();
            let line = json!({
                "at": at, "cwd": field("cwd"), "interactive": field("interactive"),
                "kind": field("kind"), "machine": machine(&root), "reason": field("reason"),
                "resume": field("resume"), "session": field("session"), "surface": field("surface"),
            });
            let day = root.join(format!(
                "marks/{}.jsonl",
                at.get(..10).unwrap_or_default().replace('-', "/")
            ));
            match day
                .parent()
                .map_or(Ok(()), fs::create_dir_all)
                .and_then(|()| OpenOptions::new().append(true).create(true).open(&day))
                .and_then(|mut file| file.write_all(format!("{line}\n").as_bytes()))
            {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("sandman mark: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("chart") => {
            let mut marks: BTreeMap<String, (Value, Value)> = BTreeMap::new();
            for file in tree(&root.join("marks"), 3) {
                for mark in fs::read_to_string(file)
                    .unwrap_or_default()
                    .lines()
                    .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                {
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
                        "session": session, "start": start["at"],
                    })
                })
                .collect();
            // a closed stdout (`| head`) ends the chart quietly
            drop(writeln!(
                io::stdout().lock(),
                "{}",
                json!({
                    "active": sorted(active, "start"), "at": now(), "machine": machine(&root),
                    "memories": memories, "taken": sorted(taken.into_values().collect(), "taken"),
                })
            ));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!(
                "usage: sandman mark < {{\"kind\":\"start\"|\"end\",\"session\":\"<uuid>\",...}}\n       sandman chart"
            );
            ExitCode::from(2)
        }
    }
}
