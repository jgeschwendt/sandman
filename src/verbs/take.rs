//! `take` — archive a session by move, then drop its pointer.
//!
//! The move is the take: after it the conversation has left Claude Code's
//! live/resumable set, which is the feature rather than a side effect. Only a
//! rename is ever used — a copy would leave the session both taken and live,
//! so a cross-device destination is an error, not a fallback.

use std::ffi::OsStr;
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
    /// How big it was, in bytes, as it went into the archive. A conversation's
    /// size is the one number that says at a glance whether a take caught the
    /// whole thing or a recreated stub.
    pub archived_bytes: usize,
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
            note(
                data_root,
                &format!("refused live session={session_id} idle={idle}s"),
            );
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
        archived_bytes: text.len(),
        archived_directory,
        queue_depth,
        pointer,
    })
}

/// The background job that still names `session_id`, by its short id.
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
/// The proof holds only for the session the job would resume next. A `/clear`
/// inside a backgrounded conversation ends that session and starts a fresh
/// one: the job carries on under the new id, and its `state.json` is left
/// naming the cleared session in `sessionId` while `resumeSessionId` has
/// already moved to the successor. That stale name protected a conversation
/// the job will never touch again — a 1.8 MB transcript sat untaken behind it
/// (2026-08-28) — so a job speaks for the id it would resume, and for the one
/// it ran only when it names no other.
///
/// Every uncertainty reads as "no job": an absent or unreadable jobs
/// directory, an entry that is not a job, a `state.json` that is missing or
/// will not parse, a field of the wrong type. The guard declines only what it
/// can point at — and it hands back which job it was pointing at, because a
/// decline nobody can trace to a job is the invisibility that made the
/// original incident a forensics exercise.
#[must_use]
pub fn live_job(claude_root: &Path, session_id: &str) -> Option<String> {
    let entries = fs::read_dir(paths::claude_jobs_dir(claude_root)).ok()?;
    entries.flatten().find_map(|entry| {
        let job = entry.path();
        if job.is_dir() && job_names(&job.join("state.json"), session_id) {
            entry.file_name().to_str().map(ToOwned::to_owned)
        } else {
            None
        }
    })
}

/// Whether one job's state names `session_id` as the session the next turn
/// would resume — `resumeSessionId` when the state carries one, and otherwise
/// the `sessionId` it ran under. They are usually the same id.
///
/// When they differ, only `resumeSessionId` counts: the job has moved on, and
/// the id it left behind names a conversation that is closed forever.
fn job_names(state: &Path, session_id: &str) -> bool {
    let Ok(text) = fs::read_to_string(state) else {
        return false;
    };
    let Ok(value) = json::parse(&text) else {
        return false;
    };
    let named = |key| value.get(key).and_then(Value::as_str);
    named("resumeSessionId").map_or_else(
        || named("sessionId") == Some(session_id),
        |resume| resume == session_id,
    )
}

/// What one drained ledger entry came to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Drained {
    /// Nothing left to take — the session was archived or forgotten by other
    /// means since the decline — so the entry went with it.
    Dropped {
        /// The session the entry named.
        session: String,
    },
    /// The take did not go through and the entry stays for a later drain.
    Kept {
        /// The session the entry named.
        session: String,
        /// What the take said, for the journal.
        why: String,
    },
    /// The job is gone and the owed take happened.
    Reclaimed {
        /// What the take did.
        outcome: TakeOutcome,
        /// The session that was reclaimed.
        session: String,
    },
    /// The ledger file itself would not read; it stays in place as evidence.
    Unreadable {
        /// The entry that could not be read.
        entry: PathBuf,
        /// What reading it said.
        why: String,
    },
}

/// Record that `session_id`'s take is owed, because `job` would resume it.
///
/// The job guard is right to decline, and until this ledger the decline was
/// also the end of the matter: nothing ever went back for the transcript once
/// the operator deleted the job, so session 09901667 sat in Claude Code's live
/// set for 22 h behind a job that no longer existed and was taken by hand
/// (2026-08-30). A decline is a debt now — one file per session, settled by
/// whichever take or reflect next finds the job gone.
pub fn remember_pending(
    data_root: &Path,
    session_id: &str,
    job: &str,
    now: Timestamp,
) -> Result<PathBuf> {
    transcript::check_session_id(session_id)?;
    let dir = paths::pending_takes_dir(data_root);
    fs::create_dir_all(&dir).map_err(|error| Error::io(&dir, error))?;
    let path = paths::pending_take(data_root, session_id);
    let entry = Value::Object(vec![
        ("declined".to_owned(), Value::string(now.iso8601())),
        ("job".to_owned(), Value::string(job)),
        ("session".to_owned(), Value::string(session_id)),
    ]);
    atomic::write(&path, &format!("{}\n", entry.render()))?;
    Ok(path)
}

