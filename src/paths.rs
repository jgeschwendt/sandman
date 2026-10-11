//! Where sandman reads and writes — the single config point.
//!
//! The data root is `~/.sandman`, overridable with `$SANDMAN_ROOT`. Nothing
//! else in the crate hardcodes it: every verb takes its roots as arguments and
//! only `main` reaches for these defaults, which is what keeps the tests off
//! the operator's real trees.
//!
//! Two tiers under the root, and the leading dot is the boundary. The hidden
//! directories — `.archive/`, `.dream/`, `.trace/` — hold raw bytes the
//! operator's `~/.sandman` repo gitignores; the visible ones — `breath/`,
//! `log/`, `memories/`, `score/`, `shifts/`, `trail/` — hold the content it
//! versions. The tiers were mixed until
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
/// The environment variable that moves Claude Code's `~/.claude.json`.
pub const CLAUDE_JSON_ENV: &str = "SANDMAN_CLAUDE_JSON";
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
/// The voyage log — one entry per day — and its index.
pub const LOG_DIR_NAME: &str = "log";
/// The run journals — one file per verb per day.
pub const TRACE_DIR_NAME: &str = ".trace";
/// Kept dream mind transcripts live under `<root>/.dream/`.
pub const DREAM_DIR_NAME: &str = ".dream";
/// The trail — one JSONL file of events per UTC day.
pub const TRAIL_DIR_NAME: &str = "trail";
/// The environment variable that moves the grove's code directory.
pub const GROVE_CODE_ENV: &str = "SANDMAN_GROVE_CODE";
/// The environment variable that replaces reconcile's extra repos,
/// colon-separated.
pub const RECONCILE_EXTRA_ENV: &str = "SANDMAN_RECONCILE_EXTRA";
/// The ruach's breaths and the problems file it reads.
pub const BREATH_DIR_NAME: &str = "breath";
/// Where Claude Code's statusline persists the five-hour window; overrides
/// `$XDG_STATE_HOME/claude/window.json`.
pub const CLAUDE_WINDOW_ENV: &str = "CLAUDE_WINDOW_FILE";
/// The environment variable that moves the machine's prompt templates.
pub const PROMPTS_ENV: &str = "SANDMAN_PROMPTS";
/// The environment variable that moves the routines' run state.
pub const ROUTINES_STATE_ENV: &str = "SANDMAN_ROUTINES_STATE";
/// The week's score pages, `<iso-week>.md`, and the Sabbath's
/// `<iso-week>-targets.md`.
pub const SCORE_DIR_NAME: &str = "score";
/// The environment variable that moves `~/.threads`.
pub const THREADS_ENV: &str = "SANDMAN_THREADS";
/// One directory per shift, plus `current.json` while one is open.
pub const SHIFTS_DIR_NAME: &str = "shifts";
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

/// Claude Code's global config, where the trust dialog's answers live:
/// `$SANDMAN_CLAUDE_JSON`, else `~/.claude.json`.
pub fn claude_json() -> Result<PathBuf> {
    if let Some(file) = env::var_os(CLAUDE_JSON_ENV).filter(|file| !file.is_empty()) {
        return Ok(PathBuf::from(file));
    }
    Ok(home()?.join(".claude.json"))
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

/// `<root>/log` — the voyage log entries and their `INDEX.md`, and nothing else.
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

/// `<root>/trail` — the event log, versioned content like `log/`.
#[must_use]
pub fn trail_dir(data_root: &Path) -> PathBuf {
    data_root.join(TRAIL_DIR_NAME)
}

/// `<root>/trail/<yyyy>/<mm>/<dd>.jsonl` — the UTC day `at` falls on.
#[must_use]
pub fn trail_day(data_root: &Path, at: crate::time::Timestamp) -> PathBuf {
    let (year, month, day, ..) = at.parts();
    trail_dir(data_root)
        .join(format!("{year:04}"))
        .join(format!("{month:02}"))
        .join(format!("{day:02}.jsonl"))
}

/// The grove's code directory: `$SANDMAN_GROVE_CODE`, else `~/.grove/code`.
/// Its checkouts sit at `<code>/<owner>/<repo>/<branch>/`.
pub fn grove_code() -> Result<PathBuf> {
    if let Some(code) = env::var_os(GROVE_CODE_ENV).filter(|code| !code.is_empty()) {
        return Ok(PathBuf::from(code));
    }
    Ok(home()?.join(".grove").join("code"))
}

/// The grove manifest, beside the code directory: `<code>/../manifest.toml`.
#[must_use]
pub fn grove_manifest(grove_code: &Path) -> PathBuf {
    grove_code
        .parent()
        .unwrap_or(grove_code)
        .join("manifest.toml")
}

/// The repos reconcile reaches beyond the grove: `$SANDMAN_RECONCILE_EXTRA`
/// (colon-separated; set and empty means none), else `~`, `~/.routines` and
/// the data root.
pub fn reconcile_extras() -> Result<Vec<PathBuf>> {
    if let Some(extra) = env::var_os(RECONCILE_EXTRA_ENV) {
        return Ok(env::split_paths(&extra)
            .filter(|path| !path.as_os_str().is_empty())
            .collect());
    }
    let home = home()?;
    Ok(vec![home.clone(), home.join(".routines"), data_root()?])
}

/// `<root>/breath` — the ruach's store.
#[must_use]
pub fn breath_dir(data_root: &Path) -> PathBuf {
    data_root.join(BREATH_DIR_NAME)
}

/// `<root>/breath/problems.md` — what the shift engine knows is open, for the
/// ruach to read against.
#[must_use]
pub fn breath_problems(data_root: &Path) -> PathBuf {
    breath_dir(data_root).join("problems.md")
}

/// `<root>/breath/sources.toml` — the ruach's feeds and their learned state.
#[must_use]
pub fn breath_sources(data_root: &Path) -> PathBuf {
    breath_dir(data_root).join("sources.toml")
}

/// `<root>/breath/ledger` — `/gigaresearch`'s claim ledger.
#[must_use]
pub fn breath_ledger(data_root: &Path) -> PathBuf {
    breath_dir(data_root).join("ledger")
}

/// `<root>/breath/passes` — one directory per ruach pass.
#[must_use]
pub fn breath_passes(data_root: &Path) -> PathBuf {
    breath_dir(data_root).join("passes")
}

/// `<root>/breath/passes/<id>` — a pass's `pass.json` and `prompt.md`.
#[must_use]
pub fn breath_pass(data_root: &Path, id: &str) -> PathBuf {
    breath_passes(data_root).join(id)
}

/// `<root>/score` — the weekly score pages.
#[must_use]
pub fn score_dir(data_root: &Path) -> PathBuf {
    data_root.join(SCORE_DIR_NAME)
}

/// `<root>/shifts` — one directory per shift.
#[must_use]
pub fn shifts_dir(data_root: &Path) -> PathBuf {
    data_root.join(SHIFTS_DIR_NAME)
}

/// `<root>/shifts/<id>` — a shift's brief, handoff and `meta.json`.
#[must_use]
pub fn shift_dir(data_root: &Path, shift: &str) -> PathBuf {
    shifts_dir(data_root).join(shift)
}

/// `<root>/shifts/current.json` — the open shift; absent between shifts.
#[must_use]
pub fn shift_current(data_root: &Path) -> PathBuf {
    shifts_dir(data_root).join("current.json")
}

/// `<root>/.reports-read` — `report`'s read cursor, runtime state in the
/// dotted tier: one ISO-8601 line, the start of the newest report shown.
#[must_use]
pub fn reports_read(data_root: &Path) -> PathBuf {
    data_root.join(".reports-read")
}

/// The five-hour window file: `$CLAUDE_WINDOW_FILE`, else
/// `$XDG_STATE_HOME/claude/window.json`, else `~/.local/state/claude/window.json`.
pub fn claude_window_file() -> Result<PathBuf> {
    if let Some(file) = env::var_os(CLAUDE_WINDOW_ENV).filter(|file| !file.is_empty()) {
        return Ok(PathBuf::from(file));
    }
    let state = match env::var_os("XDG_STATE_HOME").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => home()?.join(".local").join("state"),
    };
    Ok(state.join("claude").join("window.json"))
}

