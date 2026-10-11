//! `reconcile <event-id|shift-id>` — walk the trail back past a wrong turn.
//!
//! The revoked set is the event and everything its `parent` chains reach —
//! for a shift id, every event of the shift and theirs. Every repo the roots
//! hold is asked for commits whose `Event:` trailer names one of them (or
//! whose `Shift:` trailer names the shift); the hits its trunk reaches are
//! reverted newest-first on a `reconcile/<event-id>` branch cut from that
//! trunk, one per repo: new commits, never a reset, never a force, never the
//! repo's commit hooks. Merges are never reverted: the machine lands by
//! fast-forward only, so a trailered merge refuses its repo for the operator,
//! as does a repo with no trunk to name. Then one
//! `reconcile` event goes on the trail, and the memories whose `source:` names
//! a revoked event are listed for reflect to retire; reconcile itself never
//! touches a bank.
//!
//! The reverts happen in a temporary linked worktree on the reconcile branch,
//! so the operator's own checkouts — and whatever is uncommitted in them —
//! are never switched or touched; it is removed however the pass ends.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::bank::MEMORIES_DIR_NAME;
use crate::error::{Error, Result};
use crate::grove::{self, Repo, Roots, git, git_ok};
use crate::time::Timestamp;
use crate::verbs::trail::{self, Draft, Event};

/// The `Shift:` trailer every revert commit carries.
pub const SHIFT: &str = "reconcile";

/// Disambiguates scratch worktrees within one process.
static SCRATCH: AtomicU32 = AtomicU32::new(0);

/// What reconcile did in one repository.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepoReport {
    /// Hits the branch already reverts, or whose revert changes nothing — a
    /// rerun skips them.
    pub already: Vec<String>,
    /// The trunk ref the branch starts from.
    pub base: String,
    /// `reconcile/<event-id>`.
    pub branch: String,
    /// The checkout the repo was worked through.
    pub checkout: PathBuf,
    /// The hit whose revert conflicted and was aborted, or a trailered
    /// merge; the repo stops there.
    pub conflicts: Vec<String>,
    /// Why the repo could not be worked at all, when it could not.
    pub failed: Option<String>,
    /// `owner/repo`, or the `~`-folded path.
    pub label: String,
    /// What the operator should know, such as a repo with no trunk ref.
    pub notes: Vec<String>,
    /// Hits trunk does not reach — trunk is never behind `origin`, so these
    /// never landed: nothing to revert.
    pub off_trunk: Vec<String>,
    /// Hits to revert, newest first — what `--dry-run` reports.
    pub planned: Vec<String>,
    /// Hits reverted, newest first.
    pub reverted: Vec<String>,
}

impl RepoReport {
    /// Whether the repo ended in a conflict or a failure.
    #[must_use]
    pub fn troubled(&self) -> bool {
        self.failed.is_some() || !self.conflicts.is_empty()
    }
}

/// The whole pass.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    /// The `reconcile` event appended; `None` on a dry run, or when the pass
    /// reverted nothing and met no trouble — a rerun leaves the trail alone.
    pub event: Option<Event>,
    /// Memory files whose `source:` names a revoked event — for reflect.
    pub memories: Vec<PathBuf>,
    /// One per repository holding a hit.
    pub repos: Vec<RepoReport>,
    /// The revoked set, in trail order.
    pub revoked: Vec<String>,
}

