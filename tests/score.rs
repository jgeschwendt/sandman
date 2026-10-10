//! `score`, end to end: the built binary against a temp `$HOME`/`$SANDMAN_ROOT`
//! holding a fixture trail, shifts, breath store, threads and routine state,
//! with `$SANDMAN_NOW` pinning the week. Nothing here runs a model or git.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Disambiguates directories created within the same process.
static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Friday 2026-10-16, noon in New York — ISO week 2026-W42.
const FRIDAY: &str = "2026-10-16T16:00:00Z";

/// A fabricated home, removed on drop.
struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
        Self(env::temp_dir().join(format!(
            "sandman-score-{}-{serial}-{nanos}",
            std::process::id()
        )))
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        fs::write(&path, text).expect("write");
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_sandman"))
            .args(args)
            .env_clear()
            .env("HOME", &self.0)
            .env("PATH", env::var("PATH").unwrap_or_default())
            .env("SANDMAN_GROVE_CODE", self.0.join(".grove/code"))
            .env("SANDMAN_NOW", FRIDAY)
            .env("SANDMAN_ROOT", self.0.join(".sandman"))
            .env("SANDMAN_ROUTINES_STATE", self.0.join("state"))
            .env("SANDMAN_THREADS", self.0.join("threads"))
            .output()
            .expect("run sandman")
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// One trail line at `at`, its id unique by `n`, `extra` the remaining keys.
fn event(n: usize, at: &str, kind: &str, extra: &str) -> String {
    format!(r#"{{"at":"{at}","id":"e-{n:04}","kind":"{kind}","parent":null,{extra}}}"#)
}

/// The week 2026-W42 as the machine would have left it.
fn fixture() -> Home {
    let home = Home::new();
    for slug in ["alpha", "beta", "gamma"] {
        fs::create_dir_all(
            home.0
                .join(format!(".grove/code/jgeschwendt/{slug}/main/.git")),
        )
        .expect("checkout");
    }
    for (shift, repo, started) in [
        ("s-20261005T1400Z", "alpha", "2026-10-05T14:00:00Z"),
        ("s-20261013T1400Z", "alpha", "2026-10-13T14:00:00Z"),
        // a Sabbath shift covers nothing
        ("s-20261017T1400Z", "beta", "2026-10-17T14:00:00Z"),
    ] {
        home.write(
            &format!(".sandman/shifts/{shift}/meta.json"),
            &format!(r#"{{"repo":"jgeschwendt/{repo}","shift":"{shift}","started":"{started}"}}"#),
        );
    }
    let alpha = r#""repo":"jgeschwendt/alpha""#;
    let days: [(&str, Vec<String>); 5] = [
        (
            "08",
            vec![event(
                1,
                "2026-10-08T12:00:00Z",
                "correction",
                r#""rationale":"never use pkill -f to stop a server","refs":[]"#,
            )],
        ),
        // Sunday 23:00 in New York: still last week
        (
            "12",
            vec![event(2, "2026-10-12T03:00:00Z", "change", r#""refs":[]"#)],
        ),
        (
            "13",
            vec![
                event(
                    3,
                    "2026-10-13T14:00:00Z",
                    "shift.start",
                    &format!(
                        r#""model":"opus","refs":["brief:x","fp:handoff.previous=aaaa","fp:shift.id=1111","fp:trail.recent=bbbb"],"shift":"s-20261013T1400Z",{alpha}"#
                    ),
                ),
                event(
                    4,
                    "2026-10-13T15:30:00Z",
                    "change",
                    &format!(r#""cost_usd":1.5,"model":"opus","refs":[],"sha":"abc1234",{alpha}"#),
                ),
                event(
                    5,
                    "2026-10-13T16:00:00Z",
                    "change",
                    &format!(r#""cost_usd":0.5,"refs":[],{alpha}"#),
                ),
                event(
                    6,
                    "2026-10-13T18:00:00Z",
                    "shift.handoff",
                    &format!(
                        r#""rationale":"Landed the lexer","refs":[],"shift":"s-20261013T1400Z",{alpha}"#
                    ),
                ),
            ],
        ),
        (
            "15",
            vec![
                event(
                    7,
                    "2026-10-15T14:00:00Z",
                    "shift.start",
                    r#""refs":["fp:handoff.previous=aaaa","fp:shift.id=2222","fp:trail.recent=cccc"],"shift":"s-20261015T1400Z""#,
                ),
                event(
                    8,
                    "2026-10-15T15:00:00Z",
                    "shift.handoff",
                    r#""rationale":"Abandoned: the session died","refs":["abandoned"],"shift":"s-20261015T1400Z""#,
                ),
                event(
                    9,
                    "2026-10-15T16:00:00Z",
                    "correction",
                    r#""rationale":"never use pkill -f to stop the server","refs":[]"#,
                ),
                event(
                    10,
                    "2026-10-15T16:10:00Z",
                    "correction",
                    r#""rationale":"prefer the yaml extension","refs":[]"#,
                ),
                event(11, "2026-10-15T16:20:00Z", "guard", r#""refs":[]"#),
                event(
                    12,
                    "2026-10-15T16:30:00Z",
                    "breath",
                    r#""refs":["breath:b.md","state:adopted"]"#,
                ),
                event(13, "2026-10-15T16:40:00Z", "breath", r#""refs":["url:x"]"#),
            ],
        ),
        // Sunday 23:30 in New York: the week's last hour
        (
            "19",
            vec![event(
                14,
                "2026-10-19T03:30:00Z",
                "change",
                r#""refs":[],"repo":"jgeschwendt/beta""#,
            )],
        ),
    ];
    for (day, lines) in days {
        home.write(
            &format!(".sandman/trail/2026/10/{day}.jsonl"),
            &(lines.join("\n") + "\n"),
        );
    }
    for (file, front) in [
        (
            "2026-10-13-a.md",
            "state: new\nfiled: 2026-10-13T12:00:00Z\nupdated: 2026-10-13T12:00:00Z",
        ),
        (
            "2026-10-01-b.md",
            "state: adopted\nfiled: 2026-10-01T12:00:00Z\nupdated: 2026-10-15T12:00:00Z\nadopted: 2026-10-15T12:00:00Z",
        ),
        (
            "2026-10-02-c.md",
            "state: dropped\nfiled: 2026-10-02T12:00:00Z\nupdated: 2026-10-14T12:00:00Z",
        ),
        (
            "2026-09-01-d.md",
            "state: spiked\nfiled: 2026-09-01T12:00:00Z\nupdated: 2026-09-02T12:00:00Z",
        ),
    ] {
        home.write(
            &format!(".sandman/breath/{file}"),
            &format!("---\ntitle: {file}\nurl: https://example.com/{file}\nsource: arxiv\nrepo: jgeschwendt/alpha\nproblem: p\n{front}\n---\n\nSummary.\n"),
        );
    }
    home.write(
        ".sandman/breath/sources.toml",
        "[arxiv]\nkind = \"atom\"\nurl = \"https://example.com/feed\"\nweight = 1.2\nbudget = 10\nbears_on = [\"*\"]\nfiled = 3\nadopted = 1\n",
    );
    for (id, state, rest) in [
        ("t1", "open", "kind: decision\nkey: shift:s-20261015T1400Z"),
        ("t2", "open", "kind: problem\nanchor: session:x"),
        (
            "t3",
            "resolved",
            "kind: decision\nkey: shift:s-20261013T1400Z",
        ),
    ] {
        home.write(
            &format!("threads/{id}.md"),
            &format!("---\nstate: {state}\n{rest}\n---\n\n# {id}\n"),
        );
    }
    let run = |started: &str, exit: u8| format!(r#"{{"started":"{started}","exit":{exit}}}"#);
    home.write(
        "state/shift/runs.jsonl",
        &[
            run("2026-10-09T12:00:00Z", 1),
            run("2026-10-14T12:00:00Z", 0),
            run("2026-10-14T12:05:00Z", 1),
            run("2026-10-15T12:00:00Z", 0),
        ]
        .join("\n"),
    );
    home.write(
        "state/breathe-skim/runs.jsonl",
        &run("2026-10-14T13:17:00Z", 0),
    );
    home.write(
        "state/heartbeat/runs.jsonl",
        &run("2026-10-14T13:00:00Z", 1),
    );
    home
}

fn ok(output: &Output) -> String {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn score_writes_the_weeks_page_worst_three_first() {
    let home = fixture();
    let path = ok(&home.run(&["score"]));
    assert_eq!(
        path.trim(),
        home.0
            .join(".sandman/score/2026-W42.md")
            .display()
            .to_string()
    );
    let page = fs::read_to_string(path.trim()).expect("page");
    assert!(
        page.starts_with(
            "---\nweek: 2026-W42\nwritten: 2026-10-16T16:00:00Z\nfrom: 2026-10-12\nto: 2026-10-18\n---\n\n## Worst three\n\n| metric | value | target |\n| --- | --- | --- |\n| cost unrecorded | 10 | 0 |\n| coverage | 1/3 | 3/3 |\n| abandoned shifts | 1 | 0 |\n\n## Coverage\n"
        ),
        "{page}"
    );
    for row in [
        "| jgeschwendt/alpha | 1 |\n| jgeschwendt/beta | 0 |\n| jgeschwendt/gamma | 0 |\n| covered | 1/3 |",
        "| opened | handed off | abandoned | mean minutes |\n| --- | --- | --- | --- |\n| 2 | 1 | 1 | 150 |",
        "| breath | 2 |\n| change | 3 |\n| correction | 2 |\n| decay | 0 |\n| guard | 1 |",
        "| 12 | 1 | 2.00 | 10 |",
        "| - | 7 |\n| jgeschwendt/alpha | 4 |\n| jgeschwendt/beta | 1 |",
        "| - | 10 |\n| opus | 2 |",
        "| - | 1 |\n| adopted | 1 |",
        "| corrections | recurring |\n| --- | --- |\n| 2 | 1 |",
        "| slots | fresh | fresh ratio |\n| --- | --- | --- |\n| 4 | 3 | 0.75 |",
        "| filed | spiked | adopted | dropped |\n| --- | --- | --- | --- |\n| 1 | 0 | 1 | 1 |",
        "| arxiv | 1.20 | 3 | 1 |",
        "| decision | 1 |\n| problem | 1 |\n| from shifts | 1 |",
        "| breathe-skim | 1 | 0 |\n| shift | 3 | 1 |",
    ] {
        assert!(page.contains(row), "missing {row:?} in:\n{page}");
    }
    assert!(
        !page.contains("heartbeat"),
        "only shift and breathe-* routines"
    );

    // Rerunning rewrites the same page.
    ok(&home.run(&["score"]));
    assert_eq!(fs::read_to_string(path.trim()).expect("page"), page);
}

#[test]
fn score_json_prints_the_metrics_and_week_names_a_week() {
    let home = fixture();
    let json = ok(&home.run(&["score", "--json"]));
    for expected in [
        r#""context":{"fresh":3,"fresh_ratio":0.75,"slots":4}"#,
        r#""corrections":{"recurring":1,"total":2}"#,
        r#""coverage":{"jgeschwendt/alpha":1,"jgeschwendt/beta":0,"jgeschwendt/gamma":0}"#,
        r#""routines":{"breathe-skim":{"failed":0,"ticks":1},"shift":{"failed":1,"ticks":3}}"#,
        r#""shifts":{"abandoned":1,"handed_off":1,"mean_minutes":150,"opened":2}"#,
        r#""threads":{"by_kind":{"decision":1,"problem":1},"from_shifts":1}"#,
        r#""changes_with_sha":1,"cost_unrecorded":10,"cost_usd":2,"events":12"#,
        r#""week":"2026-W42","worst":[{"metric":"cost unrecorded","target":"0","value":"10"},{"metric":"coverage","target":"3/3","value":"1/3"},{"metric":"abandoned shifts","target":"0","value":"1"}]"#,
    ] {
        assert!(json.contains(expected), "missing {expected} in {json}");
    }
    assert!(home.0.join(".sandman/score/2026-W42.md").is_file());

    // Last week holds one correction and one change; nothing recurs before it.
    let json = ok(&home.run(&["score", "--week", "2026-W41", "--json"]));
    assert!(
        json.contains(r#""corrections":{"recurring":0,"total":1}"#),
        "{json}"
    );
    assert!(json.contains(r#""from":"2026-10-05""#), "{json}");
    assert!(home.0.join(".sandman/score/2026-W41.md").is_file());

    for bad in ["2026-W54", "W42", "2027-W53"] {
        let output = home.run(&["score", "--week", bad]);
        assert_eq!(output.status.code(), Some(2), "{bad}");
    }
}
