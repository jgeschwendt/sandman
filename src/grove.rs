//! The repos the machine commits into, and how to ask them about the trail.
//!
//! Roots are every checkout under the grove's code directory —
//! `<code>/<owner>/<repo>/<branch>/`, the repos read from the grove manifest's
//! `[roots."owner/repo"]` tables and from the tree itself, so a missing or
//! partial manifest loses nothing — plus the extra repos outside it (`~`,
//! `~/.routines`, the data root). Both lists arrive as arguments; `cli`
//! resolves them from `crate::paths`.
//!
//! Every git call here runs with the inherited repository environment
//! (`GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, …) removed: a caller that is
//! itself a git hook would otherwise point every `git -C <checkout>` at the
//! hook's repository instead.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Error, Result};

/// The repository-local variables git exports into hook children —
/// `git rev-parse --local-env-vars`, as of git 2.39.
const LOCAL_ENV: [&str; 14] = [
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_CONFIG",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_PARAMETERS",
    "GIT_DIR",
    "GIT_GRAFT_FILE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_OBJECT_DIRECTORY",
    "GIT_PREFIX",
    "GIT_REPLACE_REF_BASE",
    "GIT_WORK_TREE",
];

/// Where to look for repos.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Roots {
    /// Repos outside the grove, each a checkout in its own right.
    pub extras: Vec<PathBuf>,
    /// `~/.grove/code` or its override.
    pub grove_code: PathBuf,
    /// `$HOME`, folded back to `~` in an extra's label.
    pub home: PathBuf,
    /// The grove manifest — absent is fine.
    pub manifest: PathBuf,
}

/// One working tree.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Checkout {
    /// `owner/repo` for a grove checkout, the `~`-folded path for an extra.
    pub label: String,
    /// The working tree.
    pub path: PathBuf,
}

/// Every checkout the roots hold, grove first, each grove repo's checkouts
/// in name order. A directory counts when it carries a `.git` entry — file
/// (a linked worktree) or directory.
#[must_use]
pub fn checkouts(roots: &Roots) -> Vec<Checkout> {
    let mut repos: BTreeSet<String> = manifest_repos(&roots.manifest).into_iter().collect();
    for owner in subdirs(&roots.grove_code) {
        for repo in subdirs(&owner) {
            if let (Some(owner), Some(repo)) = (name(&owner), name(&repo)) {
                repos.insert(format!("{owner}/{repo}"));
            }
        }
    }
    let mut found = Vec::new();
    for repo in repos {
        for checkout in subdirs(&roots.grove_code.join(&repo)) {
            if is_checkout(&checkout) {
                found.push(Checkout {
                    label: repo.clone(),
                    path: checkout,
                });
            }
        }
    }
    for extra in &roots.extras {
        if is_checkout(extra) {
            found.push(Checkout {
                label: crate::paths::tildify(extra, &roots.home),
                path: extra.clone(),
            });
        }
    }
    found
}

/// The `owner/repo` keys of a grove manifest's `[roots."owner/repo"]` tables.
/// A manifest that is absent or unreadable names none.
#[must_use]
pub fn manifest_repos(manifest: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(manifest) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("[roots.\"")?
                .strip_suffix("\"]")
                .filter(|key| !key.contains('"'))
                .map(ToOwned::to_owned)
        })
        .collect()
}

/// The `trunk = "…"` a manifest root table names, keyed by `owner/repo`. A
/// root without one is absent here; its trunk is `main`.
#[must_use]
pub fn manifest_trunks(manifest: &Path) -> BTreeMap<String, String> {
    let Ok(text) = fs::read_to_string(manifest) else {
        return BTreeMap::new();
    };
    let mut trunks = BTreeMap::new();
    let mut table: Option<String> = None;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            table = line
                .strip_prefix("[roots.\"")
                .and_then(|rest| rest.strip_suffix("\"]"))
                .filter(|key| !key.contains('"'))
                .map(ToOwned::to_owned);
            continue;
        }
        let Some(repo) = &table else {
            continue;
        };
        if let Some((key, value)) = line.split_once('=')
            && key.trim() == "trunk"
        {
            let value = value.trim().trim_matches('"');
            if !value.is_empty() {
                trunks.insert(repo.clone(), value.to_owned());
            }
        }
    }
    trunks
}

