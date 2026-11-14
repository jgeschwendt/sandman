//! The build identity `sandman version` prints and the daemon's health carries: the short commit,
//! `-dirty` over uncommitted work, `unknown` outside a git checkout; it never fails the build.

use std::process::Command;

fn main() {
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };
    // a commit moves the worktree's HEAD log, staging its index; an edit touches src
    for watched in git(&["rev-parse", "--absolute-git-dir"])
        .map(|dir| [format!("{dir}/logs/HEAD"), format!("{dir}/index")])
        .into_iter()
        .flatten()
        .chain(["Cargo.toml".to_owned(), "src".to_owned()])
    {
        println!("cargo::rerun-if-changed={watched}");
    }
    let build = git(&["rev-parse", "--short", "HEAD"]).map_or_else(
        || "unknown".to_owned(),
        |hash| {
            if git(&["status", "--porcelain"]).is_some_and(|status| !status.is_empty()) {
                format!("{hash}-dirty")
            } else {
                hash
            }
        },
    );
    println!("cargo::rustc-env=SANDMAN_BUILD={build}");
}
