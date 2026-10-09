//! The trail, end to end: the built binary against a temp
//! `$HOME`/`$SANDMAN_ROOT` and a fixture grove of real git repositories.
//!
//! Every git process here — the fixture's and the binary's — runs with the
//! environment cleared, `GIT_CONFIG_GLOBAL=/dev/null` and a fixed identity,
//! so nothing reads the operator's config and nothing reaches the host repo.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Disambiguates directories created within the same process.
static COUNTER: AtomicU32 = AtomicU32::new(0);

/// The environment every git process gets on top of a cleared one.
const GIT_ENV: [(&str, &str); 6] = [
    ("GIT_AUTHOR_EMAIL", "machine@example.com"),
    ("GIT_AUTHOR_NAME", "machine"),
    ("GIT_COMMITTER_EMAIL", "machine@example.com"),
    ("GIT_COMMITTER_NAME", "machine"),
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
];

/// A fabricated machine: `$HOME` holding `.sandman` and `.grove`.
struct Machine {
    home: PathBuf,
}

impl Machine {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
        let home = env::temp_dir().join(format!(
            "sandman-trail-{label}-{}-{serial}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&home).expect("create the fabricated home");
        Self { home }
    }

    fn root(&self) -> PathBuf {
        self.home.join(".sandman")
    }

    fn grove_code(&self) -> PathBuf {
        self.home.join(".grove").join("code")
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_sandman"))
            .args(args)
            .env_clear()
            .env("HOME", &self.home)
            .env("PATH", env::var("PATH").unwrap_or_default())
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

    /// Every trail file's bytes, concatenated in path order.
    fn trail_bytes(&self) -> String {
        let mut files = Vec::new();
        let mut stack = vec![self.root().join("trail")];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    files.push(path);
                }
            }
        }
        files.sort();
        files
            .into_iter()
            .map(|path| fs::read_to_string(path).expect("trail file"))
            .collect()
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.home);
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("an exit code")
}

/// Run git in `dir` with a scrubbed environment; stdout, asserting success.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env_clear()
        .env("PATH", env::var("PATH").unwrap_or_default())
        .env("HOME", dir)
        .envs(GIT_ENV)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout(&output)
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
    ] {
        let output = machine.run(args);
        assert_eq!(code(&output), 2, "{args:?}: {}", stderr(&output));
    }
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
