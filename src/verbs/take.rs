//! `take` — archive a session by move, then drop its pointer.
//!
//! The move is the take: after it the conversation has left Claude Code's
//! live/resumable set, which is the feature rather than a side effect. Only a
//! rename is ever used — a copy would leave the session both taken and live,
//! so a cross-device destination is an error, not a fallback.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::atomic;
use crate::error::{Error, Result};
use crate::json::{self, Value};
use crate::paths;
use crate::time::Timestamp;
use crate::transcript::{self, Digest};

/// A transcript touched inside this window is treated as live. It is a
/// heuristic — Claude Code writes the last line at the end of the turn, not at
/// the end of the session — so `--force` overrides it.
pub const LIVE_WINDOW_SECONDS: i64 = 120;

/// Pointers waiting at or above this depth mean a dream is due.
pub const QUEUE_DEPTH_DUE: usize = 10;

/// What a take did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TakeOutcome {
    /// Where the transcript now lives.
    pub archived: PathBuf,
    /// Where the subagent directory now lives, when the session had one.
    pub archived_directory: Option<PathBuf>,
    /// How many pointers are queued after this one.
    pub queue_depth: usize,
    /// The pointer file.
    pub pointer: PathBuf,
}

impl TakeOutcome {
    /// Whether the queue has reached the depth that calls for a dream.
    #[must_use]
    pub fn dream_due(&self) -> bool {
        self.queue_depth >= QUEUE_DEPTH_DUE
    }
}

/// Archive `session_id` out of `claude_root` into `data_root`.
///
/// `force` skips the live check and nothing else.
pub fn take(
    data_root: &Path,
    claude_root: &Path,
    session_id: &str,
    force: bool,
) -> Result<TakeOutcome> {
    let found = transcript::find(claude_root, session_id)?;
    let Some(source) = found.transcripts.first().cloned() else {
        return Err(Error::not_found("transcript", session_id));
    };

    let now = Timestamp::now()?;
    let ended = modified_at(&source)?;
    if !force {
        let idle = now.unix_seconds() - ended.unix_seconds();
        if idle < LIVE_WINDOW_SECONDS {
            return Err(Error::refused(format!(
                "{session_id} looks live — {} touched {idle}s ago (under {LIVE_WINDOW_SECONDS}s); pass --force to take it anyway",
                source.display()
            )));
        }
    }

    let text = fs::read_to_string(&source).map_err(|error| Error::io(&source, error))?;
    let digest = transcript::digest(&text);

    let archive_dir = paths::archive_claude_dir(data_root);
    fs::create_dir_all(&archive_dir).map_err(|error| Error::io(&archive_dir, error))?;
    let name = archive_name(claude_root, &source, now);
    let archived = archive_dir.join(&name);
    rename(&source, &archived)?;

    let archived_directory = match found.directories.first() {
        Some(directory) => {
            let target = archive_dir.join(format!(
                "{}-dir",
                name.strip_suffix(".jsonl").unwrap_or(&name)
            ));
            rename(directory, &target)?;
            Some(target)
        }
        None => None,
    };

    let pointer = write_pointer(data_root, session_id, &archived, ended, &digest)?;
    let queue_depth = queue_depth(data_root)?;

    Ok(TakeOutcome {
        archived,
        archived_directory,
        queue_depth,
        pointer,
    })
}