/// Reconcile the trail past `target` — an event id, or a shift id whose every
/// event is revoked at once.
pub fn reconcile(data_root: &Path, roots: &Roots, target: &str, dry_run: bool) -> Result<Report> {
    let entries = trail::read(data_root, None)?;
    let revoked = trail::revoked_set(&entries, target)?;
    let set: BTreeSet<String> = revoked.iter().cloned().collect();
    let shift = trail::is_shift_id(target).then_some(target);

    let event = Event::new(
        Draft {
            kind: "reconcile".to_owned(),
            parent: shift.is_none().then(|| target.to_owned()),
            shift: Some(SHIFT.to_owned()),
            ..Draft::default()
        },
        Timestamp::now()?,
    );
    let branch = format!("reconcile/{target}");
    let trunks = grove::manifest_trunks(&roots.manifest);
    let repos: Vec<RepoReport> = grove::repos_with_hits(roots, &set, shift)
        .iter()
        .map(|repo| reconcile_repo(repo, &trunks, &branch, &event.id, dry_run))
        .collect();
    let memories = memories_naming(data_root, &set);

    if dry_run
        || repos
            .iter()
            .all(|repo| repo.reverted.is_empty() && !repo.troubled())
    {
        return Ok(Report {
            event: None,
            memories,
            repos,
            revoked,
        });
    }
    let mut event = event;
    event.draft.refs = shift
        .map(trail::shift_ref)
        .into_iter()
        .chain(repos.iter().flat_map(|repo| {
            repo.reverted
                .iter()
                .map(move |sha| format!("{}@{sha}", repo.label))
        }))
        .collect();
    event.draft.rationale = Some(rationale(&repos));
    let event = trail::append_event(data_root, event, false)?;
    Ok(Report {
        event: Some(event),
        memories,
        repos,
        revoked,
    })
}

/// One line for the trail: what was reverted, and what was not.
fn rationale(repos: &[RepoReport]) -> String {
    let reverted: usize = repos.iter().map(|repo| repo.reverted.len()).sum();
    let mut trouble: Vec<String> = Vec::new();
    for repo in repos {
        trouble.extend(
            repo.conflicts
                .iter()
                .map(|sha| format!("{}@{sha}", repo.label)),
        );
        if let Some(failed) = &repo.failed {
            trouble.push(format!("{} failed: {failed}", repo.label));
        }
    }
    let conflicts = if trouble.is_empty() {
        "none".to_owned()
    } else {
        trouble.join(", ")
    };
    format!(
        "reverted {reverted} commit(s) across {} repo(s); conflicts: {conflicts}",
        repos.len()
    )
}

