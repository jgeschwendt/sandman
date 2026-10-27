//! `report`, end to end: the built binary against a temp `$HOME` holding
//! shifts with their `report.md`, proposal threads and the read cursor.
//! Nothing here runs a model, the network or git.

mod common;

use std::fs;
use std::io::{BufRead, BufReader, Read, Write as _};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};

use common::{Machine, feed, stderr, stdout};

impl Machine {
    /// Three reported shifts — one in 2026-W41, two in W42 — an open one
    /// without a report, and four untitled threads of which one proposal
    /// awaits the owner. `pad` lengthens every report's Landed section.
    fn reported(pad: usize) -> Self {
        let home = Self::new("report");
        for (shift, started, eye) in [
            ("s-20261009T1400Z", "2026-10-09T14:00:00Z", "- a1"),
            ("s-20261013T1400Z", "2026-10-13T14:00:00Z", "none."),
            ("s-20261014T1400Z", "2026-10-14T14:00:00Z", "- c1\n- c2"),
            ("s-20261015T1400Z", "2026-10-15T14:00:00Z", ""),
        ] {
            home.write(
                format!(".sandman/shifts/{shift}/meta.json"),
                &format!(r#"{{"shift":"{shift}","started":"{started}"}}"#),
            );
            if !eye.is_empty() {
                home.write(
                    format!(".sandman/shifts/{shift}/report.md"),
                    &format!(
                        "# {shift}\n## Worth your eye\n{eye}\n## Landed\n{}\n## Numbers\nnone.\n",
                        "x".repeat(pad)
                    ),
                );
            }
        }
        home.write(
            ".sandman/shifts/current.json",
            &format!(
                r#"{{"path":"{}/trunk","session_id":"own","shift":"s-20261015T1400Z"}}"#,
                home.home.display()
            ),
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
                format!(".docket/{id}.md"),
                &format!("---\n{front}\nopened: 2026-10-12T10:00:00Z\n---\n"),
            );
        }
        home
    }

    fn report(&self, shift: &str) -> String {
        fs::read_to_string(self.home.join(format!(".sandman/shifts/{shift}/report.md")))
            .expect("report")
    }

    fn cursor(&self) -> String {
        fs::read_to_string(self.home.join(".sandman/.reports-read")).unwrap_or_default()
    }

    /// `sandman <args>` under this home.
    fn command(&self, args: &[&str]) -> Command {
        let mut command = self.sandman(args);
        command
            .env("SANDMAN_NOW", "2026-10-16T16:00:00Z")
            .env("SANDMAN_ROOT", self.root())
            .env("SANDMAN_DOCKET_BIN", self.home.join("thread"));
        command
    }

    /// `report <args>`'s stdout, asserting `code`.
    fn out(&self, args: &[&str], stdin: &str, code: i32) -> String {
        let output = feed(&mut self.command(&[&["report"], args].concat()), stdin);
        assert_eq!(common::code(&output), code, "{args:?}: {}", stderr(&output));
        stdout(&output)
    }
}

