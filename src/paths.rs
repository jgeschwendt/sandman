//! Where sandman reads and writes — the single config point.
//!
//! The data root is `~/.sandman`, overridable with `$SANDMAN_ROOT`. Nothing
//! else in the crate hardcodes it: every verb takes its roots as arguments and
//! only `main` reaches for these defaults, which is what keeps the tests off
//! the operator's real trees.
//!
//! Two tiers under the root, and the leading dot is the boundary. The hidden
//! directories — `.archive/`, `.dream/`, `.trace/` — hold raw bytes the
//! operator's `~/.sandman` repo gitignores; the visible ones — `log/`,
//! `memories/` — hold the content it versions. The tiers were mixed until
//! 2026-09-05, when 60 run journals went in with a commit because they shared
//! `log/` with reflect's day pages; one gitignorable prefix is what stops the
//! next one, and it only works if nothing raw is ever filed outside it.

use std::env;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// The environment variable that moves the data root.
pub const ROOT_ENV: &str = "SANDMAN_ROOT";
/// The data root's name under `$HOME`.
pub const ROOT_DIR_NAME: &str = ".sandman";
/// Claude Code's state directory, under `$HOME`.
pub const CLAUDE_DIR_NAME: &str = ".claude";
/// Claude Code's background-job records, one directory per job.
pub const JOBS_DIR_NAME: &str = "jobs";
/// Where Claude Code keeps one directory of transcripts per project.
pub const PROJECTS_DIR_NAME: &str = "projects";
/// Session pointers — the short-term recall surface.
pub const RECENT_DIR_NAME: &str = ".recent";
/// Archived transcripts live under `<root>/.archive/claude/`.
pub const ARCHIVE_DIR_NAME: &str = ".archive";
/// The archive's Claude Code lane.
pub const ARCHIVE_CLAUDE_DIR_NAME: &str = "claude";
/// Reflect's day pages and their index.
pub const LOG_DIR_NAME: &str = "log";
/// The run journals — one file per verb per day.
pub const TRACE_DIR_NAME: &str = ".trace";
/// Kept dream mind transcripts live under `<root>/.dream/`.
pub const DREAM_DIR_NAME: &str = ".dream";
/// Takes a live background job deferred — one file per session, cleared when
/// the take finally happens.
pub const PENDING_TAKES_DIR_NAME: &str = "pending-takes";

/// `$HOME`, or a typed error.
pub fn home() -> Result<PathBuf> {
    env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .ok_or(Error::MissingEnv { name: "HOME" })
}

/// The data root: `$SANDMAN_ROOT`, else `$HOME/.sandman`.
pub fn data_root() -> Result<PathBuf> {
    if let Some(root) = env::var_os(ROOT_ENV).filter(|root| !root.is_empty()) {
        return Ok(PathBuf::from(root));
    }
    Ok(home()?.join(ROOT_DIR_NAME))
}

/// Claude Code's state directory — where transcripts are found.
pub fn claude_root() -> Result<PathBuf> {
    Ok(home()?.join(CLAUDE_DIR_NAME))
}

/// `<claude root>/jobs` — one directory per background job, for as long as the
/// job is the operator's to resume.
#[must_use]
pub fn claude_jobs_dir(claude_root: &Path) -> PathBuf {
    claude_root.join(JOBS_DIR_NAME)
}

/// `<claude root>/projects` — one directory of transcripts per project.
#[must_use]
pub fn claude_projects_dir(claude_root: &Path) -> PathBuf {
    claude_root.join(PROJECTS_DIR_NAME)
}

/// `<root>/.dream` — where a dream mind's own transcript is kept, and the
/// directory the minds are run in so Claude Code writes it somewhere known.
#[must_use]
pub fn dream_dir(data_root: &Path) -> PathBuf {
    data_root.join(DREAM_DIR_NAME)
}

/// `<root>/pending-takes` — the ledger of takes a live job deferred.
#[must_use]
pub fn pending_takes_dir(data_root: &Path) -> PathBuf {
    data_root.join(PENDING_TAKES_DIR_NAME)
}

/// `<root>/pending-takes/<sid>.json` — one session's owed take.
///
/// One file per session, named by it: a session declined twice is one debt,
/// not two, and the entry is findable by the id every other log line carries.
#[must_use]
pub fn pending_take(data_root: &Path, session_id: &str) -> PathBuf {
    pending_takes_dir(data_root).join(format!("{session_id}.json"))
}