/// Retry every owed take whose job has since been retired.
///
/// Nothing here can fail the caller: a drain runs alongside a named take and
/// alongside reflect's pass, and neither has any business failing because a
/// session nobody asked about would not move. One bad entry is journalled and
/// stepped over, and the rest of the ledger is still drained.
///
/// `force` is never passed. The live window is exactly the guard wanted here:
/// a session whose job was deleted may well have been resumed by hand and be
/// mid-conversation, and that conversation's own `SessionEnd` will take it —
/// at which point a later drain finds nothing and drops the stale entry.
#[allow(
    clippy::must_use_candidate,
    reason = "the journal is the record of a drain; the report is for callers that want one"
)]
pub fn drain_pending(data_root: &Path, claude_root: &Path) -> Vec<Drained> {
    let mut drained = Vec::new();
    // No ledger, or one that will not list, is nothing owed: the drain is a
    // backstop, and a backstop that reports its own absence is noise.
    let Ok(listing) = fs::read_dir(paths::pending_takes_dir(data_root)) else {
        return drained;
    };
    let mut entries: Vec<PathBuf> = listing
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension() == Some(OsStr::new("json")) && path.is_file())
        .collect();
    // By session id: an arbitrary order, but a stable one — a drain that goes
    // wrong halfway through goes wrong the same way twice.
    entries.sort();

    for entry in entries {
        let session = match pending_session(&entry) {
            Ok(session) => session,
            Err(error) => {
                let why = error.to_string();
                note(data_root, &format!("pending unreadable {why}"));
                drained.push(Drained::Unreadable { entry, why });
                continue;
            }
        };
        // Still the job's to resume. Nothing is owed yet and nothing is
        // journalled: this is the common case, once per take per pending
        // session, and a line for it would bury the ones that matter.
        if live_job(claude_root, &session).is_some() {
            continue;
        }
        match take(data_root, claude_root, &session, false) {
            Ok(outcome) => {
                clear_pending(data_root, &entry);
                note(
                    data_root,
                    &format!(
                        "reclaimed session={session} archived={} bytes={} queue={}",
                        outcome.archived.display(),
                        outcome.archived_bytes,
                        outcome.queue_depth
                    ),
                );
                drained.push(Drained::Reclaimed { outcome, session });
            }
            // Taken or forgotten by other means since the decline — the debt
            // is settled, whoever settled it.
            Err(Error::NotFound { .. }) => {
                clear_pending(data_root, &entry);
                note(
                    data_root,
                    &format!("pending dropped session={session} gone"),
                );
                drained.push(Drained::Dropped { session });
            }
            Err(error) => {
                let why = error.to_string();
                note(data_root, &format!("pending kept session={session}: {why}"));
                drained.push(Drained::Kept { session, why });
            }
        }
    }
    drained
}

/// The session one ledger entry names.
///
/// Every uncertainty reads as "cannot say", and a "cannot say" is left on
/// disk: the entry is the only record that a take is owed, so a file that
/// will not read, will not parse, or names nothing usable is evidence to keep
/// rather than a debt to guess at or delete.
fn pending_session(entry: &Path) -> Result<String> {
    let text = fs::read_to_string(entry).map_err(|error| Error::io(entry, error))?;
    let value = json::parse(&text).map_err(|error| Error::Json {
        path: Some(entry.to_path_buf()),
        message: error.to_string(),
    })?;
    let session = value
        .get("session")
        .and_then(Value::as_str)
        .filter(|session| transcript::check_session_id(session).is_ok())
        .ok_or_else(|| Error::Json {
            path: Some(entry.to_path_buf()),
            message: "names no usable session".to_owned(),
        })?;
    Ok(session.to_owned())
}