/// A directory's subdirectories, sorted; none when it is missing.
pub(crate) fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    dirs
}

/// The last component, as UTF-8.
fn name(path: &Path) -> Option<&str> {
    path.file_name()?.to_str()
}

/// Whether `dir` is the top of a working tree.
fn is_checkout(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// Run `git -C <dir> <args>`; stdout on success, the stderr as the error.
pub fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let output = command(dir)
        .args(args)
        .output()
        .map_err(|source| Error::io(dir, source))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(Error::Git(format!(
            "git {} in {}: {}",
            args.join(" "),
            dir.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

/// Whether `git -C <dir> <args>` exits 0.
#[must_use]
pub fn git_ok(dir: &Path, args: &[&str]) -> bool {
    command(dir)
        .args(args)
        .output()
        .is_ok_and(|output| output.status.success())
}

/// `git -C <dir>` with the repository environment scrubbed.
fn command(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir);
    scrub(&mut command);
    command
}

/// Remove the repository-local git variables from a child's environment —
/// for any tool that runs git underneath (`stele check`), not just git.
pub fn scrub(command: &mut Command) {
    for variable in LOCAL_ENV {
        command.env_remove(variable);
    }
}

/// A commit whose `Event:` trailer names one of the events asked about, or
/// whose `Shift:` trailer names the shift asked about.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Hit {
    /// The event it names — the asked-about one when it names several, else
    /// its first `Event:` value, empty when it carries none.
    pub event: String,
    /// The full sha.
    pub sha: String,
    /// Its `Shift:` trailer, when it carries one.
    pub shift: Option<String>,
}

/// Every commit reachable from any ref in the checkout's repository whose
/// `Event:` trailer is in `events` or whose `Shift:` trailer is `shift`,
/// newest first.
pub fn hits(dir: &Path, events: &BTreeSet<String>, shift: Option<&str>) -> Result<Vec<Hit>> {
    let log = git(
        dir,
        &[
            "log",
            "--all",
            "-z",
            "--format=%H%x1f%(trailers:key=Event,valueonly,separator=%x2C)%x1f%(trailers:key=Shift,valueonly,separator=%x2C)",
        ],
    )?;
    let mut found = Vec::new();
    for record in log.split('\0') {
        let mut fields = record.trim_start_matches('\n').split('\x1f');
        let (Some(sha), Some(named), carried) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let carried = carried
            .map(str::trim)
            .filter(|carried| !carried.is_empty())
            .map(ToOwned::to_owned);
        let named: Vec<&str> = named
            .split(',')
            .map(str::trim)
            .filter(|event| !event.is_empty())
            .collect();
        let asked = named.iter().find(|event| events.contains(**event));
        let by_shift = shift.is_some_and(|shift| {
            carried
                .as_deref()
                .is_some_and(|carried| carried.split(',').any(|one| one.trim() == shift))
        });
        if asked.is_some() || by_shift {
            found.push(Hit {
                event: asked
                    .or_else(|| named.first())
                    .map(|event| (*event).to_owned())
                    .unwrap_or_default(),
                sha: sha.trim().to_owned(),
                shift: carried,
            });
        }
    }
    Ok(found)
}

/// One repository, however many checkouts it has: linked worktrees share
/// their commits, and asking each would count every commit once per tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Repo {
    /// Its checkouts, in the order the roots yielded them.
    pub checkouts: Vec<Checkout>,
    /// The commits naming an asked-about event, newest first.
    pub hits: Vec<Hit>,
    /// `owner/repo`, or the path for an extra.
    pub label: String,
}

/// The repos holding a commit that names one of `events` (or `shift`), keyed
/// by the git common directory their checkouts share. A checkout git refuses
/// is skipped.
#[must_use]
pub fn repos_with_hits(roots: &Roots, events: &BTreeSet<String>, shift: Option<&str>) -> Vec<Repo> {
    let mut repos: BTreeMap<String, Repo> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for checkout in checkouts(roots) {
        let Ok(common) = git(
            &checkout.path,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        ) else {
            continue;
        };
        let common = common.trim().to_owned();
        if let Some(repo) = repos.get_mut(&common) {
            repo.checkouts.push(checkout);
            continue;
        }
        let Ok(hits) = hits(&checkout.path, events, shift) else {
            continue;
        };
        order.push(common.clone());
        repos.insert(
            common,
            Repo {
                hits,
                label: checkout.label.clone(),
                checkouts: vec![checkout],
            },
        );
    }
    order
        .into_iter()
        .filter_map(|common| repos.remove(&common))
        .filter(|repo| !repo.hits.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Roots, checkouts, git, hits, manifest_repos, manifest_trunks, repos_with_hits};
    use crate::testutil::TempDir;
    use std::collections::BTreeSet;
    use std::fs;

    #[test]
    fn the_manifest_names_only_root_tables() {
        let temp = TempDir::new("grove-manifest");
        let manifest = temp.path().join("manifest.toml");
        fs::write(
            &manifest,
            "[roots.\"o/a\"]\nurl = \"x\"\n\n[roots.\"o/a\".worktrees.dev]\nbase = \"main\"\n\n[roots.\"p/b\"]\n",
        )
        .expect("manifest");
        assert_eq!(manifest_repos(&manifest), ["o/a", "p/b"]);
        let absent = temp.path().join("absent.toml");
        assert!(manifest_repos(&absent).is_empty());
        assert!(manifest_trunks(&absent).is_empty());
        fs::write(
            &manifest,
            "[roots.\"o/a\"]\nurl = \"x\"\ntrunk = \"trunk\"\n\n[roots.\"o/a\".worktrees.dev]\ntrunk = \"no\"\n\n[roots.\"p/b\"]\n",
        )
        .expect("manifest");
        let trunks = manifest_trunks(&manifest);
        assert_eq!(trunks.len(), 1);
        assert_eq!(trunks.get("o/a").map(String::as_str), Some("trunk"));
    }

    #[test]
    fn checkouts_are_branch_directories_carrying_git_and_git_refusals_are_skipped() {
        let temp = TempDir::new("grove-checkouts");
        let code = temp.path().join("code");
        for dir in [
            "o/a/main/.git",
            "o/a/dev/.git",
            "o/a/notes",
            "p/b/main/.git",
        ] {
            fs::create_dir_all(code.join(dir)).expect("dir");
        }
        let extra = temp.path().join("home");
        git(temp.path(), &["init", "-q", "home"]).expect("extra");
        // A ref to an object that is not there: `rev-parse` passes, `log` fails.
        fs::write(
            extra.join(".git/refs/heads/broken"),
            format!("{}\n", "1".repeat(40)),
        )
        .expect("broken ref");
        let roots = Roots {
            extras: vec![extra.clone(), temp.path().join("missing")],
            grove_code: code.clone(),
            home: temp.path().to_path_buf(),
            manifest: temp.path().join("manifest.toml"),
        };
        let found: Vec<(String, std::path::PathBuf)> = checkouts(&roots)
            .into_iter()
            .map(|checkout| (checkout.label, checkout.path))
            .collect();
        assert_eq!(
            found,
            [
                ("o/a".to_owned(), code.join("o/a/dev")),
                ("o/a".to_owned(), code.join("o/a/main")),
                ("p/b".to_owned(), code.join("p/b/main")),
                ("~/home".to_owned(), extra),
            ]
        );
        // The fake checkouts are no repositories and the extra's log is
        // refused: none of them is a repo, and nothing fails.
        assert!(hits(&temp.path().join("home"), &BTreeSet::new(), None).is_err());
        assert!(repos_with_hits(&roots, &BTreeSet::new(), None).is_empty());
    }
}
