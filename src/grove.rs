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

/// A directory's subdirectories, sorted; none when it is missing.
fn subdirs(dir: &Path) -> Vec<PathBuf> {
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
    for variable in LOCAL_ENV {
        command.env_remove(variable);
    }
    command
}

/// A commit whose `Event:` trailer names one of the events asked about.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Hit {
    /// The event it names.
    pub event: String,
    /// The full sha.
    pub sha: String,
    /// Its `Shift:` trailer, when it carries one.
    pub shift: Option<String>,
}

/// Every commit reachable from any ref in the checkout's repository whose
/// `Event:` trailer is in `events`, newest first.
pub fn hits(dir: &Path, events: &BTreeSet<String>) -> Result<Vec<Hit>> {
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
        let (Some(sha), Some(event), shift) = (fields.next(), fields.next(), fields.next()) else {
            continue;
        };
        let shift = shift
            .map(str::trim)
            .filter(|shift| !shift.is_empty())
            .map(ToOwned::to_owned);
        for event in event.split(',').map(str::trim) {
            if events.contains(event) {
                found.push(Hit {
                    event: event.to_owned(),
                    sha: sha.trim().to_owned(),
                    shift: shift.clone(),
                });
                break;
            }
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

/// The repos holding a commit that names one of `events`, keyed by the git
/// common directory their checkouts share. A checkout git refuses is skipped.
#[must_use]
pub fn repos_with_hits(roots: &Roots, events: &BTreeSet<String>) -> Vec<Repo> {
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
        let Ok(hits) = hits(&checkout.path, events) else {
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
    use super::{Roots, checkouts, manifest_repos};
    use crate::testutil::TempDir;
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
        assert!(manifest_repos(&temp.path().join("absent.toml")).is_empty());
    }

    #[test]
    fn checkouts_are_branch_directories_carrying_git() {
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
        fs::create_dir_all(extra.join(".git")).expect("extra");
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
    }
}
