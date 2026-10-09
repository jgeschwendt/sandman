//! `reconcile <event-id>` — walk the trail back past a wrong turn.
//!
//! The revoked set is the event and everything its `parent` chains reach.
//! Every repo the roots hold is asked for commits whose `Event:` trailer names
//! one of them; the hits are reverted newest-first on a `reconcile/<event-id>`
//! branch per repo — new commits, never a reset, never a force. Then one
//! `reconcile` event goes on the trail, and the memories whose `source:` names
//! a revoked event are listed for reflect to retire; reconcile itself never
//! touches a bank.
//!
//! The reverts happen in a temporary linked worktree on the reconcile branch,
//! so the operator's own checkouts — and whatever is uncommitted in them —
//! are never switched or touched.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::bank::MEMORIES_DIR_NAME;
use crate::error::{Error, Result};
use crate::grove::{self, Checkout, Repo, Roots, git, git_ok};
use crate::time::Timestamp;
use crate::verbs::trail::{self, Draft, Event};

/// The `Shift:` trailer every revert commit carries.
pub const SHIFT: &str = "reconcile";

/// Disambiguates scratch worktrees within one process.
static SCRATCH: AtomicU32 = AtomicU32::new(0);

/// What reconcile did in one repository.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepoReport {
    /// Hits the branch already reverts — a rerun skips them.
    pub already: Vec<String>,
    /// `reconcile/<event-id>`.
    pub branch: String,
    /// The checkout whose `HEAD` the branch started from.
    pub checkout: PathBuf,
    /// The hit whose revert conflicted and was aborted; the repo stops there.
    pub conflicts: Vec<String>,
    /// Why the repo could not be worked at all, when it could not.
    pub failed: Option<String>,
    /// `owner/repo`, or the `~`-folded path.
    pub label: String,
    /// Hits on no branch the reconcile branch descends from: nothing to
    /// revert, the branch carrying them is simply left unmerged.
    pub off_head: Vec<String>,
    /// Hits to revert, newest first — what `--dry-run` reports.
    pub planned: Vec<String>,
    /// Hits reverted, newest first.
    pub reverted: Vec<String>,
}

/// The whole pass.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    /// The `reconcile` event appended; `None` on a dry run.
    pub event: Option<Event>,
    /// Memory files whose `source:` names a revoked event — for reflect.
    pub memories: Vec<PathBuf>,
    /// One per repository holding a hit.
    pub repos: Vec<RepoReport>,
    /// The revoked set: the event, then its descendants in trail order.
    pub revoked: Vec<String>,
}

