//! The build stamp: which commit this binary was built from.
//!
//! A journal line is only as useful as the build that wrote it — the installed
//! binary is routinely a worktree behind the tree, and a line that cannot name
//! its build cannot be trusted to describe the code that is running. So the
//! commit is baked in at compile time and carried by every line.
//!
//! Nothing here may fail the build: a source tree with no git at all (a
//! vendored crate, a tarball) still compiles, and its stamp reads `unknown`.

use std::process::Command;

fn main() {
    // `.git` is a file, not a directory, in every worktree of a bare clone —
    // the path does not exist, so cargo re-runs this script on every build.
    // That is the intent: two `git` calls are cheaper than a stale stamp.
    println!("cargo::rerun-if-changed=.git/HEAD");
    println!("cargo::rustc-env=SANDMAN_GIT_HASH={}", hash());
}

/// The short commit, `-dirty` when the tree carries uncommitted work.
fn hash() -> String {
    let Some(hash) = git(&["rev-parse", "--short", "HEAD"]) else {
        return "unknown".to_owned();
    };
    match git(&["status", "--porcelain"]) {
        Some(status) if !status.is_empty() => format!("{hash}-dirty"),
        // A `git status` that would not run says nothing about the tree, so it
        // says nothing about the stamp either.
        _ => hash,
    }
}

/// One git invocation's trimmed stdout, or `None` for anything but success.
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}