/// Clear one settled ledger entry.
///
/// A removal that will not happen is journalled and otherwise tolerated: the
/// take it recorded has already happened, and the next drain finds the
/// transcript gone and drops the entry then.
fn clear_pending(data_root: &Path, entry: &Path) {
    if let Err(error) = fs::remove_file(entry) {
        note(
            data_root,
            &format!("pending stuck entry={}: {error}", entry.display()),
        );
    }
}

/// Journal one line under `take`.
fn note(data_root: &Path, line: &str) {
    crate::journal::note(data_root, "take", line);
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
    use super::{
        Drained, archive_name, classify_rename, drain_pending, live_job, queue_depth,
        remember_pending, take,
    };
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
            self.seed_session(SID, lines)
        }

        /// The same, for a session other than the fixture's default.
        fn seed_session(&self, session: &str, lines: &[&str]) -> PathBuf {
            let path = self.project().join(format!("{session}.jsonl"));
            fs::write(&path, format!("{}\n", lines.join("\n"))).expect("write transcript");
            Self::age(&path, 3600);
            path
        }

        /// Delete a background job, the way the operator does.
        fn retire(&self, short: &str) {
            fs::remove_dir_all(self.claude.join("jobs").join(short)).expect("retire the job");
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

        // The recorded size is the archived file's own.
        assert_eq!(
            u64::try_from(outcome.archived_bytes).expect("a size"),
            fs::metadata(&outcome.archived).expect("stat").len()
        );

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
        // Refused means untouched — but the refusal is on the record, so a
        // take that never happened can still be explained afterwards.
        assert!(source.is_file());
        assert!(!fixture.root.join("archive").exists());
        let journal = fs::read_to_string(crate::paths::run_log(
            &fixture.root,
            "take",
            crate::time::Timestamp::now().expect("clock"),
        ))
        .expect("the take journal");
        assert!(
            journal.contains(&format!("refused live session={SID} idle=")),
            "{journal}"
        );

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
        assert_eq!(live_job(&fixture.claude, SID), None);

        fixture.job(
            "11112222",
            Some(&format!(r#"{{"sessionId":"{SID}","state":"done"}}"#)),
        );
        // The job is named back, so the decline can say which one it was.
        assert_eq!(live_job(&fixture.claude, SID).as_deref(), Some("11112222"));
        // A jobs directory full of other people's sessions is not a match.
        assert_eq!(live_job(&fixture.claude, "no-such-session"), None);
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
        assert_eq!(live_job(&fixture.claude, SID), None);

        // …and the one job that does name it still answers, past all of them —
        // here by the id the next turn would resume, not the one it ran under.
        fixture.job(
            "99990000",
            Some(&format!(
                r#"{{"resumeSessionId":"{SID}","sessionId":"99990000-dead-beef-0000-000000000000"}}"#
            )),
        );
        assert_eq!(live_job(&fixture.claude, SID).as_deref(), Some("99990000"));
    }

    #[test]
    fn a_job_protects_only_the_session_it_would_still_resume() {
        const SUCCESSOR: &str = "0ddf70a8-1111-2222-3333-444455556666";

        // A `/clear` ended the session the job ran under and the job carried on
        // under the successor. The stale `sessionId` is a name, not a claim:
        // nothing will ever append to that transcript again.
        let moved = Fixture::new("take-jobs-moved-on");
        moved.job(
            "11112222",
            Some(&format!(
                r#"{{"sessionId":"{SID}","resumeSessionId":"{SUCCESSOR}","state":"running"}}"#
            )),
        );
        assert_eq!(live_job(&moved.claude, SID), None);
        // …and the successor is the one the job speaks for.
        assert_eq!(
            live_job(&moved.claude, SUCCESSOR).as_deref(),
            Some("11112222")
        );

        // With no `resumeSessionId` at all, the session it ran is the session
        // it would resume.
        let ran = Fixture::new("take-jobs-ran");
        ran.job(
            "22223333",
            Some(&format!(r#"{{"sessionId":"{SID}","state":"done"}}"#)),
        );
        assert_eq!(live_job(&ran.claude, SID).as_deref(), Some("22223333"));

        // The ordinary case: both fields are the one id, and it is protected.
        let same = Fixture::new("take-jobs-same");
        same.job(
            "33334444",
            Some(&format!(
                r#"{{"sessionId":"{SID}","resumeSessionId":"{SID}"}}"#
            )),
        );
        assert_eq!(live_job(&same.claude, SID).as_deref(), Some("33334444"));
    }

    /// The take journal, or empty when nothing wrote one.
    fn journal(root: &Path) -> String {
        fs::read_to_string(crate::paths::run_log(
            root,
            "take",
            Timestamp::now().expect("clock"),
        ))
        .unwrap_or_default()
    }

    /// Write a ledger entry the way a decline does.
    fn pend(root: &Path, session: &str, job: &str) -> PathBuf {
        remember_pending(root, session, job, Timestamp::now().expect("clock")).expect("pend")
    }

    #[test]
    fn a_deferred_take_is_written_down_and_settled_once_the_job_is_gone() {
        let fixture = Fixture::new("take-pending-reclaim");
        let source = fixture.seed(&transcript_lines());
        fixture.job(
            "11112222",
            Some(&format!(r#"{{"sessionId":"{SID}","state":"done"}}"#)),
        );

        let entry = pend(&fixture.root, SID, "11112222");
        assert_eq!(entry, crate::paths::pending_take(&fixture.root, SID));
        let written = fs::read_to_string(&entry).expect("read the entry");
        assert!(written.ends_with('\n'), "{written}");
        assert!(written.contains(r#""job":"11112222""#), "{written}");
        assert!(
            written.contains(&format!(r#""session":"{SID}""#)),
            "{written}"
        );
        let declined = written
            .split(r#""declined":""#)
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("declined");
        assert_eq!(
            Timestamp::parse_iso8601(declined).map(Timestamp::iso8601),
            Some(declined.to_owned())
        );

        // While the job lives there is nothing owed yet, and nothing said.
        assert_eq!(drain_pending(&fixture.root, &fixture.claude), Vec::new());
        assert!(entry.is_file());
        assert!(source.is_file(), "the transcript stays in the live set");
        assert!(journal(&fixture.root).is_empty());

        // The operator deletes the job — the moment nothing else notices.
        fixture.retire("11112222");
        let drained = drain_pending(&fixture.root, &fixture.claude);
        let [Drained::Reclaimed { outcome, session }] = drained.as_slice() else {
            panic!("expected one reclaim, got {drained:?}");
        };
        assert_eq!(session, SID);
        assert!(!source.exists(), "the take is the move");
        assert!(outcome.archived.is_file());
        assert!(outcome.pointer.is_file());
        assert!(!entry.exists(), "a settled debt leaves the ledger");
        let journal = journal(&fixture.root);
        assert!(
            journal.contains(&format!(
                "reclaimed session={SID} archived={} bytes={} queue={}",
                outcome.archived.display(),
                outcome.archived_bytes,
                outcome.queue_depth
            )),
            "{journal}"
        );
    }

    #[test]
    fn a_pending_session_with_nothing_left_to_take_is_dropped() {
        let fixture = Fixture::new("take-pending-gone");
        // No transcript at all: taken or forgotten by other means since the
        // decline. The debt is settled, whoever settled it.
        let entry = pend(&fixture.root, SID, "11112222");
        let drained = drain_pending(&fixture.root, &fixture.claude);
        assert_eq!(
            drained,
            vec![Drained::Dropped {
                session: SID.to_owned()
            }]
        );
        assert!(!entry.exists());
        let journal = journal(&fixture.root);
        assert!(
            journal.contains(&format!("pending dropped session={SID} gone")),
            "{journal}"
        );
    }

    #[test]
    fn a_pending_session_touched_inside_the_live_window_is_kept_never_forced() {
        let fixture = Fixture::new("take-pending-live");
        // The job was deleted and the operator resumed the conversation by
        // hand: it is mid-turn, and its own SessionEnd is what should take it.
        let source = fixture.seed(&transcript_lines());
        Fixture::age(&source, 5);
        let entry = pend(&fixture.root, SID, "11112222");

        let drained = drain_pending(&fixture.root, &fixture.claude);
        let [Drained::Kept { session, why }] = drained.as_slice() else {
            panic!("expected one kept entry, got {drained:?}");
        };
        assert_eq!(session, SID);
        assert!(why.contains("looks live"), "{why}");
        assert!(source.is_file(), "the conversation is left alone");
        assert!(entry.is_file(), "and the debt is still owed");
        let journal = journal(&fixture.root);
        assert!(
            journal.contains(&format!("pending kept session={SID}:")),
            "{journal}"
        );

        // Aged out, the same drain settles it — and the stale entry a
        // hand-resumed session's own ending would leave is dropped next time.
        Fixture::age(&source, 3600);
        assert!(matches!(
            drain_pending(&fixture.root, &fixture.claude).as_slice(),
            [Drained::Reclaimed { .. }]
        ));
        assert!(!entry.exists());
    }

    #[test]
    fn a_ledger_entry_that_will_not_read_is_evidence_and_stays() {
        let fixture = Fixture::new("take-pending-unreadable");
        let dir = crate::paths::pending_takes_dir(&fixture.root);
        fs::create_dir_all(&dir).expect("ledger dir");
        let malformed = dir.join("aaaa-1111.json");
        fs::write(&malformed, r#"{"session":"#).expect("malformed entry");
        let nameless = dir.join("bbbb-2222.json");
        fs::write(&nameless, r#"{"job":"11112222"}"#).expect("nameless entry");
        let hostile = dir.join("cccc-3333.json");
        fs::write(&hostile, r#"{"session":"../escape"}"#).expect("hostile entry");
        // The ledger directory carries files that are not entries.
        fs::write(dir.join("notes.txt"), "not an entry").expect("stray");

        let drained = drain_pending(&fixture.root, &fixture.claude);
        assert_eq!(drained.len(), 3, "{drained:?}");
        assert!(
            drained
                .iter()
                .all(|outcome| matches!(outcome, Drained::Unreadable { .. })),
            "{drained:?}"
        );
        for entry in [&hostile, &malformed, &nameless] {
            assert!(entry.is_file(), "{}", entry.display());
        }
        let journal = journal(&fixture.root);
        assert_eq!(
            journal
                .lines()
                .filter(|line| line.contains("pending unreadable "))
                .count(),
            3,
            "{journal}"
        );
        assert!(
            journal.contains(&format!(
                "pending unreadable {}: names no usable session",
                hostile.display()
            )),
            "{journal}"
        );
    }

    #[test]
    fn a_hostile_session_id_never_reaches_the_ledger() {
        let fixture = Fixture::new("take-pending-hostile");
        assert!(matches!(
            remember_pending(
                &fixture.root,
                "../escape",
                "11112222",
                Timestamp::now().expect("clock")
            ),
            Err(Error::InvalidInput { .. })
        ));
        assert!(!crate::paths::pending_takes_dir(&fixture.root).exists());
    }

    #[test]
    fn a_drain_settles_what_it_can_and_steps_over_the_rest() {
        const OTHER: &str = "bbbbcccc-dddd-eeee-ffff-000011112222";
        const WORKING: &str = "ccccdddd-eeee-ffff-0000-111122223333";

        let fixture = Fixture::new("take-pending-mixed");
        // One whose job still lives, one still live in itself, one ready.
        fixture.job(
            "11112222",
            Some(&format!(r#"{{"sessionId":"{OTHER}","state":"running"}}"#)),
        );
        fixture.seed_session(OTHER, &transcript_lines());
        let working = fixture.seed_session(WORKING, &transcript_lines());
        Fixture::age(&working, 5);
        let ready = fixture.seed(&transcript_lines());
        for (session, job) in [
            (OTHER, "11112222"),
            (SID, "33334444"),
            (WORKING, "55556666"),
        ] {
            pend(&fixture.root, session, job);
        }

        let drained = drain_pending(&fixture.root, &fixture.claude);
        // The entry whose job still lives is not in the report at all — it is
        // the common case, and a line for it every take would bury the rest.
        assert_eq!(drained.len(), 2, "{drained:?}");
        assert!(
            matches!(
                drained.as_slice(),
                [Drained::Reclaimed { .. }, Drained::Kept { .. }]
            ),
            "{drained:?}"
        );
        assert!(!ready.exists(), "the one that could be taken was");
        assert!(working.is_file(), "the live one was not");
        assert!(
            crate::paths::pending_take(&fixture.root, OTHER).is_file(),
            "the job's own session is still owed"
        );
        assert!(!crate::paths::pending_take(&fixture.root, SID).exists());
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