/// Reconcile the trail past `target`.
pub fn reconcile(data_root: &Path, roots: &Roots, target: &str, dry_run: bool) -> Result<Report> {
    let entries = trail::read(data_root, None)?;
    if trail::find(&entries, target).is_none() {
        return Err(Error::not_found("event", target.to_owned()));
    }
    let mut revoked = vec![target.to_owned()];
    revoked.extend(
        trail::descendants(&entries, target)
            .into_iter()
            .map(|event| event.id.clone()),
    );
    let set: BTreeSet<String> = revoked.iter().cloned().collect();

    let event = Event::new(
        Draft {
            kind: "reconcile".to_owned(),
            parent: Some(target.to_owned()),
            shift: Some(SHIFT.to_owned()),
            ..Draft::default()
        },
        Timestamp::now()?,
    );
    let branch = format!("reconcile/{target}");
    let repos: Vec<RepoReport> = grove::repos_with_hits(roots, &set)
        .iter()
        .map(|repo| reconcile_repo(repo, &branch, &event.id, dry_run))
        .collect();
    let memories = memories_naming(data_root, &set);

    if dry_run {
        return Ok(Report {
            event: None,
            memories,
            repos,
            revoked,
        });
    }
    let mut event = event;
    event.draft.refs = repos
        .iter()
        .flat_map(|repo| {
            repo.reverted
                .iter()
                .map(move |sha| format!("{}@{sha}", repo.label))
        })
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
fn reconcile_repo(repo: &Repo, branch: &str, event_id: &str, dry_run: bool) -> RepoReport {
    let mut report = RepoReport {
        branch: branch.to_owned(),
        label: repo.label.clone(),
        ..RepoReport::default()
    };
    let Some(first) = repo.checkouts.first() else {
        report.failed = Some("no checkout".to_owned());
        return report;
    };
    let branch_ref = format!("refs/heads/{branch}");
    let reused = git_ok(
        &first.path,
        &["rev-parse", "--verify", "--quiet", &branch_ref],
    );
    let (checkout, base) = if reused {
        (first, branch)
    } else {
        (best_checkout(repo), "HEAD")
    };
    report.checkout.clone_from(&checkout.path);

    let history = match git(&checkout.path, &["rev-list", base]) {
        Ok(history) => history,
        Err(error) => {
            report.failed = Some(error.to_string());
            return report;
        }
    };
    let already = reverted_by(&checkout.path, base);
    let hits: BTreeSet<&str> = repo.hits.iter().map(|hit| hit.sha.as_str()).collect();
    let reachable: Vec<&str> = history.lines().filter(|sha| hits.contains(sha)).collect();
    for hit in &repo.hits {
        if !reachable.contains(&hit.sha.as_str()) {
            report.off_head.push(hit.sha.clone());
        }
    }
    for sha in reachable {
        if already.contains(sha) {
            report.already.push(sha.to_owned());
        } else {
            report.planned.push(sha.to_owned());
        }
    }
    if dry_run || report.planned.is_empty() {
        return report;
    }

    let scratch = env::temp_dir().join(format!(
        "sandman-reconcile-{}-{}",
        process::id(),
        SCRATCH.fetch_add(1, Ordering::Relaxed)
    ));
    let scratch_arg = scratch.to_string_lossy().into_owned();
    let added = if reused {
        git(&checkout.path, &["worktree", "add", &scratch_arg, branch])
    } else {
        git(
            &checkout.path,
            &["worktree", "add", "-b", branch, &scratch_arg, "HEAD"],
        )
    };
    if let Err(error) = added {
        report.failed = Some(error.to_string());
        return report;
    }
    for sha in report.planned.clone() {
        match revert_one(&scratch, &sha, event_id) {
            Ok(()) => report.reverted.push(sha),
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
    if let Err(error) = git(&checkout.path, &["worktree", "remove", &scratch_arg]) {
        report.failed.get_or_insert_with(|| error.to_string());
    }
    report
}

/// The checkout whose `HEAD` reaches the most hits; the first on a tie.
fn best_checkout(repo: &Repo) -> &Checkout {
    let hits: BTreeSet<&str> = repo.hits.iter().map(|hit| hit.sha.as_str()).collect();
    let mut best = &repo.checkouts[0];
    let mut most = None;
    for checkout in &repo.checkouts {
        let reached = git(&checkout.path, &["rev-list", "HEAD"])
            .map(|history| history.lines().filter(|sha| hits.contains(sha)).count())
            .unwrap_or(0);
        if most.is_none_or(|most| reached > most) {
            most = Some(reached);
            best = checkout;
        }
    }
    best
}

/// The shas `base`'s history already reverts, read off the
/// `This reverts commit <sha>.` line `git revert` writes and reconcile keeps.
fn reverted_by(dir: &Path, base: &str) -> BTreeSet<String> {
    let Ok(log) = git(dir, &["log", base, "--format=%B"]) else {
        return BTreeSet::new();
    };
    log.lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("This reverts commit ")?
                .strip_suffix('.')
                .map(ToOwned::to_owned)
        })
        .collect()
}

/// Why a revert did not land.
enum Undone {
    /// The revert conflicted and was aborted.
    Conflict,
    /// Something else went wrong.
    Failed(Error),
}

/// Revert `sha` in the scratch worktree as one commit carrying the trailers.
fn revert_one(scratch: &Path, sha: &str, event_id: &str) -> std::result::Result<(), Undone> {
    let subject = git(scratch, &["log", "-1", "--format=%s", sha]).map_err(Undone::Failed)?;
    if git(scratch, &["revert", "--no-commit", sha]).is_err() {
        git(scratch, &["revert", "--abort"]).map_err(Undone::Failed)?;
        return Err(Undone::Conflict);
    }
    let message = format!(
        "Revert \"{}\"\n\nThis reverts commit {sha}.\n\n{}",
        subject.trim(),
        trail::trailers(SHIFT, event_id)
    );
    git(scratch, &["commit", "--quiet", "-m", &message]).map_err(Undone::Failed)?;
    Ok(())
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