/// Whether a background job still names `session_id` as its own.
///
/// Claude Code's daemon retires a background worker that has sat idle and done
/// for about an hour, and the retired process's exit fires `SessionEnd` with
/// an ordinary reason. That ending belongs to the worker, not to the
/// conversation: the job stays in the operator's list and is routinely picked
/// up again days later. The job's directory under `~/.claude/jobs` is the
/// proof — it outlives every worker and disappears only when the operator
/// deletes the job — so a session any `state.json` still names is one the take
/// has no business moving. Taken anyway, it pulled a 3.8 MB live conversation
/// out from under its own job and then took three recreated stubs as the
/// daemon re-settled (2026-08-26).
///
/// Every uncertainty reads as "no job": an absent or unreadable jobs
/// directory, an entry that is not a job, a `state.json` that is missing or
/// will not parse, a field of the wrong type. The guard declines only what it
/// can point at.
#[must_use]
pub fn names_live_job(claude_root: &Path, session_id: &str) -> bool {
    let Ok(entries) = fs::read_dir(paths::claude_jobs_dir(claude_root)) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let job = entry.path();
        job.is_dir() && job_names(&job.join("state.json"), session_id)
    })
}

/// Whether one job's state names `session_id` — as the session it ran, or as
/// the one the next turn would resume. They are usually the same id.
fn job_names(state: &Path, session_id: &str) -> bool {
    let Ok(text) = fs::read_to_string(state) else {
        return false;
    };
    let Ok(value) = json::parse(&text) else {
        return false;
    };
    ["resumeSessionId", "sessionId"]
        .into_iter()
        .any(|key| value.get(key).and_then(Value::as_str) == Some(session_id))
}

/// `<yyyy>-<mm>-<dd>-<HHMMSS>-<path under ~/.claude, `/` → `-`>`.
#[must_use]
pub fn archive_name(claude_root: &Path, source: &Path, now: Timestamp) -> String {
    let relative = source.strip_prefix(claude_root).unwrap_or(source);
    let flattened = relative
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "-");
    let (year, month, day, hour, minute, second) = now.parts();
    format!("{year:04}-{month:02}-{day:02}-{hour:02}{minute:02}{second:02}-{flattened}")
}

/// Move, never copy. A cross-device destination is the one io error with its
/// own variant, because the answer to it is a different data root — not a
/// retry, and never a copy.
fn rename(from: &Path, to: &Path) -> Result<()> {
    fs::rename(from, to).map_err(|error| classify_rename(from, to, &error))
}

/// The typed error for a failed rename.
fn classify_rename(from: &Path, to: &Path, error: &std::io::Error) -> Error {
    if error.kind() == ErrorKind::CrossesDevices {
        return Error::CrossDevice {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
        };
    }
    Error::io(
        from,
        std::io::Error::new(error.kind(), format!("rename to {}: {error}", to.display())),
    )
}

/// A file's mtime as a timestamp.
fn modified_at(path: &Path) -> Result<Timestamp> {
    let modified = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map_err(|error| Error::io(path, error))?;
    let elapsed = modified
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Clock)?;
    let seconds = i64::try_from(elapsed.as_secs()).map_err(|_| Error::Clock)?;
    Ok(Timestamp::from_unix_seconds(seconds))
}

/// Write `<root>/memories/.recent/<sid>.json`.
fn write_pointer(
    data_root: &Path,
    session_id: &str,
    archived: &Path,
    ended: Timestamp,
    digest: &Digest,
) -> Result<PathBuf> {
    let recent = paths::recent_dir(data_root);
    fs::create_dir_all(&recent).map_err(|error| Error::io(&recent, error))?;
    let path = recent.join(format!("{session_id}.json"));
    atomic::write(
        &path,
        &format!("{}\n", pointer(archived, ended, digest, session_id)),
    )?;
    Ok(path)
}

/// The pointer document — enough to recall by without opening the transcript.
#[must_use]
pub fn pointer(archived: &Path, ended: Timestamp, digest: &Digest, session_id: &str) -> String {
    let title = digest
        .title
        .clone()
        .unwrap_or_else(|| session_id.to_owned());
    Value::Object(vec![
        (
            "archived".to_owned(),
            Value::string(archived.display().to_string()),
        ),
        (
            "cwd".to_owned(),
            digest.cwd.clone().map_or(Value::Null, Value::String),
        ),
        ("ended".to_owned(), Value::string(ended.iso8601())),
        ("title".to_owned(), Value::string(title)),
        (
            "highlights".to_owned(),
            Value::Array(
                digest
                    .highlights
                    .iter()
                    .map(|body| Value::string(body.clone()))
                    .collect(),
            ),
        ),
    ])
    .render()
}

