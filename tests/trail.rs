//! The trail and reconcile, end to end: the built binary against a temp
//! `$HOME`/`$SANDMAN_ROOT` and a fixture grove of real git repositories.
//!
//! Every git process here — the fixture's and the binary's — runs with the
//! environment cleared, `GIT_CONFIG_GLOBAL=/dev/null` and a fixed identity,
//! so nothing reads the operator's config and nothing reaches the host repo.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use common::{GIT_ENV, Machine, code, stderr, stdout};

/// `$HOME` holding `.sandman` and `.grove`.
impl Machine {
    fn run(&self, args: &[&str]) -> Output {
        self.sandman(args)
            .env("SANDMAN_GROVE_CODE", self.grove_code())
            .env("SANDMAN_RECONCILE_EXTRA", "")
            .env("SANDMAN_ROOT", self.root())
            .envs(GIT_ENV)
            .output()
            .expect("run sandman")
    }

    /// Run the binary and hand back its stdout, asserting it exited 0.
    fn ok(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert_eq!(code(&output), 0, "{args:?}: {}", stderr(&output));
        stdout(&output)
    }

    /// Append an event and return its id.
    fn append(&self, args: &[&str]) -> String {
        let mut full = vec!["trail", "append"];
        full.extend_from_slice(args);
        self.ok(&full).trim().to_owned()
    }
}

/// Run git in `dir` with a scrubbed environment; stdout, asserting success.
fn git(dir: &Path, args: &[&str]) -> String {
    common::git(dir, args).unwrap_or_else(|error| panic!("git {args:?}: {error}"))
}

/// A fixture repo at `<code>/o/fixture/main` holding a base commit and one
/// commit per `(content, event)` pair, each carrying the machine trailers.
/// Returns the checkout and the commit shas, oldest first.
fn fixture(machine: &Machine, commits: &[(&str, &str)]) -> (PathBuf, Vec<String>) {
    fs::create_dir_all(machine.home.join(".grove")).expect("grove");
    // The manifest stand-in is empty: the tree itself must still be found.
    fs::write(machine.home.join(".grove").join("manifest.toml"), "").expect("manifest");
    let checkout = machine.grove_code().join("o").join("fixture").join("main");
    fs::create_dir_all(&checkout).expect("checkout");
    git(&checkout, &["init", "--quiet", "--initial-branch=main"]);
    fs::write(checkout.join("file.txt"), "base\n").expect("base");
    git(&checkout, &["add", "file.txt"]);
    git(&checkout, &["commit", "--quiet", "-m", "base"]);
    let mut shas = Vec::new();
    for (index, (content, event)) in commits.iter().enumerate() {
        fs::write(checkout.join("file.txt"), format!("{content}\n")).expect("content");
        let message = format!("change {index}\n\nShift: s-test\nEvent: {event}");
        git(&checkout, &["commit", "--quiet", "-am", &message]);
        shas.push(git(&checkout, &["rev-parse", "HEAD"]).trim().to_owned());
    }
    (checkout, shas)
}

/// Seed one bank memory whose `source:` line is `source`.
fn memory(machine: &Machine, file: &str, source: &str) -> PathBuf {
    let bank = machine.root().join("memories").join("-bank");
    fs::create_dir_all(&bank).expect("bank");
    let path = bank.join(file);
    fs::write(
        &path,
        format!(
            "---\nname: {file}\ndescription: d\ntype: project\nsource: {source}\n---\n\nbody\n"
        ),
    )
    .expect("memory");
    path
}