/// Revert one repository's hits, or plan to.
///
/// The branch starts from the repo's trunk, never from whichever checkout's
/// `HEAD` reaches the hits: a `shift/…` HEAD would carry the shift's other
/// commits onto the reconcile branch. A trunk behind `origin` fails the repo:
/// shipped hits it does not reach would pass for unlanded ones.
fn reconcile_repo(
    repo: &Repo,
    trunks: &BTreeMap<String, String>,
    branch: &str,
    event_id: &str,
    dry_run: bool,
) -> RepoReport {
    let mut report = RepoReport {
        branch: branch.to_owned(),
        label: repo.label.clone(),
        ..RepoReport::default()
    };
    let Some(first) = repo.checkouts.first() else {
        report.failed = Some("no checkout".to_owned());
        return report;
    };
    let dir = &first.path;
    report.checkout.clone_from(dir);
    let trunk = match trunk_ref(dir, &repo.label, trunks.get(&repo.label)) {
        Ok(trunk) => trunk,
        Err(why) => {
            report.failed = Some(why);
            return report;
        }
    };
    report.base.clone_from(&trunk);
    let upstream = trunk.replacen("refs/heads/", "refs/remotes/origin/", 1);
    if git_ok(dir, &["merge-base", "--is-ancestor", &trunk, &upstream])
        && !git_ok(dir, &["merge-base", "--is-ancestor", &upstream, &trunk])
    {
        report.failed = Some(format!(
            "trunk is behind {}: fast-forward it (grove sync) and rerun",
            upstream.trim_start_matches("refs/remotes/")
        ));
        return report;
    }
    let branch_ref = format!("refs/heads/{branch}");
    let reused = git_ok(dir, &["rev-parse", "--verify", "--quiet", &branch_ref]);
    if reused && !git_ok(dir, &["merge-base", "--is-ancestor", &trunk, &branch_ref]) {
        report.failed = Some(format!(
            "{branch} does not contain {trunk}; merge it in or delete the branch, then rerun"
        ));
        return report;
    }

    let history = match git(dir, &["rev-list", "--topo-order", "--parents", &trunk]) {
        Ok(history) => history,
        Err(error) => {
            report.failed = Some(error.to_string());
            return report;
        }
    };
    let already = reverted_by(dir, if reused { &branch_ref } else { &trunk });
    let hits: BTreeSet<&str> = repo.hits.iter().map(|hit| hit.sha.as_str()).collect();
    report.off_trunk = hits
        .iter()
        .filter(|sha| !history.contains(**sha))
        .map(|sha| (*sha).to_owned())
        .collect();
    for line in history.lines() {
        let mut shas = line.split_whitespace();
        let Some(sha) = shas.next().filter(|sha| hits.contains(sha)) else {
            continue;
        };
        if undone(&already, &hits, sha) {
            report.already.push(sha.to_owned());
        } else if shas.nth(1).is_some() {
            report.planned.clear();
            report.conflicts.push(sha.to_owned());
            report.notes.push(format!(
                "merge {sha} carries a revoked trailer, but the machine lands by fast-forward only: a trailered merge is a shift that broke the rule, so the operator reverts it by hand"
            ));
            return report;
        } else {
            report.planned.push(sha.to_owned());
        }
    }
    if dry_run || report.planned.is_empty() {
        return report;
    }

    let scratch = match Scratch::add(dir, branch, if reused { None } else { Some(&trunk) }) {
        Ok(scratch) => scratch,
        Err(error) => {
            report.failed = Some(error.to_string());
            return report;
        }
    };
    for sha in report.planned.clone() {
        match revert_one(&scratch.path, &sha, event_id) {
            Ok(true) => report.reverted.push(sha),
            Ok(false) => report.already.push(sha),
            Err(Undone::Conflict) => {
                report.conflicts.push(sha);
                break;
            }
            Err(Undone::Failed(error)) => {
                report.failed = Some(error.to_string());
                break;
            }
        }
    }
    // a branch this run cut and reverted nothing onto is not left behind
    let removed = scratch.remove().and_then(|()| {
        if reused || !report.reverted.is_empty() {
            return Ok(());
        }
        hookless(dir, &["branch", "-D", "--quiet", branch]).map(drop)
    });
    if let Err(error) = removed {
        report.failed.get_or_insert_with(|| error.to_string());
    }
    report
}

/// The ref a repo's reconcile branch starts from: the manifest's `trunk` when
/// it names one — a name that does not resolve is the repo's failure, never a
/// fallthrough — else `main`, else what `origin/HEAD` names; never `HEAD`,
/// which in a `shift/…` checkout is the very line being revoked.
fn trunk_ref(
    dir: &Path,
    label: &str,
    manifest: Option<&String>,
) -> std::result::Result<String, String> {
    let verified = |name: &str| git_ok(dir, &["rev-parse", "--verify", "--quiet", name]);
    if let Some(trunk) = manifest {
        let trunk = format!("refs/heads/{trunk}");
        return if verified(&trunk) {
            Ok(trunk)
        } else {
            Err(format!(
                "no trunk: the manifest names {trunk}, which does not resolve in {}",
                dir.display()
            ))
        };
    }
    let origin = git(
        dir,
        &["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
    )
    .unwrap_or_default();
    ["refs/heads/main", origin.trim()]
        .into_iter()
        .find(|name| !name.is_empty() && verified(name))
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!(
            "no trunk: no manifest trunk, refs/heads/main or origin/HEAD resolves in {}; name it in ~/.grove/manifest.toml as `trunk = \"…\"` under [roots.\"{label}\"]",
            dir.display()
        ))
}

