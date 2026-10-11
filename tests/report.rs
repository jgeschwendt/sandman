//! `report`, end to end: the built binary against a temp `$HOME` holding
//! shifts with their `report.md`, proposal threads and the read cursor.
//! Nothing here runs a model, the network or git.

use std::env;
use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

/// Disambiguates directories created within the same process.
static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A fabricated home, removed on drop.
struct Home(PathBuf);

impl Home {
    /// Three reported shifts — one in 2026-W41, two in W42 — an open one
    /// without a report, and four untitled threads of which one proposal
    /// awaits the owner. `pad` lengthens every report's Landed section.
    fn new(pad: usize) -> Self {
        let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
        let home =
            Self(env::temp_dir().join(format!("sandman-report-{}-{serial}", std::process::id())));
        let _ = fs::remove_dir_all(&home.0);
        for (shift, started, eye) in [
            ("s-20261009T1400Z", "2026-10-09T14:00:00Z", "- a1"),
            ("s-20261013T1400Z", "2026-10-13T14:00:00Z", "none."),
            ("s-20261014T1400Z", "2026-10-14T14:00:00Z", "- c1\n- c2"),
            ("s-20261015T1400Z", "2026-10-15T14:00:00Z", ""),
        ] {
            home.write(
                &format!(".sandman/shifts/{shift}/meta.json"),
                &format!(r#"{{"shift":"{shift}","started":"{started}"}}"#),
            );
            if !eye.is_empty() {
                home.write(
                    &format!(".sandman/shifts/{shift}/report.md"),
                    &format!(
                        "# {shift}\n## Worth your eye\n{eye}\n## Landed\n{}\n## Numbers\nnone.\n",
                        "x".repeat(pad)
                    ),
                );
            }
        }
        home.write(
            ".sandman/shifts/current.json",
            r#"{"session_id":"own","shift":"s-20261015T1400Z"}"#,
        );
        for (id, front) in [
            (
                "p1",
                "state: open\nwaiting: user\nkey: proposal:jgeschwendt/alpha:cache",
            ),
            (
                "p2",
                "state: open\nwaiting: agent\nkey: proposal:jgeschwendt/alpha:split\nanswer: approve\ndecided: 2026-10-12T11:00:00Z",
            ),
            (
                "p3",
                "state: resolved\nwaiting: user\nkey: proposal:jgeschwendt/beta:gone",
            ),
            ("t1", "state: open\nwaiting: user"),
        ] {
            home.write(
                &format!(".threads/{id}.md"),
                &format!("---\n{front}\nopened: 2026-10-12T10:00:00Z\n---\n"),
            );
        }
        home
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        fs::write(&path, text).expect("write");
    }

    fn report(&self, shift: &str) -> String {
        fs::read_to_string(self.0.join(format!(".sandman/shifts/{shift}/report.md")))
            .expect("report")
    }

    fn cursor(&self) -> String {
        fs::read_to_string(self.0.join(".sandman/.reports-read")).unwrap_or_default()
    }

    fn run(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_sandman"))
            .arg("report")
            .args(args)
            .env_clear()
            .env("HOME", &self.0)
            .env("SANDMAN_ROOT", self.0.join(".sandman"))
            // keeps the binary's coverage when `cargo llvm-cov` drives the suite
            .envs(env::var_os("LLVM_PROFILE_FILE").map(|file| ("LLVM_PROFILE_FILE", file)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run sandman");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(stdin.as_bytes())
            .expect("payload");
        child.wait_with_output().expect("output")
    }

    /// `report <args>`'s stdout, asserting `code`.
    fn out(&self, args: &[&str], stdin: &str, code: i32) -> String {
        let output = self.run(args, stdin);
        assert_eq!(
            output.status.code(),
            Some(code),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const AWAITING: &str = "## Proposals awaiting you\n- proposal:jgeschwendt/alpha:cache · untitled · opened 2026-10-12T10:00:00Z\n";

#[test]
fn the_digest_reads_unread_reports_newest_first_and_marks_them_read() {
    let empty = Home(env::temp_dir().join(format!("sandman-report-empty-{}", std::process::id())));
    assert_eq!(
        empty.out(&[], "", 0),
        "# Reports since the beginning\n\nnothing new.\n"
    );

    let home = Home::new(0);
    let [a, b, c] = ["s-20261009T1400Z", "s-20261013T1400Z", "s-20261014T1400Z"]
        .map(|shift| format!("\n---\n\n{}", home.report(shift)));
    let all = format!(
        "# Reports since the beginning\n\n## Worth your eye\n- c1 — s-20261014T1400Z\n- c2 — s-20261014T1400Z\n- a1 — s-20261009T1400Z\n\n{AWAITING}{c}{b}{a}"
    );
    assert_eq!(home.out(&[], "", 0), all);
    assert_eq!(home.cursor(), "", "only --mark-read moves the cursor");
    assert_eq!(home.out(&["--mark-read"], "", 0), all);
    assert_eq!(home.cursor(), "2026-10-14T14:00:00Z\n");
    assert_eq!(
        home.out(&[], "", 0),
        format!("# Reports since 2026-10-14T14:00:00Z\n\n## Worth your eye\nnone.\n\n{AWAITING}")
    );

    // The selectors ignore the cursor, and never move it back.
    assert_eq!(home.out(&["--all"], "", 0), all);
    assert_eq!(
        home.out(&["--week", "2026-W41", "--mark-read"], "", 0),
        format!(
            "# Reports in 2026-W41\n\n## Worth your eye\n- a1 — s-20261009T1400Z\n\n{AWAITING}{a}"
        )
    );
    assert_eq!(home.cursor(), "2026-10-14T14:00:00Z\n");
    assert_eq!(
        home.out(&["--since", "2026-10-13T14:00:00Z"], "", 0),
        format!(
            "# Reports since 2026-10-13T14:00:00Z\n\n## Worth your eye\n- c1 — s-20261014T1400Z\n- c2 — s-20261014T1400Z\n\n{AWAITING}{c}{b}"
        )
    );
    for args in [
        &["--all", "--week", "2026-W41"][..],
        &["--since", "yesterday"],
        &["--week", "2026-W99"],
        &["--hook", "--all"],
        &["--hook", "--mark-read"],
        &["--bogus"],
    ] {
        home.out(args, "", 2);
    }
    // An unwritable cursor fails the run once the digest is out.
    let cursor = home.0.join(".sandman/.reports-read");
    fs::remove_file(&cursor).expect("cursor");
    fs::create_dir(&cursor).expect("a directory in its place");
    assert_eq!(home.out(&["--all", "--mark-read"], "", 1), all);
}

#[test]
fn the_hook_shows_the_owner_each_report_once_within_the_cap() {
    let home = Home::new(3_500);
    let root = home.0.join(".sandman").display().to_string();
    // The machine's own sessions: the open shift's, and any under the data root.
    for payload in [
        r#"{"session_id":"own","cwd":"/tmp"}"#.to_owned(),
        format!(r#"{{"session_id":"breath","cwd":"{root}/breath"}}"#),
    ] {
        assert_eq!(home.out(&["--hook"], &payload, 0), "");
    }
    assert_eq!(home.cursor(), "");
    home.out(&["--hook"], "not json", 1);

    let owner = r#"{"session_id":"owner","cwd":"/tmp"}"#;
    let shown = home.out(&["--hook"], owner, 0);
    assert!(shown.starts_with(r#"{"hookSpecificOutput":{"#), "{shown}");
    assert!(shown.contains("additionalContext"), "{shown}");
    // Only the newest report fits; every Worth-your-eye bullet still shows.
    for kept in [
        "- c2",
        "- a1",
        "proposal:jgeschwendt/alpha:cache",
        "2 older report(s) cut at 6000 characters",
    ] {
        assert!(shown.contains(kept), "{kept} missing: {shown}");
    }
    assert_eq!(shown.matches("## Landed").count(), 1, "{shown}");
    assert_eq!(home.cursor(), "2026-10-14T14:00:00Z\n");
    assert_eq!(home.out(&["--hook"], owner, 0), "", "each report once");
}