/// How many pointers are waiting to be dreamt.
///
/// Dream owns what "waiting" means — a pointer it has already stamped is
/// spent, not queued — and take asks rather than counting files, because the
/// two answers drifting apart is the whole bug: counting spent pointers too
/// made every ending past the tenth read as a full queue and spawn a dream
/// with nothing to route (2026-08-25).
pub fn queue_depth(data_root: &Path) -> Result<usize> {
    crate::verbs::dream::depth(data_root)
}

#[cfg(test)]
mod tests {
    use super::{archive_name, classify_rename, names_live_job, queue_depth, take};
    use crate::error::Error;
    use crate::testutil::TempDir;
    use crate::time::Timestamp;
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    const SID: &str = "aaaabbbb-cccc-dddd-eeee-ffff00001111";
    const PROJECT: &str = "-Users-you--code-project";

    struct Fixture {
        _temp: TempDir,
        claude: PathBuf,
        root: PathBuf,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let temp = TempDir::new(label);
            let claude = temp.path().join(".claude");
            let root = temp.path().join(".sandman");
            fs::create_dir_all(claude.join("projects").join(PROJECT)).expect("project dir");
            Self {
                claude,
                root,
                _temp: temp,
            }
        }

        fn project(&self) -> PathBuf {
            self.claude.join("projects").join(PROJECT)
        }

        /// Seed a transcript and back-date it out of the live window.
        fn seed(&self, lines: &[&str]) -> PathBuf {
            let path = self.project().join(format!("{SID}.jsonl"));
            fs::write(&path, format!("{}\n", lines.join("\n"))).expect("write transcript");
            Self::age(&path, 3600);
            path
        }

        /// Seed a background job directory, carrying `state` when it has one.
        fn job(&self, short: &str, state: Option<&str>) {
            let dir = self.claude.join("jobs").join(short);
            fs::create_dir_all(&dir).expect("job dir");
            if let Some(state) = state {
                fs::write(dir.join("state.json"), state).expect("write job state");
            }
        }