/// `git` with the repository's hooks switched off. A revert is the owner's
/// reconcile, not a shift's work: a repo's pre-commit suite must neither
/// block nor slow it, and `worktree add` must not run a post-checkout hook.
fn hookless(dir: &Path, args: &[&str]) -> Result<String> {
    let mut full = vec!["-c", "core.hooksPath=/dev/null"];
    full.extend_from_slice(args);
    git(dir, &full)
}

/// A temporary linked worktree on the reconcile branch, removed with
/// `--force` however the pass ends — an early return or a panic included —
/// so a rerun never finds the branch checked out somewhere stale.
struct Scratch {
    /// The checkout it was added from.
    checkout: PathBuf,
    /// Its working tree.
    path: PathBuf,
    /// Whether it still has to be removed.
    live: bool,
}

impl Scratch {
    /// Add it on `branch` — created at `start` when given, else reused.
    fn add(checkout: &Path, branch: &str, start: Option<&str>) -> Result<Self> {
        let path = env::temp_dir().join(format!(
            "sandman-reconcile-{}-{}",
            process::id(),
            SCRATCH.fetch_add(1, Ordering::Relaxed)
        ));
        let arg = path.to_string_lossy().into_owned();
        let mut args = vec!["worktree", "add", "--quiet"];
        match start {
            Some(start) => args.extend(["-b", branch, &arg, start]),
            None => args.extend([arg.as_str(), branch]),
        }
        let scratch = Self {
            checkout: checkout.to_path_buf(),
            path,
            live: true,
        };
        hookless(checkout, &args)?;
        Ok(scratch)
    }

    /// Remove it now, reporting a failure.
    fn remove(mut self) -> Result<()> {
        self.live = false;
        let arg = self.path.to_string_lossy().into_owned();
        hookless(&self.checkout, &["worktree", "remove", "--force", &arg]).map(drop)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if self.live {
            let arg = self.path.to_string_lossy().into_owned();
            let _ = hookless(&self.checkout, &["worktree", "remove", "--force", &arg]);
            let _ = fs::remove_dir_all(&self.path);
            let _ = git(&self.checkout, &["worktree", "prune"]);
        }
    }
}

/// Each sha `base`'s history reverts, mapped to the commits that revert it —
/// read off the `This reverts commit <sha>.` line `git revert` writes and
/// reconcile keeps.
fn reverted_by(dir: &Path, base: &str) -> BTreeMap<String, Vec<String>> {
    let mut reverts: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let Ok(log) = git(dir, &["log", base, "--format=%x00%H%n%B"]) else {
        return reverts;
    };
    for commit in log.split('\0') {
        let mut lines = commit.lines();
        let Some(reverter) = lines.next() else {
            continue;
        };
        for reverted in lines.filter_map(|line| {
            line.trim()
                .strip_prefix("This reverts commit ")?
                .strip_suffix('.')
        }) {
            reverts
                .entry(reverted.to_owned())
                .or_default()
                .push(reverter.to_owned());
        }
    }
    reverts
}

/// Whether `sha`'s change is already undone: some commit reverts it and that
/// revert stands — it is not itself being revoked (a shift that `git revert`ed
/// its own commit) and not itself undone (an operator's revert of the revert).
/// Counting a bare message line would skip the original while reverting its
/// revert, putting the revoked change back.
fn undone(reverts: &BTreeMap<String, Vec<String>>, hits: &BTreeSet<&str>, sha: &str) -> bool {
    reverts.get(sha).is_some_and(|reverters| {
        reverters
            .iter()
            .any(|reverter| !hits.contains(reverter.as_str()) && !undone(reverts, hits, reverter))
    })
}

/// Why a revert did not land.
enum Undone {
    /// The revert conflicted and was aborted.
    Conflict,
    /// Something else went wrong.
    Failed(Error),
}