/// `<root>/memories/.recent` — the pointer queue.
#[must_use]
pub fn recent_dir(data_root: &Path) -> PathBuf {
    data_root
        .join(crate::bank::MEMORIES_DIR_NAME)
        .join(RECENT_DIR_NAME)
}

/// `<root>/log` — reflect's day pages and their `INDEX.md`, and nothing else.
#[must_use]
pub fn log_dir(data_root: &Path) -> PathBuf {
    data_root.join(LOG_DIR_NAME)
}

/// `<root>/.trace` — the run journals, operator forensics rather than content.
#[must_use]
pub fn trace_dir(data_root: &Path) -> PathBuf {
    data_root.join(TRACE_DIR_NAME)
}

/// `<root>/.trace/<verb>-<yyyy>-<mm>-<dd>.log` — one run log per pass per day.
#[must_use]
pub fn run_log(data_root: &Path, verb: &str, at: crate::time::Timestamp) -> PathBuf {
    let (year, month, day, ..) = at.parts();
    trace_dir(data_root).join(format!("{verb}-{year:04}-{month:02}-{day:02}.log"))
}

/// `<root>/.archive/claude` — where taken transcripts land.
#[must_use]
pub fn archive_claude_dir(data_root: &Path) -> PathBuf {
    data_root
        .join(ARCHIVE_DIR_NAME)
        .join(ARCHIVE_CLAUDE_DIR_NAME)
}

/// `<root>/.archive/claude/<yyyy>/<mm>/<dd>` — the day a take landed on.
///
/// The archive is append-only and never swept, so a flat directory only grows;
/// nesting keeps it listable by hand and makes a day's takes a `ls` rather than
/// a prefix match over everything ever taken. A calendar day and not an
/// instant, because that is what both writers already hold: `take` stamps one,
/// and `reflect` renders a page per day.
#[must_use]
pub fn archive_day_dir(data_root: &Path, year: i64, month: i64, day: i64) -> PathBuf {
    archive_claude_dir(data_root)
        .join(format!("{year:04}"))
        .join(format!("{month:02}"))
        .join(format!("{day:02}"))
}

/// A path with `$HOME` folded back to `~`, for prose that a human reads.
#[must_use]
pub fn tildify(path: &Path, home: &Path) -> String {
    path.strip_prefix(home).map_or_else(
        |_| path.display().to_string(),
        |rest| format!("~/{}", rest.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        archive_claude_dir, archive_day_dir, claude_jobs_dir, claude_projects_dir, dream_dir,
        log_dir, pending_take, pending_takes_dir, recent_dir, run_log, tildify, trace_dir,
    };
    use crate::time::Timestamp;
    use std::path::Path;

    #[test]
    fn the_root_layout_matches_the_plan() {
        let root = Path::new("/data");
        assert_eq!(recent_dir(root), Path::new("/data/memories/.recent"));
        assert_eq!(archive_claude_dir(root), Path::new("/data/.archive/claude"));
        assert_eq!(
            archive_day_dir(root, 2026, 8, 14),
            Path::new("/data/.archive/claude/2026/08/14")
        );
        assert_eq!(dream_dir(root), Path::new("/data/.dream"));
        assert_eq!(log_dir(root), Path::new("/data/log"));
        assert_eq!(trace_dir(root), Path::new("/data/.trace"));
        assert_eq!(pending_takes_dir(root), Path::new("/data/pending-takes"));
        assert_eq!(
            pending_take(root, "aaaabbbb-cccc"),
            Path::new("/data/pending-takes/aaaabbbb-cccc.json")
        );
        assert_eq!(
            claude_jobs_dir(Path::new("/Users/you/.claude")),
            Path::new("/Users/you/.claude/jobs")
        );
        assert_eq!(
            claude_projects_dir(Path::new("/Users/you/.claude")),
            Path::new("/Users/you/.claude/projects")
        );
        assert_eq!(
            run_log(root, "dream", Timestamp::from_unix_seconds(1_786_018_297)),
            Path::new("/data/.trace/dream-2026-08-06.log")
        );
    }

    #[test]
    fn tildify_folds_only_the_home_prefix() {
        let home = Path::new("/Users/you");
        assert_eq!(
            tildify(Path::new("/Users/you/.sandman"), home),
            "~/.sandman"
        );
        assert_eq!(tildify(Path::new("/tmp/scratch"), home), "/tmp/scratch");
        assert_eq!(tildify(home, home), "~/");
    }
}