/// `report serve` on a free port, answering `requests` before it exits — and
/// a client for it: `ask(request line, extra headers, body)` sends one
/// request with this server's `Host`, and hands back the whole response.
fn serve(home: &Machine, requests: usize) -> (Child, impl Fn(&str, &str, &str) -> String) {
    let mut child = home
        .command(&[
            "report",
            "serve",
            "--bind",
            "127.0.0.1:0",
            "--requests",
            &requests.to_string(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run sandman");
    let mut line = String::new();
    BufReader::new(child.stdout.as_mut().expect("stdout"))
        .read_line(&mut line)
        .expect("the address");
    let address = line
        .trim()
        .strip_prefix("listening on http://")
        .expect(&line)
        .to_owned();
    let ask = move |request: &str, headers: &str, body: &str| {
        let mut stream = TcpStream::connect(&address).expect("connect");
        write!(
            stream,
            "{request} HTTP/1.1\r\n{headers}Host: {address}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .expect("send");
        stream.shutdown(Shutdown::Write).expect("half-close");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("response");
        response
    };
    (child, ask)
}

const AWAITING: &str = "## Proposals awaiting you\n- proposal:jgeschwendt/alpha:cache · untitled · opened 2026-10-12T10:00:00Z\n";

#[test]
fn the_digest_reads_unread_reports_newest_first_and_marks_them_read() {
    let empty = Machine::new("report-empty");
    assert_eq!(
        empty.out(&[], "", 0),
        "# Reports since the beginning\n\nnothing new.\n"
    );

    let home = Machine::reported(0);
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
    let cursor = home.home.join(".sandman/.reports-read");
    fs::remove_file(&cursor).expect("cursor");
    fs::create_dir(&cursor).expect("a directory in its place");
    assert_eq!(home.out(&["--all", "--mark-read"], "", 1), all);
}

#[test]
fn the_hook_shows_the_owner_unread_reports_within_the_cap_and_never_marks_them() {
    let home = Machine::reported(3_500);
    let root = home.home.join(".sandman").display().to_string();
    // The machine's own sessions: any under the data root, and the open
    // shift's — by session_id, or by its path before `attach` writes one.
    fs::create_dir(home.home.join("trunk")).expect("trunk");
    std::os::unix::fs::symlink(home.home.join("trunk"), home.home.join("link")).expect("link");
    for payload in [
        r#"{"session_id":"own","cwd":"/tmp"}"#.to_owned(),
        format!(r#"{{"session_id":"breath","cwd":"{root}/breath"}}"#),
        format!(
            r#"{{"session_id":"early","cwd":"{}/link"}}"#,
            home.home.display()
        ),
    ] {
        assert_eq!(home.out(&["--hook"], &payload, 0), "");
    }
    assert_eq!(home.cursor(), "");
    home.out(&["--hook"], "not json", 1);

    let owner = r#"{"session_id":"owner","cwd":"/nowhere"}"#;
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
    assert_eq!(home.cursor(), "", "only the page marks reports read");
    assert_eq!(home.out(&["--hook"], owner, 0), shown);
    home.out(&["--mark-read"], "", 0);
    assert_eq!(home.out(&["--hook"], owner, 0), "", "silent once read");
}

#[test]
fn serve_shows_a_night_and_answers_proposals_through_thread() {
    let home = Machine::reported(0);
    // Friday's night: 22:00 and, past midnight, 03:00 in New York.
    for (shift, cost, eye, landed) in [
        (
            "s-20261017T0200Z",
            "1.25",
            "- watch <script>alert(1)</script> and `<b>`",
            "| abc1234 | **parser** |",
        ),
        ("s-20261017T0700Z", "2.25", "- b1", "| def5678 | lexer |"),
    ] {
        let started = format!("2026-10-17T{}:00:00Z", &shift[11..13]);
        home.write(
            format!(".sandman/shifts/{shift}/meta.json"),
            &format!(
                r#"{{"cost_usd":{cost},"repo":"jgeschwendt/alpha","shift":"{shift}","started":"{started}","theme":"Ship"}}"#
            ),
        );
        home.write(
            format!(".sandman/shifts/{shift}/report.md"),
            &format!("# {shift} · Ship\n## Worth your eye\n{eye}\n## Landed\n| sha | what |\n| --- | --- |\n{landed}\n## Numbers\nnone.\n"),
        );
    }
    home.write(
        ".docket/p1.md",
        "---\nstate: open\nwaiting: user\nkey: proposal:jgeschwendt/alpha:cache\nopened: 2026-10-12T10:00:00Z\n---\n\n# Cache the parser\n\n## agent · 2026-10-12T10:00:00Z\n\n| option | shifts |\n| --- | --- |\n| **cache** | 2 |\n",
    );
    // revised and re-asked, it is back on the page with the exchange; revised
    // and not yet re-asked, it waits on the agent and is not
    let revised = "---\nstate: open\nwaiting: {waiting}\nkey: proposal:jgeschwendt/alpha:{slug}\nopened: 2026-10-12T10:00:00Z\nanswer: revise\ndecided: 2026-10-13T10:00:00Z\n---\n\n# Split {slug}\n\n## agent · 2026-10-12T10:00:00Z\n\nfirst matrix\n\n## user · 2026-10-13T10:00:00Z · answer: revise\n\nsmaller, please\n";
    for (id, waiting, slug, reply) in [
        (
            "p2",
            "user",
            "arena",
            "\n## agent · 2026-10-14T10:00:00Z\n\nsecond matrix\n",
        ),
        ("p3", "agent", "pool", ""),
    ] {
        home.write(
            format!(".docket/{id}.md"),
            &(revised
                .replace("{waiting}", waiting)
                .replace("{slug}", slug)
                + reply),
        );
    }
    let thread = home.script(
        "thread",
        "printf '%s\\n' \"$*\" >> \"$HOME/argv\"\n[ \"$6\" != revise ] || { echo 'thread: <refused>' >&2; exit 1; }\n",
    );

    // a shift that never started belongs to no night
    home.write(".sandman/shifts/s-20261016T0000Z/meta.json", "{}");
    let (mut server, ask) = serve(&home, 19);
    let tuesday = ask("GET /?night=2026-10-13", "", "");
    assert_eq!(home.cursor(), "", "an older night's page leaves the cursor");
    ask("GET /", "Sec-Fetch-Site: cross-site\r\n", "");
    assert_eq!(
        home.cursor(),
        "",
        "another site's request leaves the cursor"
    );
    let page = ask("GET /", "", "");
    assert_eq!(
        home.cursor(),
        "2026-10-17T07:00:00Z\n",
        "the newest night marks it"
    );
    for expected in [
        "HTTP/1.1 200 OK\r\n",
        "<p>Friday→Saturday night · 2 shifts · $3.50 · 2 landed · 2 proposals</p>",
        "<h3>Split arena</h3><p class=\"meta\">jgeschwendt/alpha · opened 2026-10-12T10:00:00Z</p><h2>agent · 2026-10-12T10:00:00Z</h2><p>first matrix</p><blockquote><h2>user · 2026-10-13T10:00:00Z · answer: revise</h2><p>smaller, please</p><h2>agent · 2026-10-14T10:00:00Z</h2><p>second matrix</p></blockquote><form",
        "<h3>Cache the parser</h3><p class=\"meta\">jgeschwendt/alpha · opened 2026-10-12T10:00:00Z</p><h2>agent · 2026-10-12T10:00:00Z</h2><table><thead><tr><th>option</th><th>shifts</th></tr></thead><tbody><tr><td><strong>cache</strong></td><td>2</td></tr></tbody></table>",
        "<input type=\"hidden\" name=\"key\" value=\"proposal:jgeschwendt/alpha:cache\">",
        "<button name=\"answer\" value=\"approve\">approve</button><button name=\"answer\" value=\"revise\">revise</button><button name=\"answer\" value=\"decline\">decline</button>",
        "<ul><li>b1 — <a href=\"#s-20261017T0700Z\">s-20261017T0700Z</a></li><li>watch &lt;script&gt;alert(1)&lt;/script&gt; and <code>&lt;b&gt;</code> — <a href=\"#s-20261017T0200Z\">",
        "<tr><th>shift</th><th>sha</th><th>what</th></tr></thead><tbody><tr><td><a href=\"#s-20261017T0700Z\">s-20261017T0700Z</a></td><td>def5678</td><td>lexer</td></tr><tr><td><a href=\"#s-20261017T0200Z\">s-20261017T0200Z</a></td><td>abc1234</td><td><strong>parser</strong></td></tr>",
        "<details id=\"s-20261017T0200Z\"><summary>s-20261017T0200Z · Ship · jgeschwendt/alpha · $1.25</summary><h1>s-20261017T0200Z · Ship</h1>",
        "<nav><a href=\"/?night=2026-10-16\">2026-10-16</a><a href=\"/?night=2026-10-15\">",
    ] {
        assert!(page.contains(expected), "{expected} missing: {page}");
    }
    assert!(
        !page.contains("<script") && !page.contains("Split pool"),
        "{page}"
    );
    assert!(
        tuesday
            .contains("<p>Tuesday→Wednesday night · 1 shifts · $0.00 · 0 landed · 2 proposals</p>")
            && tuesday.contains(
                "<h2>Worth your eye</h2><p>none.</p></section><section><h2>Landed</h2><p>none.</p>"
            ),
        "{tuesday}"
    );

    let answer = |fields: &str| {
        let response = ask(
            "POST /answer",
            "Content-Type: application/x-www-form-urlencoded\r\n",
            fields,
        );
        response[9..12].to_owned() + &response[response.find("\r\n\r\n").expect("a body")..]
    };
    let key = "key=proposal%3Ajgeschwendt%2Falpha%3Acache";
    assert_eq!(
        answer(&format!("{key}&answer=approve&note=ship+it%21%zz")),
        "303\r\n\r\n"
    );
    assert_eq!(
        answer(&format!("{key}&answer=decline&note=+")),
        "303\r\n\r\n"
    );
    assert!(
        answer(&format!("{key}&answer=revise")).ends_with("<pre>thread: &lt;refused&gt;\n</pre>\n")
    );
    assert_eq!(
        answer("key=proposal%3Ajgeschwendt%2Falpha%3Aarena&answer=approve"),
        "303\r\n\r\n",
        "a re-asked proposal takes a second answer"
    );
    assert!(answer("key=proposal%3Anone&answer=approve").starts_with("404"));
    assert!(answer(&format!("{key}&answer=maybe")).starts_with("400"));
    assert_eq!(
        fs::read_to_string(home.home.join("argv")).expect("argv"),
        "reply p1 --as user --answer approve --body ship it!%zz\nreply p1 --as user --answer decline\nreply p1 --as user --answer revise\nreply p2 --as user --answer approve\n"
    );
    let decisions = stdout(&feed(
        &mut home.command(&["trail", "log", "--kind", "decision", "--json"]),
        "",
    ));
    assert_eq!(decisions.lines().count(), 3, "{decisions}");
    for said in ["approved", "declined"] {
        assert!(
            decisions.contains(&format!(r#""kind":"decision","parent":null,"rationale":"owner {said} proposal proposal:jgeschwendt/alpha:cache","refs":["thread:p1"],"repo":"jgeschwendt/alpha""#)),
            "{decisions}"
        );
    }
    // A trail that cannot be appended to, then a thread binary that is gone.
    let trail = home.home.join(".sandman/trail");
    fs::remove_dir_all(&trail).expect("trail");
    fs::write(&trail, "").expect("a file in its place");
    assert!(answer(&format!("{key}&answer=approve")).starts_with("500"));
    fs::remove_file(&thread).expect("thread");
    assert!(answer(&format!("{key}&answer=approve")).starts_with("502"));

    for (request, headers, status) in [
        ("GET /", "Host: evil.example:1705\r\n", "403"),
        ("POST /answer", "Origin: http://evil.example\r\n", "403"),
        ("DELETE /", "", "405"),
        ("GET /nope", "", "404"),
        ("GET /?night=2026-01-01", "", "404"),
        ("GET /", "Content-Length: 99999999\r\n", "400"),
        ("", "", "400"),
        ("POST /answer", "Content-Length: 5\r\n", "400"),
    ] {
        let response = ask(request, headers, "");
        assert!(
            response.starts_with(&format!("HTTP/1.1 {status} ")),
            "{request}: {response}"
        );
    }
    assert!(server.wait().expect("exit").success());
}

#[test]
fn serve_binds_only_loopback_and_shows_an_empty_machine() {
    let empty = Machine::new("report-serve");
    let (mut server, ask) = serve(&empty, 1);
    let page = ask("GET /", "", "");
    assert!(
        page.contains(
            "<p>no nights yet</p></header><section><h2>Proposals awaiting you</h2><p>none.</p>"
        ),
        "{page}"
    );
    assert!(server.wait().expect("exit").success());
    let taken = TcpListener::bind("127.0.0.1:0").expect("a port");
    let taken = taken.local_addr().expect("address").to_string();
    for (args, code) in [
        (&["serve", "--bind", "0.0.0.0:1705"][..], 2),
        (&["serve", "--bind", "localhost"], 2),
        (&["serve", "--requests", "some"], 2),
        (&["serve", "--bogus"], 2),
        (&["serve", "--bind", &taken], 1),
    ] {
        empty.out(args, "", code);
    }
}

#[test]
fn serve_keeps_the_rotation_checklist() {
    let home = Machine::new("report-rotation");
    for slug in ["alpha", "beta", "gamma"] {
        fs::create_dir_all(
            home.grove_code()
                .join(format!("jgeschwendt/{slug}/main/.git")),
        )
        .expect("checkout");
    }
    home.write(
        ".sandman/shifts/s-20261005T1200Z/meta.json",
        r#"{"repo":"jgeschwendt/beta","started":"2026-10-05T12:00:00Z"}"#,
    );
    // Friday 2026-10-16 reads W42's targets, which freeze gamma.
    home.write(
        ".sandman/score/2026-W42-targets.md",
        "## jgeschwendt/gamma\nfrozen\n",
    );
    let file = home.write(
        ".sandman/shifts/rotation.toml",
        "off = [\"jgeschwendt/alpha\"]\n",
    );
    let (mut server, ask) = serve(&home, 4);
    let page = ask("GET /", "", "");
    assert!(
        page.contains(concat!(
            "<section><h2>Rotation</h2><form method=\"post\" action=\"/rotation\">",
            "<p><input type=\"checkbox\" id=\"rot-jgeschwendt_alpha\" name=\"on\" value=\"jgeschwendt/alpha\"><label for=\"rot-jgeschwendt_alpha\">jgeschwendt/alpha</label><span class=\"meta\">never shifted</span></p>",
            "<p><input type=\"checkbox\" id=\"rot-jgeschwendt_beta\" name=\"on\" value=\"jgeschwendt/beta\" checked><label for=\"rot-jgeschwendt_beta\">jgeschwendt/beta</label><span class=\"meta\">2026-10-05T12:00:00Z</span></p>",
            "<p><input type=\"checkbox\" id=\"rot-jgeschwendt_gamma\" name=\"on\" value=\"jgeschwendt/gamma\" checked><label for=\"rot-jgeschwendt_gamma\">jgeschwendt/gamma</label><span class=\"meta\">never shifted</span><span class=\"meta\">frozen</span></p>",
            "<button>Save</button></form></section><section><h2>Worth your eye</h2>"
        )),
        "{page}"
    );
    // Unchecked boxes send nothing: beta and gamma go off; a stranger is ignored.
    let saved = ask(
        "POST /rotation",
        "Content-Type: application/x-www-form-urlencoded\r\n",
        "on=jgeschwendt%2Falpha&on=someone%2Felse",
    );
    assert!(
        saved.starts_with("HTTP/1.1 303 See Other\r\nLocation: /\r\n"),
        "{saved}"
    );
    let off = r#"off = ["jgeschwendt/beta", "jgeschwendt/gamma"]"#;
    assert_eq!(
        fs::read_to_string(&file).expect("rotation"),
        format!("{off}\n")
    );
    let guards = stdout(&feed(
        &mut home.command(&["trail", "log", "--kind", "guard", "--json"]),
        "",
    ));
    assert!(
        guards.contains(&format!(
            r#""kind":"guard","parent":null,"rationale":"owner set the rotation: {}""#,
            off.replace('"', "\\\"")
        )),
        "{guards}"
    );
    // A rotation that will not read or write is the server's fault.
    fs::remove_file(&file).expect("rm");
    fs::create_dir(&file).expect("a directory in its place");
    for request in ["GET /", "POST /rotation"] {
        let response = ask(request, "", "");
        assert!(response.starts_with("HTTP/1.1 500 "), "{response}");
    }
    assert!(server.wait().expect("exit").success());
}

#[test]
fn serve_keeps_off_a_repo_whose_trunk_is_missing() {
    let home = Machine::new("report-rotation-gone");
    fs::create_dir_all(home.grove_code().join("jgeschwendt/alpha/main/.git")).expect("checkout");
    let file = home.write(
        ".sandman/shifts/rotation.toml",
        "off = [\"jgeschwendt/gone\"]\n",
    );
    let (mut server, ask) = serve(&home, 3);
    let saved = ask("POST /rotation", "", "");
    assert!(saved.starts_with("HTTP/1.1 303 "), "{saved}");
    assert_eq!(
        fs::read_to_string(&file).expect("rotation"),
        "off = [\"jgeschwendt/alpha\", \"jgeschwendt/gone\"]\n"
    );
    // listed unchecked, so checking it turns it back on
    home.write(
        ".sandman/shifts/s-20261005T1200Z/meta.json",
        r#"{"repo":"jgeschwendt/alpha","started":"2026-10-05T12:00:00Z"}"#,
    );
    let page = ask("GET /", "", "");
    assert!(page.contains("value=\"jgeschwendt/gone\">"), "{page}");
    let saved = ask("POST /rotation", "", "on=jgeschwendt%2Fgone");
    assert!(saved.starts_with("HTTP/1.1 303 "), "{saved}");
    assert_eq!(
        fs::read_to_string(&file).expect("rotation"),
        "off = [\"jgeschwendt/alpha\"]\n"
    );
    assert!(server.wait().expect("exit").success());
}