/// The machine's prompt templates: `$SANDMAN_PROMPTS`, else the `prompts/`
/// directory of sandman's trunk checkout in the grove. Not the build's own
/// source tree — the installed binary is built in a scratch worktree that is
/// gone by the time it runs, and the trunk is where the Sabbath edits land.
pub fn prompts_dir() -> Result<PathBuf> {
    if let Some(dir) = env::var_os(PROMPTS_ENV).filter(|dir| !dir.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    Ok(grove_code()?
        .join("jgeschwendt")
        .join("sandman")
        .join("main")
        .join("prompts"))
}

/// The operator's out-of-session message store: `$SANDMAN_THREADS`, else
/// `~/.threads`.
#[must_use]
pub fn threads_dir(home: &Path) -> PathBuf {
    env::var_os(THREADS_ENV)
        .filter(|dir| !dir.is_empty())
        .map_or_else(|| home.join(".threads"), PathBuf::from)
}

/// The routines' run state, one directory per routine holding `runs.jsonl`:
/// `$SANDMAN_ROUTINES_STATE`, else `~/.routines/.state`.
pub fn routines_state() -> Result<PathBuf> {
    match env::var_os(ROUTINES_STATE_ENV).filter(|dir| !dir.is_empty()) {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => Ok(home()?.join(".routines").join(".state")),
    }
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
        archive_claude_dir, archive_day_dir, breath_ledger, breath_pass, breath_passes,
        breath_problems, breath_sources, claude_jobs_dir, claude_projects_dir, dream_dir,
        grove_manifest, log_dir, pending_take, pending_takes_dir, recent_dir, run_log, score_dir,
        shift_current, shift_dir, tildify, trace_dir, trail_day, trail_dir,
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
        assert_eq!(trail_dir(root), Path::new("/data/trail"));
        assert_eq!(
            trail_day(root, Timestamp::from_unix_seconds(1_786_018_297)),
            Path::new("/data/trail/2026/08/06.jsonl")
        );
        assert_eq!(
            grove_manifest(Path::new("/Users/you/.grove/code")),
            Path::new("/Users/you/.grove/manifest.toml")
        );
        assert_eq!(pending_takes_dir(root), Path::new("/data/pending-takes"));
        assert_eq!(breath_ledger(root), Path::new("/data/breath/ledger"));
        assert_eq!(breath_passes(root), Path::new("/data/breath/passes"));
        assert_eq!(
            breath_pass(root, "b-20261016T160000Z-skim"),
            Path::new("/data/breath/passes/b-20261016T160000Z-skim")
        );
        assert_eq!(breath_problems(root), Path::new("/data/breath/problems.md"));
        assert_eq!(breath_sources(root), Path::new("/data/breath/sources.toml"));
        assert_eq!(score_dir(root), Path::new("/data/score"));
        assert_eq!(
            shift_dir(root, "s-20261009T2300Z"),
            Path::new("/data/shifts/s-20261009T2300Z")
        );
        assert_eq!(shift_current(root), Path::new("/data/shifts/current.json"));
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