        fn age(path: &Path, seconds: u64) {
            let when = SystemTime::now() - Duration::from_secs(seconds);
            fs::File::open(path)
                .expect("open")
                .set_modified(when)
                .expect("set mtime");
        }
    }

    fn transcript_lines() -> Vec<&'static str> {
        vec![
            r#"{"type":"attachment","cwd":"/Users/you/.code/project"}"#,
            r#"{"type":"user","message":{"content":"port the memory verbs"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","input":{"command":"sandman remember \"the queue is the surface\""}}]}}"#,
        ]
    }

    #[test]
    fn take_moves_the_transcript_and_writes_the_pointer() {
        let fixture = Fixture::new("take-end-to-end");
        let source = fixture.seed(&transcript_lines());
        fs::create_dir_all(fixture.project().join(SID)).expect("sidecar dir");
        fs::write(
            fixture.project().join(SID).join("agent.jsonl"),
            "{\"type\":\"user\"}\n",
        )
        .expect("sidecar transcript");

        let outcome = take(&fixture.root, &fixture.claude, SID, false).expect("take");

        // The move happened: nothing is left behind in the live set.
        assert!(!source.exists());
        assert!(!fixture.project().join(SID).exists());
        assert!(outcome.archived.is_file());
        let name = outcome
            .archived
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .expect("archive name");
        assert!(
            name.ends_with(&format!("-projects-{PROJECT}-{SID}.jsonl")),
            "{name}"
        );
        // <yyyy>-<mm>-<dd>-<HHMMSS>- prefix.
        let stamp: Vec<&str> = name.splitn(5, '-').collect();
        assert_eq!(stamp[0].len(), 4);
        assert_eq!(stamp[1].len(), 2);
        assert_eq!(stamp[2].len(), 2);
        assert_eq!(stamp[3].len(), 6);
        assert!(stamp[3].bytes().all(|byte| byte.is_ascii_digit()));

        let directory = outcome
            .archived_directory
            .clone()
            .expect("archived directory");
        assert!(directory.is_dir());
        assert!(directory.join("agent.jsonl").is_file());
        assert_eq!(
            directory.file_name().and_then(std::ffi::OsStr::to_str),
            Some(format!("{}-dir", name.trim_end_matches(".jsonl")).as_str())
        );

        // The pointer carries the whole short-term surface.
        let pointer = fs::read_to_string(&outcome.pointer).expect("read pointer");
        assert_eq!(
            outcome.pointer,
            fixture
                .root
                .join("memories")
                .join(".recent")
                .join(format!("{SID}.json"))
        );
        assert!(pointer.ends_with('\n'));
        assert!(pointer.contains(&format!(r#""archived":"{}""#, outcome.archived.display())));
        assert!(pointer.contains(r#""cwd":"/Users/you/.code/project""#));
        assert!(pointer.contains(r#""title":"port the memory verbs""#));
        assert!(pointer.contains(r#""highlights":["the queue is the surface"]"#));
        let ended = pointer
            .split(r#""ended":""#)
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("ended");
        assert_eq!(
            Timestamp::parse_iso8601(ended).map(Timestamp::iso8601),
            Some(ended.to_owned())
        );
        assert_eq!(outcome.queue_depth, 1);
        assert!(!outcome.dream_due());
    }

    #[test]
    fn a_transcript_without_a_title_falls_back_to_the_session_id() {
        let fixture = Fixture::new("take-untitled");
        fixture.seed(&["{\"type\":\"assistant\"}", "not json"]);
        let outcome = take(&fixture.root, &fixture.claude, SID, false).expect("take");
        let pointer = fs::read_to_string(&outcome.pointer).expect("read pointer");
        assert!(pointer.contains(&format!(r#""title":"{SID}""#)));
        assert!(pointer.contains(r#""cwd":null"#));
        assert!(pointer.contains(r#""highlights":[]"#));
        assert!(outcome.archived_directory.is_none());
    }

    #[test]
    fn a_live_transcript_is_refused_until_forced() {
        let fixture = Fixture::new("take-live");
        let source = fixture.seed(&transcript_lines());
        Fixture::age(&source, 5);

        match take(&fixture.root, &fixture.claude, SID, false) {
            Err(Error::Refused { message }) => {
                assert!(message.contains("looks live"), "{message}");
                assert!(message.contains("--force"), "{message}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        // Refused means untouched.
        assert!(source.is_file());
        assert!(!fixture.root.join("archive").exists());

        let outcome = take(&fixture.root, &fixture.claude, SID, true).expect("forced take");
        assert!(!source.exists());
        assert!(outcome.archived.is_file());
    }

    #[test]
    fn an_unknown_session_is_a_typed_error() {
        let fixture = Fixture::new("take-missing");
        assert!(matches!(
            take(&fixture.root, &fixture.claude, "no-such-session", false),
            Err(Error::NotFound {
                what: "transcript",
                ..
            })
        ));
        assert!(matches!(
            take(&fixture.root, &fixture.claude, "../escape", false),
            Err(Error::InvalidInput { .. })
        ));
    }

    #[test]
    fn the_queue_depth_counts_only_undreamed_pointers() {
        let fixture = Fixture::new("take-queue");
        assert_eq!(queue_depth(&fixture.root).expect("empty root"), 0);
        let recent = fixture.root.join("memories").join(".recent");
        fs::create_dir_all(&recent).expect("recent dir");
        for index in 0..12 {
            fs::write(recent.join(format!("sid-{index}.json")), "{}\n").expect("pointer");
        }
        fs::write(recent.join("notes.txt"), "not a pointer").expect("stray");
        // A pointer dream has already routed is spent, not queued: counting
        // it would make the next ending spawn a dream over nothing.
        for index in 0..30 {
            fs::write(
                recent.join(format!("spent-{index}.json")),
                "{\"dreamed\":\"2026-08-11T12:00:00Z\"}\n",
            )
            .expect("dreamed pointer");
        }
        assert_eq!(queue_depth(&fixture.root).expect("count"), 12);

        fixture.seed(&transcript_lines());
        let outcome = take(&fixture.root, &fixture.claude, SID, false).expect("take");
        assert_eq!(outcome.queue_depth, 13);
        assert!(outcome.dream_due());
    }

    #[test]
    fn a_job_directory_still_naming_the_session_reports_it_live() {
        let fixture = Fixture::new("take-jobs");
        // No jobs directory at all: nothing to be named by.
        assert!(!names_live_job(&fixture.claude, SID));

        fixture.job(
            "11112222",
            Some(&format!(r#"{{"sessionId":"{SID}","state":"done"}}"#)),
        );
        assert!(names_live_job(&fixture.claude, SID));
        // A jobs directory full of other people's sessions is not a match.
        assert!(!names_live_job(&fixture.claude, "no-such-session"));
    }

    #[test]
    fn an_unreadable_job_is_skipped_rather_than_believed() {
        let fixture = Fixture::new("take-jobs-partial");
        fixture.job("00000000", Some(r#"{"sessionId":"#)); // malformed
        fixture.job("33334444", None); // no state at all
        fixture.job("55556666", Some(r#"{"sessionId":42}"#)); // wrong type
        fixture.job("77778888", Some(r#"{"cwd":"/tmp"}"#)); // neither field
        // The jobs directory carries files as well as jobs.
        fs::write(fixture.claude.join("jobs").join("pins.json"), "[]").expect("pins");
        assert!(!names_live_job(&fixture.claude, SID));

        // …and the one job that does name it still answers, past all of them —
        // here by the id the next turn would resume, not the one it ran under.
        fixture.job(
            "99990000",
            Some(&format!(
                r#"{{"resumeSessionId":"{SID}","sessionId":"99990000-dead-beef-0000-000000000000"}}"#
            )),
        );
        assert!(names_live_job(&fixture.claude, SID));
    }

    #[test]
    fn a_cross_device_rename_is_its_own_error_and_never_a_copy() {
        let error = classify_rename(
            Path::new("/a/live.jsonl"),
            Path::new("/b/archived.jsonl"),
            &io::Error::from(io::ErrorKind::CrossesDevices),
        );
        match error {
            Error::CrossDevice { from, to } => {
                assert_eq!(from, Path::new("/a/live.jsonl"));
                assert_eq!(to, Path::new("/b/archived.jsonl"));
            }
            other => panic!("expected CrossDevice, got {other:?}"),
        }
        assert!(matches!(
            classify_rename(
                Path::new("/a"),
                Path::new("/b"),
                &io::Error::from(io::ErrorKind::PermissionDenied)
            ),
            Error::Io { .. }
        ));
    }

    #[test]
    fn the_archive_name_flattens_the_path_under_claude_root() {
        let now = Timestamp::from_unix_seconds(1_786_018_297);
        assert_eq!(
            archive_name(
                Path::new("/Users/you/.claude"),
                Path::new("/Users/you/.claude/projects/-Users-you/sid.jsonl"),
                now
            ),
            "2026-08-06-121137-projects--Users-you-sid.jsonl"
        );
        // A path outside the root keeps its own components rather than escaping.
        assert_eq!(
            archive_name(
                Path::new("/Users/you/.claude"),
                Path::new("/elsewhere/sid.jsonl"),
                now
            ),
            "2026-08-06-121137--elsewhere-sid.jsonl"
        );
    }
}