/// Revert `sha` in the scratch worktree as one commit carrying the trailers;
/// `false`, with nothing committed, when the revert changes nothing — a hand
/// revert without git's `This reverts commit` line already undid it.
fn revert_one(scratch: &Path, sha: &str, event_id: &str) -> std::result::Result<bool, Undone> {
    let subject = git(scratch, &["log", "-1", "--format=%s", sha]).map_err(Undone::Failed)?;
    if hookless(scratch, &["revert", "--no-commit", sha]).is_err() {
        hookless(scratch, &["revert", "--abort"]).map_err(Undone::Failed)?;
        return Err(Undone::Conflict);
    }
    if git_ok(scratch, &["diff", "--cached", "--quiet"]) {
        hookless(scratch, &["revert", "--abort"]).map_err(Undone::Failed)?;
        return Ok(false);
    }
    let message = format!(
        "Revert \"{}\"\n\nThis reverts commit {sha}.\n\n{}",
        subject.trim(),
        trail::trailers(SHIFT, event_id)
    );
    hookless(
        scratch,
        &["commit", "--no-verify", "--quiet", "-m", &message],
    )
    .map_err(Undone::Failed)?;
    Ok(true)
}

/// Every live memory under `<root>/memories` whose `source:` names one of
/// `events`. `_archive/` and the hidden directories are already out of play.
#[must_use]
pub fn memories_naming(data_root: &Path, events: &BTreeSet<String>) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![data_root.join(MEMORIES_DIR_NAME)];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for path in entries.flatten().map(|entry| entry.path()) {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if path.is_dir() {
                if !name.starts_with('.') && name != "_archive" {
                    stack.push(path);
                }
            } else if path.extension().is_some_and(|ext| ext == "md")
                && name != "MEMORY.md"
                && source_line(&path).is_some_and(|source| {
                    events.iter().any(|event| source.contains(event.as_str()))
                })
            {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// The `source:` value of a memory's frontmatter.
fn source_line(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    if lines.next()? != "---" {
        return None;
    }
    lines
        .take_while(|line| *line != "---")
        .find_map(|line| line.strip_prefix("source:"))
        .map(|source| source.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::{RepoReport, memories_naming, rationale};
    use crate::testutil::TempDir;
    use std::collections::BTreeSet;
    use std::fs;

    #[test]
    fn memories_are_matched_on_source_and_archives_are_skipped() {
        let temp = TempDir::new("reconcile-memories");
        let bank = temp.path().join("memories").join("-bank");
        fs::create_dir_all(bank.join("_archive")).expect("bank");
        let memory = |path: std::path::PathBuf, source: &str| {
            fs::write(
                path,
                format!("---\nname: n\ndescription: d\ntype: project\nsource: {source}\n---\n\nbody e-2\n"),
            )
            .expect("memory");
        };
        memory(bank.join("project_hit.md"), "shift s-1 · event e-2");
        memory(bank.join("project_miss.md"), "remember 2026-10-09");
        memory(bank.join("_archive").join("old_project_hit.md"), "e-2");
        let events: BTreeSet<String> = ["e-1".to_owned(), "e-2".to_owned()].into();
        assert_eq!(
            memories_naming(temp.path(), &events),
            [bank.join("project_hit.md")]
        );
    }

    #[test]
    fn the_rationale_names_every_conflict() {
        let clean = RepoReport {
            label: "o/a".to_owned(),
            reverted: vec!["abc".to_owned()],
            ..RepoReport::default()
        };
        assert_eq!(
            rationale(std::slice::from_ref(&clean)),
            "reverted 1 commit(s) across 1 repo(s); conflicts: none"
        );
        let stuck = RepoReport {
            conflicts: vec!["def".to_owned()],
            label: "o/b".to_owned(),
            ..RepoReport::default()
        };
        assert_eq!(
            rationale(&[clean, stuck]),
            "reverted 1 commit(s) across 2 repo(s); conflicts: o/b@def"
        );
    }
}