#[test]
fn append_and_log_round_trip_with_sorted_keys_and_a_null_parent() {
    let machine = Machine::new("round-trip");
    let first = machine.append(&[
        "--kind",
        "change",
        "--shift",
        "s-1",
        "--repo",
        "o/r",
        "--branch",
        "shift/s-1",
        "--sha",
        "9b32c84aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "--rationale",
        "tightened the brief",
        "--cost",
        "0.42",
        "--model",
        "opus",
        "--ref",
        "t65",
        "--ref",
        "https://example.com",
    ]);
    assert!(first.starts_with("e-"), "{first}");
    let second = machine.append(&[
        "--json",
        &format!(r#"{{"kind":"promote","parent":"{first}","refs":["memo.md"]}}"#),
    ]);

    let raw = machine.ok(&["trail", "log", "--json"]);
    let lines: Vec<&str> = raw.lines().collect();
    assert_eq!(lines.len(), 2, "{raw}");
    assert!(
        lines[0].starts_with(r#"{"at":""#)
            && lines[0].contains(&format!(
                r#""branch":"shift/s-1","cost_usd":0.42,"id":"{first}","kind":"change","model":"opus","parent":null,"rationale":"tightened the brief","refs":["t65","https://example.com"],"repo":"o/r","sha":"9b32c84aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","shift":"s-1"}}"#
            )),
        "{}",
        lines[0]
    );
    // Absent keys are omitted; `parent` and `refs` never are.
    assert!(
        lines[1].ends_with(&format!(
            r#""id":"{second}","kind":"promote","parent":"{first}","refs":["memo.md"]}}"#
        )),
        "{}",
        lines[1]
    );
    // The file is today's, and `today` names it.
    let today = machine.ok(&["trail", "today"]);
    assert_eq!(
        fs::read_to_string(today.trim()).expect("today's file"),
        format!("{raw}")
    );

    let human = machine.ok(&["trail", "log"]);
    let rows: Vec<&str> = human.lines().collect();
    assert_eq!(rows.len(), 2);
    assert!(
        rows[0].ends_with(&format!(
            "  {first}  change  o/r  9b32c84  tightened the brief"
        )),
        "{}",
        rows[0]
    );
    assert!(rows[1].ends_with(&format!("  {second}  promote  -  -  -")));

    let filtered = machine.ok(&["trail", "log", "--kind", "promote"]);
    assert_eq!(filtered.lines().count(), 1);
    assert!(filtered.contains(&second));
    assert_eq!(
        machine
            .ok(&["trail", "log", "--repo", "o/r"])
            .lines()
            .count(),
        1
    );
    assert_eq!(machine.ok(&["trail", "log", "--shift", "s-2"]), "");
    assert_eq!(
        machine
            .ok(&["trail", "log", "--since", "2999-01-01"])
            .lines()
            .count(),
        0
    );
}

#[test]
fn ids_are_unique_within_the_days_file() {
    let machine = Machine::new("unique");
    let ids: std::collections::BTreeSet<String> = (0..12)
        .map(|_| machine.append(&["--kind", "breath"]))
        .collect();
    assert_eq!(ids.len(), 12);
    assert_eq!(machine.trail_bytes().lines().count(), 12);
}

#[test]
fn an_unknown_kind_is_a_usage_failure_that_writes_nothing() {
    let machine = Machine::new("bad-kind");
    for args in [
        &["trail", "append", "--kind", "nonsense"][..],
        &["trail", "append", "--json", r#"{"kind":"nonsense"}"#],
        &["trail", "append"],
        &["trail", "append", "--kind", "change", "--cost", "-1"],
        &["trail", "frobnicate"],
        &["trail", "append", "--kind", "decision"],
    ] {
        let output = machine.run(args);
        assert_eq!(code(&output), 2, "{args:?}: {}", stderr(&output));
    }
    // `decision` is the owner's answer, written by the page alone.
    let decision = machine.run(&["trail", "append", "--json", r#"{"kind":"decision"}"#]);
    assert!(
        stderr(&decision).contains("written by `report serve` only"),
        "{}",
        stderr(&decision)
    );
    // A parent the trail does not hold is a failure, not a usage error.
    let orphan = machine.run(&["trail", "append", "--kind", "change", "--parent", "e-nope"]);
    assert_eq!(code(&orphan), 1, "{}", stderr(&orphan));
    assert_eq!(machine.trail_bytes(), "");
}

#[test]
fn trailers_print_the_two_lines_a_machine_commit_carries() {
    let machine = Machine::new("trailers");
    assert_eq!(
        machine.ok(&["trail", "trailers", "--shift", "s-1", "--event", "e-2"]),
        "Shift: s-1\nEvent: e-2\n"
    );
    assert_eq!(
        code(&machine.run(&["trail", "trailers", "--shift", "s-1"])),
        2
    );
}

#[test]
fn revoke_is_idempotent_and_show_lists_descendants_and_commits() {
    let machine = Machine::new("revoke-show");
    let root = machine.append(&["--kind", "promote", "--rationale", "the root"]);
    let child = machine.append(&["--kind", "change", "--parent", &root]);
    let grandchild = machine.append(&["--kind", "change", "--parent", &child]);
    let stranger = machine.append(&["--kind", "change"]);
    let (_, shas) = fixture(&machine, &[("one", &child)]);

    let revocation = machine.ok(&["trail", "revoke", &root, "--reason", "wrong path"]);
    let again = machine.run(&["trail", "revoke", &root, "--reason", "still wrong"]);
    assert_eq!(code(&again), 0, "{}", stderr(&again));
    assert!(
        stderr(&again).contains("already revoked"),
        "{}",
        stderr(&again)
    );
    assert_eq!(stdout(&again), revocation);
    let revokes = machine.ok(&["trail", "log", "--kind", "revoke", "--json"]);
    assert_eq!(revokes.lines().count(), 1, "{revokes}");
    assert!(revokes.contains(&format!(r#""parent":"{root}""#)));
    assert!(revokes.contains(r#""rationale":"wrong path""#));
    assert_eq!(
        code(&machine.run(&["trail", "revoke", "e-nope", "--reason", "x"])),
        1
    );
    assert_eq!(code(&machine.run(&["trail", "revoke", &root])), 2);

    let alone = machine.ok(&["trail", "show", &stranger]);
    assert!(
        alone.contains("descendants:\n  -\n") && alone.ends_with("commits:\n  -\n"),
        "{alone}"
    );
    let shown = machine.ok(&["trail", "show", &root]);
    assert!(shown.contains(&format!("id: {root}")), "{shown}");
    assert!(shown.contains("rationale: the root"), "{shown}");
    assert!(
        shown.contains(&child) && shown.contains(&grandchild),
        "{shown}"
    );
    assert!(!shown.contains(&stranger), "{shown}");
    assert!(shown.contains(revocation.trim()), "{shown}");
    assert!(
        shown.contains(&format!("o/fixture@{}  {child}", shas[0])),
        "{shown}"
    );
}

#[test]
fn reconcile_reverts_the_revoked_line_and_names_the_memories_to_retire() {
    let machine = Machine::new("reconcile");
    let first = machine.append(&["--kind", "promote", "--shift", "s-test"]);
    let second = machine.append(&["--kind", "change", "--shift", "s-test", "--parent", &first]);
    let (checkout, shas) = fixture(&machine, &[("one", &first), ("two", &second)]);
    let head = git(&checkout, &["rev-parse", "HEAD"]);
    let doomed = memory(&machine, "project_doomed.md", &format!("dream · {second}"));
    memory(&machine, "project_kept.md", "remember 2026-10-09T00:00:00Z");

    machine.ok(&["trail", "revoke", &first, "--reason", "wrong path"]);
    let report = machine.ok(&["reconcile", &first]);

    // Both commits reverted, newest first, on the reconcile branch.
    let branch = format!("reconcile/{first}");
    let subjects = git(&checkout, &["log", &branch, "--format=%s", "-4"]);
    assert_eq!(
        subjects.lines().collect::<Vec<_>>(),
        [
            "Revert \"change 0\"",
            "Revert \"change 1\"",
            "change 1",
            "change 0"
        ],
        "{report}"
    );
    assert_eq!(
        git(&checkout, &["show", &format!("{branch}:file.txt")]),
        "base\n"
    );
    for sha in &shas {
        assert!(report.contains(&format!("reverted: {sha}")), "{report}");
    }

    // The operator's checkout is where it was, and no scratch tree is left.
    assert_eq!(git(&checkout, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        fs::read_to_string(checkout.join("file.txt")).expect("file"),
        "two\n"
    );
    assert_eq!(git(&checkout, &["worktree", "list"]).lines().count(), 1);

    // One reconcile event, its id on every revert commit.
    let events = machine.ok(&["trail", "log", "--kind", "reconcile", "--json"]);
    assert_eq!(events.lines().count(), 1, "{events}");
    let line = events.trim();
    assert!(line.contains(&format!(r#""parent":"{first}""#)), "{line}");
    assert!(line.contains(r#""shift":"reconcile""#), "{line}");
    for sha in &shas {
        assert!(line.contains(&format!("o/fixture@{sha}")), "{line}");
    }
    assert!(line.contains("conflicts: none"), "{line}");
    let id_at = line.find(r#""id":""#).expect("id") + 6;
    let id = &line[id_at..id_at + line[id_at..].find('"').expect("id end")];
    let bodies = git(&checkout, &["log", &branch, "--format=%B", "-2"]);
    assert_eq!(bodies.matches("Shift: reconcile").count(), 2, "{bodies}");
    assert_eq!(
        bodies.matches(&format!("Event: {id}")).count(),
        2,
        "{bodies}"
    );
    assert!(report.contains(&format!("event: {id}")), "{report}");

    // The memory naming the descendant is listed for retirement, untouched.
    let listed = report
        .split("for reflect to retire:")
        .nth(1)
        .expect("retirement section");
    assert!(listed.contains("project_doomed.md"), "{report}");
    assert!(!listed.contains("project_kept.md"), "{report}");
    assert!(doomed.is_file());

    // A second run finds the work done: no new commits, both hits already
    // reverted, and no second event on the trail.
    let trail = machine.trail_bytes();
    let rerun = machine.ok(&["reconcile", &first]);
    assert_eq!(machine.trail_bytes(), trail, "{rerun}");
    assert!(rerun.contains("event: none"), "{rerun}");
    assert_eq!(
        git(&checkout, &["rev-list", "--count", &branch]),
        "5\n",
        "{rerun}"
    );
    for sha in &shas {
        assert!(
            rerun.contains(&format!("already reverted: {sha}")),
            "{rerun}"
        );
    }
}

#[test]
fn a_dry_run_reports_the_plan_and_changes_nothing() {
    let machine = Machine::new("dry-run");
    let first = machine.append(&["--kind", "promote"]);
    let second = machine.append(&["--kind", "change", "--parent", &first]);
    let (checkout, shas) = fixture(&machine, &[("one", &first), ("two", &second)]);
    memory(&machine, "project_doomed.md", &second);
    let trail = machine.trail_bytes();
    let refs = git(&checkout, &["for-each-ref"]);

    let report = machine.ok(&["reconcile", &first, "--dry-run"]);
    assert!(report.starts_with("dry run"), "{report}");
    let planned: Vec<&str> = report
        .lines()
        .filter_map(|line| line.trim().strip_prefix("planned: "))
        .collect();
    assert_eq!(planned, [shas[1].as_str(), shas[0].as_str()], "{report}");
    assert!(report.contains("project_doomed.md"), "{report}");

    assert_eq!(machine.trail_bytes(), trail);
    assert_eq!(git(&checkout, &["for-each-ref"]), refs);
    assert_eq!(git(&checkout, &["worktree", "list"]).lines().count(), 1);

    assert_eq!(code(&machine.run(&["reconcile", "e-nope"])), 1);
    assert_eq!(code(&machine.run(&["reconcile"])), 2);
    let lonely = machine.append(&["--kind", "change"]);
    let report = machine.ok(&["reconcile", &lonely, "--dry-run"]);
    assert!(
        report.contains("no commit carries a revoked event"),
        "{report}"
    );

    // A reconcile branch left behind trunk fails the repo; one at trunk is reused.
    let branch = format!("reconcile/{first}");
    git(&checkout, &["branch", &branch, "main~2"]);
    let behind = machine.run(&["reconcile", &first]);
    assert_eq!(code(&behind), 1);
    assert!(
        stdout(&behind).contains(&format!("{branch} does not contain refs/heads/main")),
        "{}",
        stdout(&behind)
    );
    git(&checkout, &["branch", "--force", &branch, "main"]);
    // Checked out elsewhere, it cannot be added as the scratch tree.
    let elsewhere = machine.home.join("elsewhere");
    let elsewhere_arg = elsewhere.display().to_string();
    git(
        &checkout,
        &["worktree", "add", "--quiet", &elsewhere_arg, &branch],
    );
    assert_eq!(code(&machine.run(&["reconcile", &first])), 1);
    git(&checkout, &["worktree", "remove", &elsewhere_arg]);
    let reused = machine.ok(&["reconcile", &first]);
    assert!(
        reused.contains(&format!("reverted: {}", shas[0])),
        "{reused}"
    );
    assert_eq!(git(&checkout, &["worktree", "list"]).lines().count(), 1);
}

#[test]
fn a_conflicting_revert_is_aborted_and_reported() {
    let machine = Machine::new("conflict");
    let first = machine.append(&["--kind", "change"]);
    // A later hand edit of the same line: reverting the machine's commit
    // cannot apply cleanly.
    let (checkout, shas) = fixture(&machine, &[("one", &first), ("hand edit", "e-human")]);

    let output = machine.run(&["reconcile", &first]);
    assert_eq!(code(&output), 1, "a conflict fails the pass");
    let report = stdout(&output);
    assert_eq!(
        report.matches(shas[0].as_str()).count(),
        1,
        "the hit prints once, under conflict: {report}"
    );
    assert!(
        report.contains(&format!("conflict: {}", shas[0])),
        "{report}"
    );
    assert!(stderr(&output).contains("o/fixture"), "{}", stderr(&output));
    // Nothing was reverted, so the branch the pass cut is gone again.
    assert_eq!(
        git(&checkout, &["for-each-ref", "refs/heads/reconcile"]),
        ""
    );
    assert_eq!(git(&checkout, &["worktree", "list"]).lines().count(), 1);
    assert_eq!(git(&checkout, &["status", "--porcelain"]), "");
    let events = machine.ok(&["trail", "log", "--kind", "reconcile", "--json"]);
    assert!(
        events.contains(&format!("conflicts: o/fixture@{}", shas[0])),
        "{events}"
    );
}

/// Commit everything in `dir` with the machine trailers; the new sha.
fn commit(dir: &Path, subject: &str, event: Option<&str>) -> String {
    git(dir, &["add", "--all"]);
    let message = event.map_or_else(
        || subject.to_owned(),
        |event| format!("{subject}\n\nShift: s-test\nEvent: {event}"),
    );
    git(dir, &["commit", "--quiet", "-m", &message]);
    git(dir, &["rev-parse", "HEAD"]).trim().to_owned()
}

#[test]
fn a_failing_commit_hook_never_blocks_a_revert_or_strands_the_scratch_tree() {
    let machine = Machine::new("hooks");
    let first = machine.append(&["--kind", "change"]);
    let (checkout, shas) = fixture(&machine, &[("one", &first)]);
    for hook in ["commit-msg", "post-checkout", "pre-commit"] {
        let path = checkout.join(".git").join("hooks").join(hook);
        machine.script(path, "echo refused by the suite >&2\nexit 1\n");
    }

    let report = machine.ok(&["reconcile", &first]);
    assert!(
        report.contains(&format!("reverted: {}", shas[0])),
        "{report}"
    );
    let branch = format!("reconcile/{first}");
    assert_eq!(
        git(&checkout, &["show", &format!("{branch}:file.txt")]),
        "base\n"
    );
    assert_eq!(git(&checkout, &["worktree", "list"]).lines().count(), 1);

    let rerun = machine.ok(&["reconcile", &first]);
    assert!(
        rerun.contains(&format!("already reverted: {}", shas[0])),
        "{rerun}"
    );
    assert_eq!(git(&checkout, &["worktree", "list"]).lines().count(), 1);
}

#[test]
fn hits_off_trunk_are_skipped_and_the_branch_starts_from_trunk() {
    let machine = Machine::new("trunk");
    let event = machine.append(&["--kind", "change", "--shift", "s-test"]);
    let (main, _) = fixture(&machine, &[]);
    let shift = machine
        .grove_code()
        .join("o")
        .join("fixture")
        .join("shift-x");
    let shift_arg = shift.display().to_string();
    git(
        &main,
        &[
            "worktree", "add", "--quiet", "-b", "shift/x", &shift_arg, "main",
        ],
    );
    fs::write(shift.join("other.txt"), "hit\n").expect("hit");
    let hit = commit(&shift, "the hit", Some(&event));

    // Unmerged: trunk does not reach the hit, so nothing is reverted.
    let report = machine.ok(&["reconcile", &event]);
    assert!(report.contains(&format!("off trunk: {hit}")), "{report}");
    assert!(!report.contains("reverted:"), "{report}");
    assert_eq!(
        git(&main, &["for-each-ref", "refs/heads/reconcile"]),
        "",
        "no branch for nothing to revert"
    );

    // Merged, and the shift branch moves on: the revert lands on trunk's line.
    git(&main, &["merge", "--quiet", "--ff-only", "shift/x"]);
    let trunk = git(&main, &["rev-parse", "HEAD"]).trim().to_owned();
    fs::write(shift.join("extra.txt"), "unrelated\n").expect("unrelated");
    let unrelated = commit(&shift, "unrelated", None);
    let report = machine.ok(&["reconcile", &event]);
    assert!(report.contains(&format!("reverted: {hit}")), "{report}");
    assert!(report.contains("from refs/heads/main"), "{report}");
    let branch = format!("reconcile/{event}");
    assert_eq!(
        git(&main, &["rev-parse", &format!("{branch}^")]).trim(),
        trunk
    );
    let history = git(&main, &["rev-list", &branch]);
    assert!(!history.contains(&unrelated), "{history}");
}

#[test]
fn revoking_a_shift_reverts_every_commit_it_made() {
    let machine = Machine::new("shift-wide");
    let first = machine.append(&["--kind", "change", "--shift", "s-test"]);
    let second = machine.append(&["--kind", "change", "--shift", "s-test"]);
    let stranger = machine.append(&["--kind", "change", "--shift", "s-other"]);
    // The third commit's event never reached the trail; its Shift: still counts.
    let (checkout, shas) = fixture(
        &machine,
        &[("one", &first), ("two", &second), ("three", "e-unlogged")],
    );
    // Another shift's commit in the same repo is no hit.
    fs::write(checkout.join("other.txt"), "stranger\n").expect("stranger");
    git(&checkout, &["add", "other.txt"]);
    git(
        &checkout,
        &[
            "commit",
            "--quiet",
            "-m",
            &format!("stranger\n\nShift: s-other\nEvent: {stranger}"),
        ],
    );
    let foreign = git(&checkout, &["rev-parse", "HEAD"]).trim().to_owned();

    let revocation = machine.ok(&["trail", "revoke", "s-test", "--reason", "the whole shift"]);
    let again = machine.ok(&["trail", "revoke", "s-test", "--reason", "twice"]);
    assert_eq!(again, revocation, "a shift is revoked once");
    let revokes = machine.ok(&["trail", "log", "--kind", "revoke", "--json"]);
    assert_eq!(revokes.lines().count(), 1, "{revokes}");
    assert!(revokes.contains(r#""parent":null"#), "{revokes}");
    assert!(revokes.contains(r#""refs":["shift:s-test"]"#), "{revokes}");
    assert_eq!(
        code(&machine.run(&["trail", "revoke", "s-nope", "--reason", "x"])),
        1
    );

    let report = machine.ok(&["reconcile", "s-test"]);
    for sha in &shas {
        assert!(report.contains(&format!("reverted: {sha}")), "{report}");
    }
    assert!(
        report.contains(&first) && report.contains(&second),
        "{report}"
    );
    assert!(!report.contains(&stranger), "{report}");
    assert!(!report.contains(&foreign), "{report}");
    assert_eq!(
        git(&checkout, &["show", "reconcile/s-test:file.txt"]),
        "base\n"
    );
    assert_eq!(
        git(&checkout, &["show", "reconcile/s-test:other.txt"]),
        "stranger\n",
        "another shift's commit survives"
    );
    let events = machine.ok(&["trail", "log", "--kind", "reconcile", "--json"]);
    assert!(events.contains(r#""parent":null"#), "{events}");
    assert!(
        events.contains(r#""refs":["shift:s-test","o/fixture@"#),
        "{events}"
    );
}

/// The trailers for `event` under shift `s-test`.
fn trailered(subject: &str, event: &str) -> String {
    format!("{subject}\n\nShift: s-test\nEvent: {event}")
}

/// Reconcile `event`, asserting the repo is refused at `merge` as a conflict
/// with nothing reverted and no branch cut.
fn refused_at(machine: &Machine, checkout: &Path, event: &str, merge: &str) {
    let output = machine.run(&["reconcile", event]);
    let report = stdout(&output);
    assert_eq!(code(&output), 1, "{report}");
    assert!(report.contains(&format!("conflict: {merge}")), "{report}");
    assert!(report.contains("fast-forward only"), "{report}");
    assert!(!report.contains("reverted:"), "{report}");
    assert_eq!(git(checkout, &["for-each-ref", "refs/heads/reconcile"]), "");
}

#[test]
fn a_criss_cross_is_refused_with_nothing_reverted() {
    let machine = Machine::new("criss-cross");
    let event = machine.append(&["--kind", "change", "--shift", "s-test"]);
    let (checkout, _) = fixture(&machine, &[]);
    git(&checkout, &["switch", "--quiet", "-c", "shift/x"]);
    fs::write(checkout.join("other.txt"), "shift work\n").expect("work");
    commit(&checkout, "work", Some(&event));
    git(&checkout, &["switch", "--quiet", "main"]);
    fs::write(checkout.join("file.txt"), "trunk moved\n").expect("trunk");
    commit(&checkout, "trunk edit", None);
    // Each side merges the other's tip, then trunk takes the shift's merge.
    let merge = |into: &str, from: &str| {
        git(&checkout, &["switch", "--quiet", into]);
        let message = trailered(&format!("merge {from}"), &event);
        git(
            &checkout,
            &["merge", "--quiet", "--no-ff", from, "-m", &message],
        );
    };
    git(&checkout, &["branch", "trunk-tip", "main"]);
    git(&checkout, &["branch", "shift-tip", "shift/x"]);
    merge("shift/x", "trunk-tip");
    merge("main", "shift-tip");
    merge("main", "shift/x");
    let top = git(&checkout, &["rev-parse", "main"]).trim().to_owned();
    refused_at(&machine, &checkout, &event, &top);
}

#[test]
fn a_merged_foreign_branch_is_refused_and_the_humans_commit_survives() {
    let machine = Machine::new("foreign");
    let event = machine.append(&["--kind", "change", "--shift", "s-test"]);
    let (checkout, _) = fixture(&machine, &[]);
    git(&checkout, &["switch", "--quiet", "-c", "human"]);
    fs::write(checkout.join("human.txt"), "a human's work\n").expect("human");
    commit(&checkout, "human work", None);
    git(&checkout, &["switch", "--quiet", "-c", "shift/x", "main"]);
    fs::write(checkout.join("other.txt"), "shift work\n").expect("work");
    commit(&checkout, "work", Some(&event));
    let message = trailered("merge human", &event);
    git(
        &checkout,
        &["merge", "--quiet", "--no-ff", "human", "-m", &message],
    );
    let merge = git(&checkout, &["rev-parse", "HEAD"]).trim().to_owned();
    git(&checkout, &["switch", "--quiet", "main"]);
    git(&checkout, &["merge", "--quiet", "--ff-only", "shift/x"]);
    refused_at(&machine, &checkout, &event, &merge);
    assert_eq!(
        git(&checkout, &["show", "main:human.txt"]),
        "a human's work\n"
    );
}

#[test]
fn a_fast_forwarded_shift_is_reverted_past_an_untrailered_merge() {
    let machine = Machine::new("ff");
    let event = machine.append(&["--kind", "change", "--shift", "s-test"]);
    let (checkout, _) = fixture(&machine, &[]);
    git(&checkout, &["switch", "--quiet", "-c", "human"]);
    fs::write(checkout.join("human.txt"), "a human's work\n").expect("human");
    commit(&checkout, "human work", None);
    git(&checkout, &["switch", "--quiet", "main"]);
    git(
        &checkout,
        &["merge", "--quiet", "--no-ff", "human", "-m", "merge human"],
    );
    git(&checkout, &["switch", "--quiet", "-c", "shift/x"]);
    let shas: Vec<String> = ["one", "two"]
        .iter()
        .map(|content| {
            fs::write(checkout.join("file.txt"), format!("{content}\n")).expect("work");
            commit(&checkout, content, Some(&event))
        })
        .collect();
    git(&checkout, &["switch", "--quiet", "main"]);
    git(&checkout, &["merge", "--quiet", "--ff-only", "shift/x"]);

    let report = machine.ok(&["reconcile", &event]);
    for sha in &shas {
        assert!(report.contains(&format!("reverted: {sha}")), "{report}");
    }
    let branch = format!("reconcile/{event}");
    assert_eq!(
        git(&checkout, &["show", &format!("{branch}:file.txt")]),
        "base\n"
    );
    assert_eq!(
        git(&checkout, &["show", &format!("{branch}:human.txt")]),
        "a human's work\n"
    );

    let tip = git(&checkout, &["rev-parse", &branch]);
    let rerun = machine.ok(&["reconcile", &event]);
    assert_eq!(git(&checkout, &["rev-parse", &branch]), tip, "{rerun}");
    assert!(!rerun.contains("  reverted: "), "{rerun}");
}

#[test]
fn a_repo_with_no_trunk_to_name_fails_until_the_manifest_names_it() {
    let machine = Machine::new("no-trunk");
    let event = machine.append(&["--kind", "change", "--shift", "s-test"]);
    let grove = machine.home.join(".grove");
    fs::create_dir_all(&grove).expect("grove");
    fs::write(grove.join("manifest.toml"), "").expect("manifest");
    let checkout = machine.grove_code().join("o").join("fixture").join("trunk");
    fs::create_dir_all(&checkout).expect("checkout");
    git(&checkout, &["init", "--quiet", "--initial-branch=trunk"]);
    fs::write(checkout.join("file.txt"), "base\n").expect("base");
    commit(&checkout, "base", None);
    git(&checkout, &["switch", "--quiet", "-c", "shift/y"]);
    fs::write(checkout.join("file.txt"), "shift\n").expect("work");
    let hit = commit(&checkout, "work", Some(&event));

    let output = machine.run(&["reconcile", &event]);
    let report = stdout(&output);
    assert_eq!(code(&output), 1, "{report}");
    assert!(report.contains("failed: no trunk"), "{report}");
    assert!(
        report.contains(r#"~/.grove/manifest.toml as `trunk = "…"` under [roots."o/fixture"]"#),
        "{report}"
    );
    assert!(!report.contains("reverted:"), "{report}");
    assert_eq!(
        git(&checkout, &["for-each-ref", "refs/heads/reconcile"]),
        ""
    );
    assert_eq!(
        git(&checkout, &["show", "HEAD:file.txt"]),
        "shift\n",
        "nothing reverted"
    );

    fs::write(
        grove.join("manifest.toml"),
        "[roots.\"o/fixture\"]\ntrunk = \"trunk\"\n",
    )
    .expect("manifest");
    let report = machine.ok(&["reconcile", &event]);
    assert!(report.contains("from refs/heads/trunk"), "{report}");
    assert!(report.contains(&format!("off trunk: {hit}")), "{report}");
}

#[test]
fn a_manifest_trunk_that_does_not_resolve_fails_the_repo() {
    let machine = Machine::new("bad-trunk");
    let event = machine.append(&["--kind", "change", "--shift", "s-test"]);
    let (checkout, _) = fixture(&machine, &[("one", &event)]);
    fs::write(
        machine.home.join(".grove").join("manifest.toml"),
        "[roots.\"o/fixture\"]\ntrunk = \"trunk\"\n",
    )
    .expect("manifest");

    let output = machine.run(&["reconcile", &event]);
    let report = stdout(&output);
    assert_eq!(code(&output), 1, "{report}");
    assert!(
        report.contains("failed: no trunk: the manifest names refs/heads/trunk"),
        "{report}"
    );
    assert_eq!(
        git(&checkout, &["for-each-ref", "refs/heads/reconcile"]),
        ""
    );
}

#[test]
fn a_hand_revert_without_gits_line_is_already_reverted() {
    let machine = Machine::new("hand-revert");
    let first = machine.append(&["--kind", "change"]);
    let (checkout, shas) = fixture(&machine, &[("one", &first)]);
    fs::write(checkout.join("file.txt"), "base\n").expect("undo");
    commit(&checkout, "Revert \"change 0\"", None);
    let trail = machine.trail_bytes();

    let report = machine.ok(&["reconcile", &first]);
    assert!(
        report.contains(&format!("already reverted: {}", shas[0])),
        "{report}"
    );
    assert!(!report.contains("  reverted:"), "{report}");
    assert_eq!(
        git(&checkout, &["for-each-ref", "refs/heads/reconcile"]),
        ""
    );
    assert_eq!(git(&checkout, &["worktree", "list"]).lines().count(), 1);
    assert_eq!(git(&checkout, &["status", "--porcelain"]), "");
    assert_eq!(machine.trail_bytes(), trail);
}

#[test]
fn a_revoked_revert_does_not_count_its_target_as_reverted() {
    let machine = Machine::new("self-revert");
    let first = machine.append(&["--kind", "change"]);
    let (checkout, shas) = fixture(&machine, &[("bad", &first)]);
    fs::write(checkout.join("file.txt"), "base\n").expect("undo");
    let subject = format!("Revert \"change 0\"\n\nThis reverts commit {}.", shas[0]);
    commit(&checkout, &subject, Some(&first));
    fs::write(checkout.join("other.txt"), "y\n").expect("other");
    commit(&checkout, "y", Some(&first));

    let report = machine.ok(&["reconcile", &first]);
    assert!(!report.contains("already reverted"), "{report}");
    assert_eq!(
        git(&checkout, &["show", &format!("reconcile/{first}:file.txt")]),
        "base\n"
    );
}

#[test]
fn a_trunk_behind_origin_fails_until_it_is_fast_forwarded() {
    let machine = Machine::new("stale-trunk");
    let event = machine.append(&["--kind", "change"]);
    let (checkout, _) = fixture(&machine, &[]);
    let origin = machine.home.join("origin.git");
    let other = machine.home.join("other");
    let (origin_arg, other_arg) = (origin.display().to_string(), other.display().to_string());
    git(&checkout, &["clone", "--quiet", "--bare", ".", &origin_arg]);
    git(&checkout, &["remote", "add", "origin", &origin_arg]);
    git(
        &machine.home,
        &["clone", "--quiet", &origin_arg, &other_arg],
    );
    fs::write(other.join("file.txt"), "shipped\n").expect("work");
    let hit = commit(&other, "shipped", Some(&event));
    git(&other, &["push", "--quiet", "origin", "main"]);
    git(&checkout, &["fetch", "--quiet", "origin"]);

    let output = machine.run(&["reconcile", &event]);
    let report = stdout(&output);
    assert_eq!(code(&output), 1, "{report}");
    assert!(
        report.contains(
            "failed: trunk is behind origin/main: fast-forward it (grove sync) and rerun"
        ),
        "{report}"
    );
    assert!(!report.contains("reverted:"), "{report}");
    assert!(!report.contains("off trunk"), "{report}");
    assert_eq!(
        git(&checkout, &["for-each-ref", "refs/heads/reconcile"]),
        ""
    );

    git(&checkout, &["merge", "--quiet", "--ff-only", "origin/main"]);
    let report = machine.ok(&["reconcile", &event]);
    assert!(report.contains(&format!("reverted: {hit}")), "{report}");
}
