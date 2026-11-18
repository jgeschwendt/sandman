//! End to end: the built binary over temp data roots. The `chart` fixture's output is
//! `docs/data/chart.json`, the sample the docs page draws; every daemon a test starts is killed
//! when its guard drops.

// the binary's dependencies, unused by the suite
use hotswap as _;
use signal_hook as _;

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use std::collections::BTreeSet;
    use std::error::Error;
    use std::fs::{File, Permissions};
    use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
    use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _, PermissionsExt as _, symlink};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::os::unix::process::CommandExt as _;
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio, id};
    use std::sync::OnceLock;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use std::{env, fs, iter, thread};

    /// Recall's corpus: candidates in two banks (one description past 160 characters, a tie on
    /// score broken by date, a remembered trace), and in a third a `user` memory, a pinned one and a
    /// `user` trace (the always-set) and a muted one (nothing); and intentions, never candidates:
    /// two past due, one waiting on an event, one upcoming, one done.
    const CORPUS: [(&str, &str); 15] = [
        (
            "intentions/0a1b2c3d.md",
            "---\ndue: 2026-11-03\ncreated: 2026-11-02T09:00:00Z\nabout: /work/alpha\nsession: @a\nstate: open\n---\n\nRerun the alpha bench on the new box\n",
        ),
        (
            "intentions/2a3b4c5d.md",
            "---\ndue: 2026-11-01\ncreated: 2026-10-30T09:00:00Z\nstate: done\nclosed: 2026-11-01T10:00:00Z\n---\n\nPin the node version\n",
        ),
        (
            "intentions/4e5f6a7b.md",
            "---\nafter: after grove ships\ncreated: 2026-11-04T09:00:00Z\nstate: open\n---\n\nCut sandman over to the rewrite\n",
        ),
        (
            "intentions/6e7f8a9b.md",
            "---\ndue: 2026-11-10\ncreated: 2026-11-06T09:00:00Z\nstate: open\n---\n\nDraft the beta release notes\n",
        ),
        (
            "intentions/8c9d0e1f.md",
            "---\ndue: 2099-01-01\ncreated: 2026-11-06T09:00:00Z\nstate: open\n---\n\nRenew the signing key\n",
        ),
        (
            "memories/-work-alpha/feedback_clock.md",
            "---\nname: Pass a clock in\ndescription: Tests take time from an injected clock, never a mocked one: a mock hides the drift a real clock shows within a day, and every fixture pins its dates \u{2014} na\u{ef}ve caf\u{e9} clocks lose\ntype: feedback\ncreated: 2026-11-02T00:00:00Z\n---\n\nInject the clock.\n",
        ),
        (
            "memories/-work-alpha/project_vpn.md",
            "---\nname: Alpha deploys need the vpn\ndescription: the deploy host is private\ntype: project\ncreated: 2026-11-01T00:00:00Z\nupdated: 2026-11-05T00:00:00Z\n---\n\nConnect first.\n",
        ),
        (
            "memories/-work-alpha/reference_grammar.md",
            "---\nname: The grammar lives in one file\ndescription: src/grammar.pest\ntype: reference\ncreated: 2026-11-01T00:00:00Z\n---\n\nOne file.\n",
        ),
        (
            "memories/-work-beta/feedback_tie.md",
            "---\nname: Beta ties break by date\ndescription: newer first\ntype: feedback\ncreated: 2026-11-04T00:00:00Z\n---\n\nNewer first.\n",
        ),
        (
            "memories/-work-beta/project_node.md",
            "---\nname: Beta pins its node version\ndescription: node 22 from mise\ntype: project\ncreated: 2026-11-01T00:00:00Z\n---\n\nmise use node 22.\n",
        ),
        (
            "memories/common/feedback_muted.md",
            "---\nname: Muted\ndescription: never a candidate\ntype: feedback\nrecall: mute\n---\n\nMuted.\n",
        ),
        (
            "memories/common/project_pinned.md",
            "---\nname: Pinned\ndescription: always sent\ntype: project\nrecall: pin\n---\n\nPinned.\n",
        ),
        (
            "memories/common/user_short.md",
            "---\nname: Prefers short replies\ndescription: lead with the result\ntype: user\n---\n\nShort.\n",
        ),
        (
            "traces/-work-beta/feedback_retry.md",
            "---\nname: Beta retries back off twice\ndescription: a failed beta deploy retries twice, then stops\ntype: feedback\ncreated: 2026-11-06T00:00:00Z\nsession: @a\nsource: remember 2026-11-06T00:00:00Z \u{b7} @a\n---\n\nTwice, then stop.\n",
        ),
        (
            "traces/common/user_terse.md",
            "---\nname: Writes terse commits\ndescription: one line, present tense\ntype: user\ncreated: 2026-11-06T00:00:00Z\nsource: remember 2026-11-06T00:00:00Z \u{b7} -\n---\n\nTerse.\n",
        ),
    ];
    /// Session ids for the fixture: `5a000000-0000-4000-8000-00000000000<letter>`.
    const ID: &str = "5a000000-0000-4000-8000-00000000000";
    /// The binary's release (a checkout's: the crate version) and build, as `version` prints it.
    const VERSION: &str = concat!(env!("SANDMAN_RELEASE"), "-", env!("SANDMAN_BUILD"));
    /// A test's fallible outcome.
    type Outcome = Result<(), Box<dyn Error>>;

    /// A running `sandman serve`, killed and reaped when it drops.
    struct Daemon(Child);

    impl Drop for Daemon {
        fn drop(&mut self) {
            drop(self.0.kill());
            drop(self.0.wait());
        }
    }

    /// `kill <signal> <pid>`: whether the signal was delivered.
    fn kill(signal: &str, pid: &str) -> io::Result<bool> {
        Ok(Command::new("kill")
            .args([signal, pid])
            .stderr(Stdio::null())
            .status()?
            .success())
    }

    /// `request` to the daemon over `root`, and its whole reply.
    fn ask(root: &str, request: &str) -> io::Result<String> {
        let mut stream = UnixStream::connect(format!("{root}/run/serve.sock"))?;
        stream.write_all(request.as_bytes())?;
        let mut reply = String::new();
        stream.read_to_string(&mut reply).map(|_| reply)
    }

    /// `sandman args` with `envs` in place of `HOME` and the `SANDMAN_*` vars; every transport is
    /// curl, and the first on `PATH` refuses (exit 7) unless a test names its own (`stub`).
    fn command(envs: &[(&str, &str)], args: &[&str]) -> Command {
        static REFUSE: OnceLock<String> = OnceLock::new();
        let bin = REFUSE.get_or_init(|| {
            let dir = env::temp_dir().join(format!("sandman-refuse-{}", id()));
            drop(fs::create_dir_all(&dir));
            drop(fs::write(dir.join("curl"), "#!/bin/sh\nexit 7\n"));
            drop(fs::set_permissions(
                dir.join("curl"),
                Permissions::from_mode(0o755),
            ));
            format!("{}:{}", dir.display(), env::var("PATH").unwrap_or_default())
        });
        let mut command = Command::new(env!("CARGO_BIN_EXE_sandman"));
        let _: &mut Command = command
            .args(args)
            // not env_clear: llvm-cov reaches the child through LLVM_PROFILE_FILE
            .env_remove("CLAUDE_CODE_OAUTH_TOKEN")
            .env_remove("CLAUDE_CODE_SESSION_ID")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CLAUDE_MEMORY_PIPELINE")
            .env_remove("HOME")
            .env_remove("SANDMAN_CLAUDE_TOKEN_FILE")
            .env_remove("SANDMAN_MACHINE")
            .env_remove("SANDMAN_NO_TAKE")
            .env_remove("SANDMAN_ROOT")
            .env_remove("TYPESAFE_API_KEY")
            .env("PATH", bin)
            .env("SANDMAN_OLLAMA", "http://ollama.test")
            .envs(envs.iter().copied());
        command
    }

    /// `date -u` with `args`, trimmed; a failed `date` is an error, never an empty stamp.
    fn date(args: &[&str]) -> Result<String, Box<dyn Error>> {
        let out = Command::new("date").arg("-u").args(args).output()?;
        out.status.success().then_some(()).ok_or("date failed")?;
        Ok(String::from_utf8(out.stdout)?.trim().to_owned())
    }

    /// Writes each `(path, text)` under `dir`, `@` in either standing for the fixture's id prefix.
    fn plant(dir: &str, files: &[(&str, &str)]) -> Outcome {
        for &(path, text) in files {
            let file = PathBuf::from(dir).join(path.replace('@', ID));
            fs::create_dir_all(file.parent().ok_or("parent")?)?;
            fs::write(file, text.replace('@', ID))?;
        }
        Ok(())
    }

    /// Every line of `root`'s ledger, `at` as `A` and `ms` as 0, newline-joined.
    fn ledger(root: &str) -> Result<String, Box<dyn Error>> {
        let mut lines = Vec::new();
        for day in fs::read_dir(format!("{root}/ledger"))?.flatten() {
            for month in fs::read_dir(day.path())?.flatten() {
                for file in fs::read_dir(month.path())?.flatten() {
                    for line in fs::read_to_string(file.path())?.lines() {
                        let mut entry: Value = serde_json::from_str(line)?;
                        entry["at"] = "A".into();
                        if entry.get("ms").is_some() {
                            entry["ms"] = 0u8.into();
                        }
                        lines.push(entry.to_string());
                    }
                }
            }
        }
        Ok(lines.join("\n"))
    }

    /// `text` with each `id: <12 lowercase hex>` line's id as `<id>`: a minted id, checked for its
    /// shape; any other stays, to fail the comparison.
    fn masked(text: &str) -> String {
        text.split_inclusive('\n')
            .map(|line| {
                let id = line.strip_prefix("id: ").map_or("", str::trim_end);
                if id.len() == 12
                    && id
                        .bytes()
                        .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
                {
                    "id: <id>\n"
                } else {
                    line
                }
            })
            .collect()
    }

    /// `Err` showing both sides unless `got == want`.
    fn same(got: &str, want: &str) -> Outcome {
        if got == want {
            Ok(())
        } else {
            Err(format!("got:\n{got}\nwant:\n{want}").into())
        }
    }

    /// `command(envs, args)`, `stdin` piped in: its exit code, a newline, its stdout, then its
    /// stderr.
    fn sandman(
        envs: &[(&str, &str)],
        args: &[&str],
        stdin: &str,
    ) -> Result<String, Box<dyn Error>> {
        ran(&mut command(envs, args), stdin)
    }

    /// `command` run, `stdin` piped in: its exit code, a newline, its stdout, then its stderr.
    fn ran(command: &mut Command, stdin: &str) -> Result<String, Box<dyn Error>> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .ok_or("stdin")?
            .write_all(stdin.as_bytes())?;
        let out = child.wait_with_output()?;
        Ok(format!(
            "{}\n{}{}",
            out.status.code().unwrap_or_default(),
            String::from_utf8(out.stdout)?,
            String::from_utf8(out.stderr)?
        ))
    }

    /// `sandman serve` over `root` with `envs` beside it (a model command never a real one, no
    /// weekly review unless a test lowers its evidence floor), and the health line it prints once
    /// ready.
    fn serve(root: &str, envs: &[(&str, &str)]) -> Result<(Daemon, String), Box<dyn Error>> {
        let claude = format!("{root}/claude");
        let mut daemon = Daemon(
            command(
                &[
                    ("CLAUDE_CONFIG_DIR", claude.as_str()),
                    ("SANDMAN_CONSOLIDATOR", "exit 9"),
                    ("SANDMAN_ENCODER", "exit 9"),
                    ("SANDMAN_MACHINE", "studio"),
                    ("SANDMAN_REFLECT_MIN", "1000"),
                    ("SANDMAN_REFLECTOR", "exit 9"),
                    ("SANDMAN_ROOT", root),
                ],
                &["serve"],
            )
            .envs(envs.iter().copied())
            .stdout(Stdio::piped())
            .spawn()?,
        );
        let mut ready = String::new();
        let _: usize =
            BufReader::new(daemon.0.stdout.take().ok_or("stdout")?).read_line(&mut ready)?;
        Ok((daemon, ready))
    }

    /// A stub `curl` in `<root>/bin`, and `PATH` with it first: each request appended to
    /// `<root>/<name>.in` (`embed`, `pplx`, `kev` or `jev` by its URL), its argv to `<name>.argv`,
    /// the key variable to `<name>.env` and Jev's config to `jev.config`; `<root>/<name>.fail`,
    /// when present, is sourced first; an embedding is `[clock, deploy, beta, 1]`, each 1 when the
    /// text holds the word, a decision `<root>/answers`.
    fn stub(root: &str) -> Result<String, Box<dyn Error>> {
        let bin = format!("{root}/bin");
        fs::create_dir_all(&bin)?;
        fs::write(
            format!("{bin}/curl"),
            format!(
                r#"#!/bin/sh
for arg; do url=$arg; done
case $url in */api/embed) n=embed;; *1703*) n=kev;; *typesafe*) n=jev;; *) n=pplx;; esac
printf '%s\n' "$@" > {root}/$n.argv
printf '[%s]' "$SANDMAN_SYSTEMONE_KEY" > {root}/$n.env
case "$*" in *-K*) cat > {root}/jev.config; body=$(cat <&3);; *) body=$(cat);; esac
printf '%s\n' "$body" >> {root}/$n.in
[ -f {root}/$n.fail ] && . {root}/$n.fail
[ $n = embed ] || exec cat {root}/answers
printf '%s\n' "$body" | awk '{{ s = $0; sub(/^\{{"input":\["/, "", s); sub(/"\],"model".*$/, "", s); n = split(s, t, /","/); printf "{{\"embeddings\":["; for (i = 1; i <= n; i++) printf "%s[%d,%d,%d,1]", (i > 1 ? "," : ""), (index(t[i], "clock") > 0), (index(t[i], "deploy") > 0), (index(t[i], "beta") > 0); print "]}}" }}'
"#
            ),
        )?;
        fs::set_permissions(format!("{bin}/curl"), Permissions::from_mode(0o755))?;
        Ok(format!("{bin}:{}", env::var("PATH")?))
    }

    /// A fresh temp dir named for the test, as a string for `SANDMAN_ROOT`.
    fn temp(name: &str) -> Result<String, Box<dyn Error>> {
        let dir = env::temp_dir().join(format!("sandman-{name}-{}", id()));
        drop(fs::remove_dir_all(&dir));
        fs::create_dir_all(&dir)?;
        Ok(dir.to_str().ok_or("temp path")?.to_owned())
    }

    /// `check`, polled every 10 ms for up to 10 s.
    fn until<F: Fn() -> bool>(check: F) -> Outcome {
        let polled = (0u16..1000).any(|_| {
            check() || {
                thread::sleep(Duration::from_millis(10));
                false
            }
        });
        Ok(polled.then_some(()).ok_or("timed out")?)
    }

    /// `chart` over a fixture of every file kind it reads, and every one it skips, prints
    /// `docs/data/chart.json`.
    #[test]
    fn chart() -> Outcome {
        let root = temp("chart")?;
        let mark = |at: &str, kind: &str, letter: &str, cwd: &str, machine: &str| {
            format!(
                r#"{{"at":"{at}","cwd":"{cwd}","interactive":true,"kind":"{kind}","machine":"{machine}","reason":null,"resume":null,"session":"{ID}{letter}","surface":"cli"}}"#
            )
        };
        let memory = |name: &str, kind: &str, created: &str, source: &str| {
            format!(
                "---\nname: {name}\ndescription: sample\ntype: {kind}\ncreated: {created}\nsource: {source}\n---\n\nsample body\n"
            )
        };
        let pointer = |title: &str, cwd: &str, highlights: &str, dreamed: &str| {
            format!(
                r#"{{"archived":"sample","cwd":"{cwd}","ended":"sample","title":"{title}","highlights":[{highlights}]{dreamed}}}"#
            )
        };
        // an end line with its outcome; one without reads as null
        let ended = |line: String, outcome: &str| {
            line.replacen(
                ",\"kind\"",
                &format!(",\"outcome\":\"{outcome}\",\"kind\""),
                1,
            )
        };
        let marks = [
            mark(
                "2026-11-09T08:05:00Z",
                "start",
                "a",
                "/work/alpha",
                "studio",
            ),
            // interactive: a's true outlasts its reload's false, c's is false, e says nothing
            mark(
                "2026-11-09T08:40:00Z",
                "start",
                "a",
                "/work/reload",
                "studio",
            )
            .replace("true", "false"),
            ended(
                mark("2026-11-09T09:30:00Z", "end", "a", "/work/alpha", "studio"),
                "taken",
            ),
            mark("2026-11-09T10:15:00Z", "start", "b", "/work/beta", "studio"),
            ended(
                mark("2026-11-09T12:40:00Z", "end", "b", "/work/beta", "studio"),
                "held",
            ),
            mark(
                "2026-11-09T14:00:00Z",
                "start",
                "c",
                "/work/alpha",
                "laptop",
            )
            .replace("true", "false"),
            mark("2026-11-09T15:10:00Z", "end", "c", "/work/alpha", "laptop"),
            format!(
                r#"{{"at":"2026-11-09T09:31:00Z","error":"sample","kind":"encode","machine":"studio","outcome":"failed","session":"{ID}a","traces":0}}"#
            ),
            format!(
                r#"{{"at":"2026-11-09T09:40:00Z","kind":"encode","machine":"studio","outcome":"encoded","session":"{ID}a","traces":1}}"#
            ),
            "not json".to_owned(),
            r#"{"kind":"start"}"#.to_owned(),
            // a denied tool call: a guard, never a session's mark
            format!(
                r#"{{"at":"2026-11-09T09:10:00Z","kind":"guard","machine":"studio","rationale":"sample: never force-push","session":"{ID}a","tool":"Bash"}}"#
            ),
        ];
        let files = [
            ("ledger/2026/11/09.jsonl", marks.join("\n")),
            (
                "ledger/2026/11/10.jsonl",
                [
                    ended(
                        mark("2026-11-10T06:00:00Z", "end", "e", "/work/gamma", "laptop"),
                        "gone",
                    ),
                    mark("2026-11-10T07:45:00Z", "start", "d", "/work/beta", "studio"),
                    format!(
                        r#"{{"at":"2026-11-10T07:50:00Z","kind":"guard","machine":"studio","rationale":"sample: ask before deleting","session":"{ID}g","tool":null}}"#
                    ),
                ]
                .join("\n"),
            ),
            (
                ".archive/claude/2026/11/09/093005-projects--work-alpha-@a.jsonl",
                "\n".repeat(4800),
            ),
            (
                ".archive/claude/2026/11/09/120000-projects--work-beta-@b.jsonl",
                "\n".repeat(900),
            ),
            (
                ".archive/claude/2026/11/09/124002-projects--work-beta-@b.jsonl",
                "\n".repeat(2600),
            ),
            (
                ".archive/claude/2026/11/09/151004-projects--work-alpha-@c.jsonl",
                "\n".repeat(1500),
            ),
            (
                ".archive/claude/2026/11/09/151004-projects--work-alpha-@c-dir/a.jsonl",
                String::new(),
            ),
            (
                ".archive/claude/2026/11/09/12ab56-projects--work-@f.jsonl",
                String::new(),
            ),
            (".archive/claude/2026/11/09/151004-@f.jsonl", String::new()),
            (".archive/claude/2026/11/09/notes.jsonl", String::new()),
            (
                ".archive/claude/2026/11/10/060001-projects--work-gamma-@e.jsonl",
                "\n".repeat(700),
            ),
            (
                "memories/.recent/@a.json",
                pointer(
                    "sample: tighten the parser",
                    "/work/alpha",
                    "\"x\",\"y\",\"z\"",
                    r#","dreamed":"2026-11-09T22:00:00Z""#,
                ),
            ),
            (
                "memories/.recent/@b.json",
                pointer("sample: draft the release notes", "/work/beta", "\"x\"", ""),
            ),
            ("memories/.recent/@c.json", "[]".to_owned()),
            (
                "memories/.recent/@e.json",
                pointer(
                    "sample: green the gamma suite",
                    "/work/gamma",
                    "",
                    r#","dreamed":"2026-11-10T09:00:00Z""#,
                ),
            ),
            (
                "memories/-work-alpha/feedback_parser_spans.md",
                memory(
                    "Parser errors carry spans",
                    "feedback",
                    "2026-11-09T22:00:00Z",
                    &format!("dream 2026-11-09T22:00:00Z \u{b7} {ID}a \u{b7} minds=opus"),
                ),
            ),
            ("memories/-work-alpha/MEMORY.md", String::new()),
            ("memories/-work-alpha/_draft.md", String::new()),
            ("memories/-work-alpha/notes.txt", String::new()),
            (
                "memories/-work-alpha/reference_grammar.md",
                // remembered without a session: its origin its source
                memory(
                    "The grammar lives in one file",
                    "reference",
                    "2026-11-09T08:40:00Z",
                    "remember 2026-11-09T08:40:00Z",
                )
                .replacen("source:", "origin: hand\nsource:", 1),
            ),
            (
                "memories/-work-beta/loose.md",
                "no frontmatter\n".to_owned(),
            ),
            (
                "memories/-work-beta/project_release.md",
                memory(
                    "Release notes are cut weekly",
                    "project",
                    "2026-11-10T03:00:00Z",
                    "reflect 2026-11-10T03:00:00Z",
                ),
            ),
            (
                "memories/-work-gamma/feedback_clean_tree.md",
                memory(
                    "The gamma suite needs a clean tree",
                    "feedback",
                    "2026-11-10T09:00:00Z",
                    &format!("dream 2026-11-10T09:00:00Z \u{b7} {ID}e \u{b7} minds=opus"),
                ),
            ),
            ("memories/_archive/old.md", String::new()),
            ("memories/TOOLS.md", String::new()),
            (
                "knowledge/launchd/playbook_swap_a_daemon.md",
                memory(
                    "Swap a launchd daemon",
                    "playbook",
                    "2026-11-09T09:45:00Z",
                    &format!("consolidate 2026-11-10T04:00:00Z \u{b7} {ID}a \u{b7} model=opus"),
                ),
            ),
            // provenance past the first stamp: a topic stamp with no session, a backfilled line
            (
                "knowledge/launchd/knowledge_labels.md",
                format!(
                    "---\nid: 3f9a0c1d2b4e\nname: Labels name the project\ntype: knowledge\ncreated: 2026-11-10T05:00:00Z\nsession: {ID}e\nfrom: 9e8d7c6b5a4f 0a1b2c3d4e5f\nsessions: {ID}c {ID}b sample {ID}c\nsource: consolidate 2026-11-10T05:00:00Z \u{b7}  \u{b7} model=opus\n---\n\nsample body\n"
                ),
            ),
            ("knowledge/launchd/MEMORY.md", String::new()),
            (
                "traces/-work-alpha/feedback_sample_spans.md",
                format!(
                    "---\nid: 6c5d4e3f2a1b\nname: Sample: spans\ntype: feedback\ncreated: 2026-11-09T09:40:00Z\nsession: {ID}a\nsource: encode 2026-11-09T09:40:00Z \u{b7} {ID}a \u{b7} model=opus\n---\n\nsample body\n"
                ),
            ),
            (
                "traces/-work-alpha/.feedback_sample_spans.md.1",
                String::new(),
            ),
            // d's live transcript: its `seen`
            ("claude/projects/-work-beta/@d.jsonl", String::new()),
            // intentions: one due, one waiting on an event, one done; a stray file skipped
            (
                "intentions/1f2e3d4c.md",
                format!("---\ndue: 2026-11-12\ncreated: 2026-11-09T09:20:00Z\nabout: /work/alpha\nsession: {ID}a\nstate: open\n---\n\nsample: rerun the alpha bench\n"),
            ),
            (
                "intentions/5b6a7980.md",
                "---\ndue: 2026-11-10\ncreated: 2026-11-09T11:00:00Z\nstate: done\nclosed: 2026-11-10T08:00:00Z\nnote: sample: shipped\n---\n\nsample: draft the beta notes\n".to_owned(),
            ),
            (
                "intentions/9c8b7a6d.md",
                format!("---\nafter: after the beta ships\ncreated: 2026-11-09T12:30:00Z\nsession: {ID}b\nstate: open\n---\n\nsample: cut the release\n"),
            ),
            ("intentions/notes.txt", String::new()),
        ];
        for (path, text) in files {
            let file = PathBuf::from(&root).join(path.replace('@', ID));
            fs::create_dir_all(file.parent().ok_or("parent")?)?;
            fs::write(file, text)?;
        }
        File::open(format!("{root}/claude/projects/-work-beta/{ID}d.jsonl"))?
            .set_modified(UNIX_EPOCH + Duration::from_secs(1_794_299_400))?;
        let want: Value = serde_json::from_str(include_str!("../docs/data/chart.json"))?;
        let out = sandman(
            &[
                ("CLAUDE_CONFIG_DIR", &format!("{root}/claude")),
                ("SANDMAN_MACHINE", "studio"),
                ("SANDMAN_ROOT", &root),
            ],
            &["chart"],
            "",
        )?;
        let mut got: Value =
            serde_json::from_str(out.strip_prefix("0\n").ok_or_else(|| out.clone())?)?;
        got["at"] = want["at"].clone();
        same(&got.to_string(), &want.to_string())
    }

    /// `check` reports each note's violations of invariants 1–3 in reader order and writes
    /// nothing, not even a machine id; `--fix` mints ids (missing, malformed or taken), drops empty
    /// and `-` source keys and an unknown origin, sorts and dedupes `from:` and `sessions:`, gives a
    /// sourceless note its origin by stamp (frontmatter for a file without it), writes a `check`
    /// line and reports what is left; a clean store says so, exit 0; a failed write is exit 1.
    #[test]
    fn check() -> Outcome {
        let root = temp("check")?;
        let notes = [
            (
                "memories/alpha/first.md",
                "---\nid: 0a0a0a0a0a0a\nname: First\norigin: routine\nsource: encode\n---\n\nbody\n",
            ),
            (
                "memories/alpha/noid.md",
                "---\nname: No id\nsource: remember 2026-11-01T00:00:00Z \u{b7} -\n---\n\nbody\n",
            ),
            (
                "memories/alpha/second.md",
                "---\nid: 0a0a0a0a0a0a\nname: Second\nsession: @b\n---\n\nbody\n",
            ),
            ("memories/alpha/MEMORY.md", "never read\n"),
            ("memories/beta/loose.md", "no frontmatter\n"),
            (
                "knowledge/topic/bad.md",
                "---\nid: XYZ\nname: Bad\norigin: manual\nfrom: 1b1b1b1b1b1b nope 0a0a0a0a0a0a 0a0a0a0a0a0a\nsource: consolidate\n---\n\nbody\n",
            ),
            (
                "traces/b/empty.md",
                "---\nid: 2b2b2b2b2b2b\nfrom:\norigin: -\nsessions:\nstrength: 3\nsource: dream 2026-01-01\n---\n\nbody\n",
            ),
            (
                "traces/b/odd.md",
                "---\nid: 3c3c3c3c3c3c\nsessions: @c @b notuuid @b\nsource: encode\n---\n\nbody\n",
            ),
        ];
        plant(&root, &notes)?;
        let envs = [("SANDMAN_ROOT", root.as_str())];
        let read = |path: &str| fs::read_to_string(format!("{root}/{path}")).unwrap_or_default();
        let all = || notes.map(|note| masked(&read(note.0))).concat();
        let before = all();
        let left = [
            "knowledge/topic/bad.md: 3: from: nope is not an id",
            "traces/b/empty.md: 3: strength is counted, never stored",
            "traces/b/odd.md: 3: sessions: notuuid is not a session id",
        ];
        let bad = [
            "alpha/noid.md: 1: no id",
            "alpha/noid.md: 2: no source",
            "alpha/second.md: 1: id 0a0a0a0a0a0a is also alpha/first.md's",
            "beta/loose.md: 1: no id",
            "beta/loose.md: 2: no source",
            "knowledge/topic/bad.md: 1: id XYZ is not 12 lowercase hex",
            "knowledge/topic/bad.md: 2: origin manual is not hand, import or routine",
            left[0],
            "knowledge/topic/bad.md: 3: from is not sorted once",
            "traces/b/empty.md: 2: from is \"\"",
            "traces/b/empty.md: 2: origin is \"-\"",
            "traces/b/empty.md: 2: sessions is \"\"",
            "traces/b/empty.md: 2: no source",
            left[1],
            left[2],
            "traces/b/odd.md: 3: sessions is not sorted once",
        ];
        same(
            &format!(
                "{}{}{}",
                sandman(&envs, &["check"], "")?,
                fs::exists(format!("{root}/machine"))?,
                all() == before
            ),
            &format!("1\n{}\nfalsetrue", bad.join("\n")),
        )?;
        let fixed = sandman(
            &[&envs[..], &[("SANDMAN_MACHINE", "studio")]].concat(),
            &["check", "--fix"],
            "",
        )?;
        same(
            &format!("{fixed}{}\n{}", all().replace('@', ID), ledger(&root)?),
            &format!(
                "1\ncheck: fixed 6\n{}\n{}\n{}",
                left.join("\n"),
                [
                    "---\nid: <id>\nname: First\norigin: routine\nsource: encode\n---\n\nbody\n",
                    "---\nid: <id>\nname: No id\norigin: hand\nsource: remember 2026-11-01T00:00:00Z \u{b7} -\n---\n\nbody\n",
                    "---\nid: <id>\nname: Second\nsession: @b\n---\n\nbody\n",
                    "never read\n",
                    "---\nid: <id>\norigin: import\n---\n\nno frontmatter\n",
                    "---\nid: <id>\nname: Bad\nfrom: 0a0a0a0a0a0a 1b1b1b1b1b1b nope\nsource: consolidate\n---\n\nbody\n",
                    "---\nid: <id>\nstrength: 3\norigin: routine\nsource: dream 2026-01-01\n---\n\nbody\n",
                    "---\nid: <id>\nsessions: @b @c notuuid\nsource: encode\n---\n\nbody\n",
                ]
                .concat()
                .replace('@', ID),
                r#"{"at":"A","fixed":6,"kind":"check","machine":"studio"}"#
            ),
        )?;
        // a valid id kept, each minted one new
        let ids: BTreeSet<String> = notes
            .iter()
            .filter(|note| !note.0.ends_with("MEMORY.md"))
            .map(|note| read(note.0).lines().nth(1).unwrap_or_default().to_owned())
            .collect();
        same(
            &format!(
                "{} {}",
                ids.len(),
                ["0a0a0a0a0a0a", "2b2b2b2b2b2b", "3c3c3c3c3c3c"]
                    .iter()
                    .all(|id| ids.contains(&format!("id: {id}")))
            ),
            "7 true",
        )?;
        for path in [
            "knowledge/topic/bad.md",
            "traces/b/empty.md",
            "traces/b/odd.md",
        ] {
            fs::remove_file(format!("{root}/{path}"))?;
        }
        same(
            &sandman(&envs, &["check"], "")?,
            "0\ncheck: 4 notes: clean\n",
        )?;
        let blocked = format!("{root}/blocked");
        plant(&blocked, &[("memories/locked/x.md", "no id\n")])?;
        fs::set_permissions(
            format!("{blocked}/memories/locked"),
            Permissions::from_mode(0o555),
        )?;
        let failed = sandman(
            &[("SANDMAN_ROOT", blocked.as_str())],
            &["check", "--fix"],
            "",
        );
        fs::set_permissions(
            format!("{blocked}/memories/locked"),
            Permissions::from_mode(0o755),
        )?;
        same(
            &failed?,
            "1\nsandman check: Permission denied (os error 13)\n",
        )
    }

    /// Prospective memory against a fixed local today, Saturday 2026-11-14: `defer` resolves each
    /// time form to a date and keeps any other words as an event, a malformed date's too; a
    /// missing what or when is refused, a failed write is exit 1. `due` lists the due ones most
    /// overdue first, then those waiting on an event, `--all` the upcoming after them by date;
    /// `done`, `drop` and a push rewrite one in place, an unknown or ambiguous id refused; each a
    /// ledger line. The seed lists ten and counts the rest. Without the fixed day, `today` is the
    /// date in `$TZ`.
    #[test]
    fn defer() -> Outcome {
        let root = fs::canonicalize(temp("defer")?)?
            .to_str()
            .ok_or("root")?
            .to_owned();
        let (session, work, odd) = (
            format!("{ID}a"),
            format!("{root}/work"),
            format!("{root}/odd"),
        );
        fs::create_dir_all(&work)?;
        let envs = [
            ("HOME", root.as_str()),
            ("SANDMAN_MACHINE", "studio"),
            ("SANDMAN_ROOT", &root),
            ("SANDMAN_TODAY", "2026-11-14"),
        ];
        let (mut ids, mut got) = (Vec::new(), String::new());
        for when in [
            "today",
            "Tomorrow",
            "saturday",
            "monday",
            "2026-12-01",
            "+3d",
            "+2w",
            "2026-11-02",
            " after grove ships ",
        ] {
            let what = format!("sample: {}", when.trim());
            let mut args = vec!["defer", &what, "--when", when, "--session", &session];
            if when == "monday" {
                args.extend(["--about", &work]);
            }
            let out = sandman(&envs, &args, "")?;
            let id = out.get(2..10).unwrap_or_default().to_owned();
            got.push_str(&out.replace(&id, "<id>"));
            ids.push(id);
        }
        same(
            &got,
            &[
                "2026-11-14",
                "2026-11-15",
                "2026-11-21",
                "2026-11-16",
                "2026-12-01",
                "2026-11-17",
                "2026-11-28",
                "2026-11-02",
                "waiting: after grove ships",
            ]
            .map(|shown| format!("0\n<id> {shown}\n"))
            .concat(),
        )?;
        let refused = "2\nsandman defer: takes \"<what>\" first, or --id <id>, and --when today|tomorrow|<weekday>|YYYY-MM-DD|+<n>d|+<n>w|<event>\n";
        for args in [
            &["defer", "x"][..],
            &["defer", "x", "--when", " "],
            &["defer", "--when", "today"],
            &["defer", "--id", "x"],
        ] {
            same(&sandman(&envs, args, "")?, refused)?;
        }
        let row = |at: usize, when: &str, about: &str| {
            let what = [
                "today",
                "Tomorrow",
                "saturday",
                "monday",
                "2026-12-01",
                "+3d",
                "+2w",
                "2026-11-02",
                "after grove ships",
            ][at];
            let id = ids.get(at).map_or("", String::as_str);
            format!("  \u{b7} {id} {when} \u{2014} sample: {what}{about}\n")
        };
        let header = "due \u{b7} deferred work: raise it with the person, never start it unasked\n";
        let due = [
            row(7, "2026-11-02", ""),
            row(0, "2026-11-14", ""),
            row(8, "waiting: after grove ships", ""),
        ]
        .concat();
        same(&sandman(&envs, &["due"], "")?, &format!("0\n{header}{due}"))?;
        same(
            &sandman(&envs, &["due", "--all"], "")?,
            &format!(
                "0\n{header}{due}{}",
                [
                    row(1, "2026-11-15", ""),
                    row(3, "2026-11-16", " (~/work)"),
                    row(5, "2026-11-17", ""),
                    row(2, "2026-11-21", ""),
                    row(6, "2026-11-28", ""),
                    row(4, "2026-12-01", ""),
                ]
                .concat()
            ),
        )?;
        // closed by its whole id or a unique start; pushed to a new when, an event to a date
        plant(
            &root,
            &[
                ("intentions/abc00001.md", ""),
                ("intentions/abc00002.md", ""),
            ],
        )?;
        let id = |at: usize| ids.get(at).map_or("", String::as_str);
        let mut closes = String::new();
        for args in [
            &[
                "done",
                id(7),
                "--note",
                "sample: ran it",
                "--session",
                &session,
            ][..],
            &["drop", id(0).get(..7).unwrap_or_default()],
            &["defer", "--id", id(8), "--when", "+5d"],
            &["done", "zz"],
            &["drop", "abc"],
            &["done"],
        ] {
            closes.push_str(&sandman(&envs, args, "")?);
        }
        same(
            &closes,
            &format!(
                "0\n{} done\n0\n{} dropped\n0\n{} 2026-11-19\n1\nsandman done: zz is not one intention's id (0 match)\n1\nsandman drop: abc is not one intention's id (2 match)\n1\nsandman done:  is not one intention's id (0 match)\n",
                id(7),
                id(0),
                id(8)
            ),
        )?;
        same(&sandman(&envs, &["due"], "")?, "0\nnothing due\n")?;
        let mut files = String::new();
        for at in [7, 8, 3] {
            let whole = fs::read_to_string(format!("{root}/intentions/{}.md", id(at)))?;
            for line in whole.lines() {
                let stamped = ["created: ", "closed: "]
                    .iter()
                    .find(|key| line.starts_with(**key));
                files.push_str(
                    &stamped.map_or_else(|| line.to_owned(), |key| format!("{key}<now>")),
                );
                files.push('\n');
            }
        }
        same(
            &files,
            &format!(
                "---\ndue: 2026-11-02\ncreated: <now>\nsession: {session}\nstate: done\nclosed: <now>\nnote: sample: ran it\n---\n\nsample: 2026-11-02\n---\ndue: 2026-11-19\ncreated: <now>\nsession: {session}\nstate: open\n---\n\nsample: after grove ships\n---\ndue: 2026-11-16\ncreated: <now>\nabout: {work}\nsession: {session}\nstate: open\n---\n\nsample: monday\n"
            ),
        )?;
        let mut lines = ledger(&root)?;
        for (at, held) in ids.iter().enumerate() {
            lines = lines.replace(held.as_str(), &format!("<{at}>"));
        }
        let deferred = |at: usize, field: &str, when: &str| {
            let mut line = json!({"at": "A", "id": format!("<{at}>"), "kind": "defer", "machine": "studio", "session": session});
            line[field] = when.into();
            line.to_string()
        };
        same(
            &lines,
            &[
                deferred(0, "due", "2026-11-14"),
                deferred(1, "due", "2026-11-15"),
                deferred(2, "due", "2026-11-21"),
                deferred(3, "due", "2026-11-16"),
                deferred(4, "due", "2026-12-01"),
                deferred(5, "due", "2026-11-17"),
                deferred(6, "due", "2026-11-28"),
                deferred(7, "due", "2026-11-02"),
                deferred(8, "after", "after grove ships"),
                format!(r#"{{"at":"A","id":"<7>","kind":"done","machine":"studio","note":"sample: ran it","session":"{session}"}}"#),
                r#"{"at":"A","id":"<0>","kind":"drop","machine":"studio","session":null}"#.to_owned(),
                r#"{"at":"A","due":"2026-11-19","id":"<8>","kind":"push","machine":"studio","session":null}"#.to_owned(),
            ]
            .join("\n"),
        )?;
        // twelve due: the seed lists ten, then how many more
        let many = format!("{root}/many");
        for day in 1u8..=12 {
            plant(
                &many,
                &[(
                    &format!("intentions/{day:08}.md"),
                    &format!(
                        "---\ndue: 2026-11-{day:02}\ncreated: 2026-10-01T00:00:00Z\nstate: open\n---\n\nsample: {day}\n"
                    ),
                )],
            )?;
        }
        let seeded = sandman(
            &[
                ("SANDMAN_MACHINE", "studio"),
                ("SANDMAN_ROOT", &many),
                ("SANDMAN_TODAY", "2026-11-14"),
            ],
            &["recall"],
            &format!(r#"{{"session":"{session}"}}"#),
        )?;
        let block = seeded
            .split_once("\n\n")
            .and_then(|(_, rest)| rest.split_once("always \u{b7}"))
            .map_or("", |(block, _)| block);
        same(
            block,
            &format!(
                "{header}{}  \u{2026} 2 more: sandman due\n",
                (1u8..=10)
                    .map(|day| format!(
                        "  \u{b7} {day:08} 2026-11-{day:02} \u{2014} sample: {day}\n"
                    ))
                    .collect::<Vec<_>>()
                    .concat()
            ),
        )?;
        // a malformed date is an event's words; a blocked store fails the write
        same(
            sandman(
                &[
                    ("SANDMAN_MACHINE", "studio"),
                    ("SANDMAN_ROOT", &odd),
                    ("SANDMAN_TODAY", "2026-11-14"),
                ],
                &["defer", "x", "--when", "2026-02-30"],
                "",
            )?
            .get(11..)
            .unwrap_or_default(),
            "waiting: 2026-02-30\n",
        )?;
        plant(&root, &[("blocked/intentions", "")])?;
        let blocked = sandman(
            &[("SANDMAN_ROOT", &format!("{root}/blocked"))],
            &["defer", "x", "--when", "today"],
            "",
        )?;
        same(blocked.get(..17).unwrap_or_default(), "1\nsandman defer: ")?;
        // the local date: the zone's, each side of the date line
        for zone in ["Pacific/Kiritimati", "Etc/GMT+12"] {
            let local = Command::new("date").arg("+%F").env("TZ", zone).output()?;
            same(
                sandman(
                    &[
                        ("SANDMAN_MACHINE", "studio"),
                        ("SANDMAN_ROOT", &odd),
                        ("TZ", zone),
                    ],
                    &["defer", "x", "--when", "today"],
                    "",
                )?
                .get(11..)
                .unwrap_or_default(),
                &String::from_utf8(local.stdout)?,
            )?;
        }
        Ok(())
    }

    /// The tick encodes this machine's taken episodes one a tick, oldest first, through the stub
    /// encoder: the guidance override, the contract and the digest on its stdin (highlights first,
    /// results dropped, the middle cut) and both guard variables in its env; traces into the bank
    /// of the directory they are about, else the cwd's, a clash `_2`; zero traces; skipped without
    /// a run, gone, and failed four ways with no files. A stop mid-run kills the
    /// encoder's group and writes no line, so the next daemon runs it again, into its timeout.
    #[test]
    fn encode() -> Outcome {
        let root = temp("encode")?;
        let (stub, work) = (format!("{root}/stub.sh"), format!("{root}/work"));
        let trace = r#"{"type":"feedback","name":"Sample: pass the clock in!","description":"sample: one line","body":"sample: the body\n\nWhy: sample."}"#;
        let about = |dir: &str| trace.replacen('{', &format!(r#"{{"about":"{work}/{dir}","#), 1);
        let (elsewhere, missing) = (about("elsewhere"), about("missing"));
        let filler = "x".repeat(3000);
        let ran = |name: &str, input: &str| json!({"type": "tool_use", "name": name, "input": {"command": input, "file_path": input}});
        let user = |cwd: &str, text: &str| json!({"type": "user", "cwd": cwd, "message": {"content": text}});
        let transcript = [
            user(&format!("{work}/r/gone"), "case two"),
            json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "sample: on it"}, ran("Bash", "sandman remember \"sample: keep the clock\""), ran("Bash", "sandman remember \"sample: not yet saved\"")]}}),
            json!({"type": "user", "message": {"content": [{"type": "tool_result", "content": "sample: dropped"}]}}),
            json!({"type": "user", "message": {"content": [{"type": "text", "text": filler}]}}),
            json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Read", "input": {"file_path": "/work/sample.rs"}}, {"type": "text", "text": "sample: done"}]}}),
        ]
        .map(|line| line.to_string())
        .join("\n");
        let mut files = vec![
            ("archive/a.jsonl".to_owned(), transcript),
            (
                "stub.sh".to_owned(),
                format!(
                    r#"echo "env $CLAUDE_MEMORY_PIPELINE $SANDMAN_NO_TAKE" >> "$0.log"
T='{trace}'
case $(tee -a "$0.log" | sed -n 's/^user: case //p') in
  two) printf '```json\n{{"traces":[%s,%s]}}\n```\n' "$T" "$T";;
  one) printf '{{"traces":[%s]}}\n' "$T";;
  none) printf '{{"traces":[]}}\n';;
  exit) printf 'sample: broke\n' >&2; exit 3;;
  junk) printf 'not json\n';;
  type) printf '{{"traces":[{{"type":"opinion","name":"x","description":"x","body":"x"}}]}}\n';;
  about) printf '{{"traces":[%s]}}\n' '{elsewhere}';;
  nodir) printf '{{"traces":[%s]}}\n' '{missing}';;
  slow) echo $$ >> "$0.pid"; exec sleep 30;;
esac
"#
                ),
            ),
            (
                "work/r/main/.git".to_owned(),
                "gitdir: /sample/bare/worktrees/main\n".to_owned(),
            ),
            (
                "work/r/next/.git".to_owned(),
                "gitdir: /sample/bare/worktrees/next\n".to_owned(),
            ),
            ("work/repo/.git/worktrees/x/HEAD".to_owned(), String::new()),
            (
                "work/repo/.claude/worktrees/x/.git".to_owned(),
                format!("gitdir: {work}/repo/.git/worktrees/x\n"),
            ),
            ("work/repo/src/lib.rs".to_owned(), String::new()),
            ("work/elsewhere/README".to_owned(), String::new()),
            // what session a remembered on the fly; a dream of it is not
            (
                "memories/-sample/user_said.md".to_owned(),
                "---\nname: Sample: said once\ndescription: sample: a memory remembered\ntype: user\nsession: @a\nsource: remember 2026-11-09T08:00:10Z \u{b7} @a\n---\n\nsample: other\n".to_owned(),
            ),
            (
                "memories/-sample/user_dreamed.md".to_owned(),
                "---\nname: Sample: dreamed\ndescription: x\ntype: user\nsession: @a\nsource: dream\n---\n\nsample: keep the clock\n".to_owned(),
            ),
            (
                "traces/-sample/feedback_kept.md".to_owned(),
                "---\nname: Sample: keep the clock\ndescription: sample: remembered at once\ntype: feedback\ncreated: 2999-01-01T00:00:00Z\nsession: @a\nsource: remember 2026-11-09T08:00:20Z \u{b7} @a\n---\n\nsample:  keep\n the clock\n".to_owned(),
            ),
            (
                "prompts/encode.md".to_owned(),
                "sample: guidance\n".to_owned(),
            ),
        ];
        let mut ledger = Vec::new();
        // the episodes, oldest first: (letter, its case, its cwd, its start's interactive, its machine)
        for (minute, (letter, case, cwd, interactive, machine)) in [
            ("a", "", "", true, "studio"),
            ("b", "one", "/repo/.claude/worktrees/x", true, "studio"),
            ("c", "one", "/repo/src", true, "studio"),
            ("d", "one", "", true, "studio"),
            ("e", "none", "/repo", true, "studio"),
            ("f", "one", "/repo", false, "studio"),
            ("g", "", "", true, "studio"),
            ("h", "exit", "/repo", true, "studio"),
            ("i", "junk", "/repo", true, "studio"),
            ("j", "type", "/repo", true, "studio"),
            ("k", "one", "/repo", true, "laptop"),
            ("m", "about", "/repo", true, "studio"),
            ("n", "nodir", "/repo", true, "studio"),
            ("l", "slow", "/repo", true, "studio"),
        ]
        .into_iter()
        .enumerate()
        {
            if !case.is_empty() {
                let at = if cwd.is_empty() {
                    String::new()
                } else {
                    format!("{work}{cwd}")
                };
                files.push((
                    format!("archive/{letter}.jsonl"),
                    user(&at, &format!("case {case}")).to_string(),
                ));
            }
            ledger.push(format!(
                r#"{{"at":"2026-11-09T08:{minute:02}:00Z","interactive":{interactive},"kind":"start","machine":"{machine}","session":"{ID}{letter}"}}
{{"archived":"{root}/archive/{letter}.jsonl","at":"2026-11-09T08:{minute:02}:30Z","cwd":null,"kind":"end","machine":"{machine}","outcome":"taken","session":"{ID}{letter}"}}"#
            ));
        }
        // read after its take, e still encodes, once
        ledger.push(format!(
            r#"{{"at":"2026-11-09T09:00:00Z","file":"x","kind":"show","machine":"studio","session":"{ID}e"}}"#
        ));
        files.push(("ledger/2026/11/09.jsonl".to_owned(), ledger.join("\n")));
        plant(
            &root,
            &files
                .iter()
                .map(|file| (file.0.as_str(), file.1.as_str()))
                .collect::<Vec<_>>(),
        )?;
        let (home, encoder) = (
            format!("{root}/home"),
            format!("sh {stub} --model sample-model"),
        );
        let envs = [
            ("HOME", home.as_str()),
            ("SANDMAN_DIGEST_BYTES", "2000"),
            ("SANDMAN_ENCODER", encoder.as_str()),
            ("SANDMAN_TICK_MS", "10"),
        ];
        let (mut daemon, _) = serve(
            &root,
            &[&envs[..], &[("SANDMAN_ENCODE_MS", "60000")]].concat(),
        )?;
        let read = |path: &str| fs::read_to_string(path).unwrap_or_default();
        until(|| !read(&format!("{stub}.pid")).is_empty())?;
        same(
            &format!(
                "{} {:?}",
                kill("-TERM", &daemon.0.id().to_string())?,
                daemon.0.wait()?.code()
            ),
            "true Some(0)",
        )?;
        let log = format!("{root}/run/serve.log");
        let (stopped, day) = (read(&log), date(&["+%Y/%m/%d"])?);
        let today = format!("{root}/ledger/{day}.jsonl");
        same(
            &format!(
                "{} {}",
                read(&today).contains(&format!("{ID}l")),
                stopped
                    .lines()
                    .last()
                    .and_then(|line| line.get(28..))
                    .unwrap_or_default()
            ),
            r#"false ,"event":"exit","reason":"signal TERM"}"#,
        )?;
        let (mut again, _) = serve(
            &root,
            &[&envs[..], &[("SANDMAN_ENCODE_MS", "300")]].concat(),
        )?;
        until(|| read(&today).contains(&format!("{ID}l")))?;
        same(
            &format!(
                "{} {:?}",
                kill("-TERM", &again.0.id().to_string())?,
                again.0.wait()?.code()
            ),
            "true Some(0)",
        )?;
        let lines = read(&today)
            .lines()
            .map(|line| {
                let mut entry: Value = serde_json::from_str(line)?;
                entry["at"] = json!(entry["at"].as_str().map(str::len));
                Ok(entry.to_string())
            })
            .collect::<Result<Vec<_>, serde_json::Error>>()?;
        let line = |letter: &str, outcome: &str, traces: u8, error: &str| {
            format!(
                r#"{{"at":20,{error}"kind":"encode","machine":"studio","outcome":"{outcome}","session":"{ID}{letter}","traces":{traces}}}"#
            )
        };
        same(
            &lines.join("\n"),
            &[
                line("a", "encoded", 2, ""),
                line("b", "encoded", 1, ""),
                line("c", "encoded", 1, ""),
                line("d", "encoded", 1, ""),
                line("e", "encoded", 0, ""),
                line("f", "skipped", 0, ""),
                line("g", "gone", 0, ""),
                line("h", "failed", 0, r#""error":"exit status: 3: sample: broke","#),
                line("i", "failed", 0, r#""error":"output: EOF while parsing a value at line 1 column 1","#),
                line(
                    "j",
                    "failed",
                    0,
                    r#""error":"output: not a trace: {\"body\":\"x\",\"description\":\"x\",\"name\":\"x\",\"type\":\"opinion\"}","#,
                ),
                line("m", "encoded", 1, ""),
                line("n", "encoded", 1, ""),
                line("l", "failed", 0, r#""error":"timed out after 300 ms","#),
            ]
            .join("\n"),
        )?;
        // what the stub read, run by run: the first whole, the rest by their case
        let seen = read(&format!("{stub}.log"));
        let runs: Vec<&str> = seen.split("env 1 1\n").skip(1).collect();
        let cases: Vec<&str> = runs
            .iter()
            .map(|run| {
                run.lines()
                    .find_map(|said| said.strip_prefix("user: case "))
                    .unwrap_or_default()
            })
            .collect();
        same(
            &format!("{}\n{cases:?}", runs.first().unwrap_or(&"")),
            &format!(
                "sample: guidance\n\n{}\n# episode\ndate: 2026-11-09T08:00:30Z\ncwd: {work}/r/gone\n\n# already remembered\n- Sample: said once \u{2014} sample: a memory remembered\n- Sample: keep the clock \u{2014} sample: remembered at once\n\n# highlights\n- sample: not yet saved\n\n# transcript\nuser: case two\nassistant: sample: on it\ntool: Bash sandman remember \"sample: keep the clock\"\ntool: Bash sandman remember \"sample: not yet saved\"\n\u{2026} 3007 bytes omitted \u{2026}\ntool: Read /work/sample.rs\nassistant: sample: done\n\n{:?}",
                include_str!("../src/prompts/encode.contract.md"),
                [
                    "two", "one", "one", "one", "none", "exit", "junk", "type", "about", "nodir",
                    "slow", "slow"
                ]
            ),
        )?;
        // every encoder the stop and the timeout ended is gone, its group with it
        for pid in read(&format!("{stub}.pid")).lines() {
            same(&kill("-0", pid)?.to_string(), "false")?;
        }
        let key = |dir: &str| dir.replace(|letter: char| !letter.is_ascii_alphanumeric(), "-");
        let mut written = Vec::new();
        for bank in fs::read_dir(format!("{root}/traces"))? {
            for file in fs::read_dir(bank?.path())? {
                let path = file?.path();
                written.push(
                    path.strip_prefix(format!("{root}/traces"))?
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
        written.sort();
        let first = read(&format!(
            "{root}/traces/{}/feedback_sample_pass_the_clock_in.md",
            key(&format!("{work}/r"))
        ));
        let now = first
            .lines()
            .nth(5)
            .and_then(|said| said.strip_prefix("created: "))
            .unwrap_or_default();
        let mut want = [
            format!(
                "{}/feedback_sample_pass_the_clock_in.md",
                key(&format!("{work}/r"))
            ),
            format!(
                "{}/feedback_sample_pass_the_clock_in_2.md",
                key(&format!("{work}/r"))
            ),
            format!(
                "{}/feedback_sample_pass_the_clock_in.md",
                key(&format!("{work}/repo"))
            ),
            format!(
                "{}/feedback_sample_pass_the_clock_in.md",
                key(&format!("{work}/repo/src"))
            ),
            format!("{}/feedback_sample_pass_the_clock_in.md", key(&home)),
            "-sample/feedback_kept.md".to_owned(),
            format!(
                "{}/feedback_sample_pass_the_clock_in.md",
                key(&format!("{work}/elsewhere"))
            ),
            format!(
                "{}/feedback_sample_pass_the_clock_in_2.md",
                key(&format!("{work}/repo"))
            ),
        ];
        want.sort();
        let abouts = [("elsewhere", ""), ("repo", "_2")].map(|(dir, clash)| {
            read(&format!(
                "{root}/traces/{}/feedback_sample_pass_the_clock_in{clash}.md",
                key(&format!("{work}/{dir}"))
            ))
            .lines()
            .nth(6)
            .unwrap_or_default()
            .to_owned()
        });
        same(
            &format!(
                "{}{written:?}\n{abouts:?}",
                masked(&first.replace(now, "<now>"))
            ),
            &format!(
                "---\nid: <id>\nname: Sample: pass the clock in!\ndescription: sample: one line\ntype: feedback\ncreated: <now>\nsession: {ID}a\narchived: {root}/archive/a.jsonl\nsource: encode <now> \u{b7} {ID}a \u{b7} model=sample-model\n---\n\nsample: the body\n\nWhy: sample.\n{want:?}\n{:?}",
                [
                    format!("about: {work}/elsewhere"),
                    format!("about: {work}/missing")
                ]
            ),
        )
    }

    /// The model run reads its token file at each run, so the stub encoder rearranges it for the
    /// next: absent, nothing set and nothing noted; mode 600, its trimmed text in
    /// `$CLAUDE_CODE_OAUTH_TOKEN` and the file named on the line; mode 644, refused and the token
    /// unset; empty, ignored. The token reaches no log, ledger or run file.
    #[test]
    fn token() -> Outcome {
        let root = temp("token")?;
        let (stub, file) = (format!("{root}/stub.sh"), format!("{root}/token"));
        let mut files = vec![(
            "stub.sh".to_owned(),
            format!(
                r#"case=$(sed -n 's/^user: case //p')
echo "$case ${{CLAUDE_CODE_OAUTH_TOKEN-unset}}" >> "$0.log"
case $case in
  absent) printf ' sample-token \n' > {file}; chmod 600 {file};;
  secret) chmod 644 {file};;
  loose) : > {file}; chmod 600 {file};;
esac
printf '{{"traces":[]}}\n'
"#
            ),
        )];
        let mut ledger = Vec::new();
        for (minute, (letter, case)) in [
            ("a", "absent"),
            ("b", "secret"),
            ("c", "loose"),
            ("d", "empty"),
        ]
        .into_iter()
        .enumerate()
        {
            files.push((
                format!("archive/{letter}.jsonl"),
                json!({"type": "user", "message": {"content": format!("case {case}")}}).to_string(),
            ));
            ledger.push(format!(
                r#"{{"at":"2026-11-09T08:{minute:02}:00Z","interactive":true,"kind":"start","machine":"studio","session":"{ID}{letter}"}}
{{"archived":"{root}/archive/{letter}.jsonl","at":"2026-11-09T08:{minute:02}:30Z","cwd":null,"kind":"end","machine":"studio","outcome":"taken","session":"{ID}{letter}"}}"#
            ));
        }
        files.push(("ledger/2026/11/09.jsonl".to_owned(), ledger.join("\n")));
        plant(
            &root,
            &files
                .iter()
                .map(|planted| (planted.0.as_str(), planted.1.as_str()))
                .collect::<Vec<_>>(),
        )?;
        let (home, encoder) = (format!("{root}/home"), format!("sh {stub}"));
        let (mut daemon, _) = serve(
            &root,
            &[
                ("HOME", home.as_str()),
                ("SANDMAN_CLAUDE_TOKEN_FILE", file.as_str()),
                ("SANDMAN_ENCODER", encoder.as_str()),
                ("SANDMAN_TICK_MS", "10"),
            ],
        )?;
        let read = |path: &str| fs::read_to_string(path).unwrap_or_default();
        let today = format!("{root}/ledger/{}.jsonl", date(&["+%Y/%m/%d"])?);
        until(|| read(&today).contains(&format!("{ID}d")))?;
        same(
            &format!(
                "{} {:?}",
                kill("-TERM", &daemon.0.id().to_string())?,
                daemon.0.wait()?.code()
            ),
            "true Some(0)",
        )?;
        let lines: Vec<String> = read(&today)
            .lines()
            .map(|line| line.get(29..).unwrap_or_default().to_owned())
            .collect();
        let line = |letter: &str, token: &str| {
            format!(
                r#""kind":"encode","machine":"studio","outcome":"encoded","session":"{ID}{letter}",{token}"traces":0}}"#
            )
        };
        same(
            &format!("{}\n{}", lines.join("\n"), read(&format!("{stub}.log"))),
            &format!(
                "{}\nabsent unset\nsecret sample-token\nloose unset\nempty unset\n",
                [
                    line("a", ""),
                    line("b", &format!(r#""token":"{file}","#)),
                    line(
                        "c",
                        &format!(r#""token":"refused: {file}: mode 100644, want a regular file of mode 600","#)
                    ),
                    line("d", &format!(r#""token":"ignored: {file} is empty","#)),
                ]
                .join("\n")
            ),
        )?;
        let written = [
            "run/encode.err",
            "run/encode.in",
            "run/encode.out",
            "run/serve.log",
        ]
        .map(|path| read(&format!("{root}/{path}")))
        .concat();
        same(
            &format!("{written}{}", read(&today))
                .contains("sample-token")
                .to_string(),
            "false",
        )
    }

    /// The tick consolidates one quiet bank a tick, the one whose oldest trace is oldest, never
    /// while an episode waits: the stub consolidator reads the default guidance, the contract and
    /// the bank (whole memories within the budget, the rest listed) with both guard variables set;
    /// keep, merge and both prunes land, every consumed or pruned file and the reply moved under
    /// `.pruned/`; each malformed reply, and a write that fails, leaves the tree as it was and
    /// writes `failed`, which only a newer trace re-arms; a stop mid-run kills the consolidator and
    /// writes no line, so the next daemon runs it again, into its timeout; `chart` reads the stamp.
    #[test]
    fn consolidate() -> Outcome {
        let root = temp("consolidate")?;
        let stub = format!("{root}/stub.sh");
        let note = |name: &str, kind: &str, created: &str, rest: &str, body: &str| {
            format!(
                "---\nname: {name}\ndescription: sample: {name}\ntype: {kind}\ncreated: {created}\n{rest}---\n\n{body}\n"
            )
        };
        let trace = |name: &str, created: &str, letter: &str| {
            note(
                name,
                "feedback",
                created,
                &format!(
                    "session: @{letter}\narchived: /sample/{letter}.jsonl\nsource: encode {created} \u{b7} @{letter} \u{b7} model=opus\n"
                ),
                "sample: trace body",
            )
        };
        let (big, small) = ("x".repeat(700), "sample: memory body");
        let alpha = [
            (
                "traces/alpha/feedback_t_keep.md",
                // remembered on the fly: kept like any trace, its id, from, session and about
                // carried
                note(
                    "T keep",
                    "feedback",
                    "2026-11-01T01:00:00Z",
                    "about: /work/alpha\nsession: @a\nid: 0a0a0a0a0a0a\nfrom: 1b1b1b1b1b1b\nsource: remember 2026-11-01T01:00:00Z \u{b7} @a\n",
                    "sample: trace body",
                ),
            ),
            (
                "traces/alpha/feedback_t_merge.md",
                trace("T merge", "2026-11-01T01:01:00Z", "b"),
            ),
            (
                "traces/alpha/feedback_t_prune.md",
                trace("T prune", "2026-11-01T01:02:00Z", "c"),
            ),
            (
                "memories/alpha/project_m_merge.md",
                note(
                    "M merge",
                    "project",
                    "2026-10-01T00:00:00Z",
                    // provenance off its first stamp, moved to before the new one; its id first,
                    // its origin gone once it has sessions
                    "origin: import\nsessions: @c\nrecall: pin\nid: 2c2c2c2c2c2c\nsource: remember 2026-10-01T00:00:00Z\n",
                    "sample: before",
                ),
            ),
            (
                "memories/alpha/reference_m_prune.md",
                note("M prune", "reference", "2026-10-02T00:00:00Z", "", small),
            ),
            (
                "memories/alpha/user_m_listed.md",
                note("M listed", "user", "2026-10-03T00:00:00Z", "", &big),
            ),
        ];
        let mut files: Vec<(String, String)> = alpha
            .iter()
            .map(|file| (file.0.to_owned(), file.1.clone()))
            .collect();
        // the failing banks, oldest trace first; each stub case is its bank's name
        let failing = [
            "exit", "junk", "shape", "verb", "field", "ghost", "twice", "listed", "again",
            "unnamed", "locked", "rearm", "slow",
        ];
        for (minute, bank) in failing.iter().enumerate() {
            files.push((
                format!("traces/{bank}/feedback_t.md"),
                trace("T", &format!("2026-11-01T02:{minute:02}:00Z"), "d"),
            ));
        }
        for (path, body) in [
            ("again/project_m.md", small),
            ("listed/project_m.md", &big),
            ("locked/project_m.md", small),
        ] {
            files.push((
                format!("memories/{path}"),
                note("M", "project", "2026-10-04T00:00:00Z", "", body),
            ));
        }
        // the oldest trace of all, but its bank is not quiet: its newest is still to come
        for (file, created) in [
            ("old", "2026-11-01T00:00:00Z"),
            ("new", "2999-01-01T00:00:00Z"),
        ] {
            files.push((
                format!("traces/quiet/feedback_{file}.md"),
                trace("T", created, "e"),
            ));
        }
        let keep = r#"{"op":"keep","trace":"feedback_t_keep.md","type":"project","name":"Sample: kept","description":"sample: kept","body":"sample: kept body"}"#;
        let merge = r#"{"op":"merge","trace":"feedback_t_merge.md","memory":"project_m_merge.md","description":"sample: merged","body":"sample: merged body"}"#;
        let prunes = r#"{"op":"prune","trace":"feedback_t_prune.md","reason":"sample: says what project_m_merge.md says"},{"op":"prune","memory":"reference_m_prune.md","reason":"sample: superseded"}"#;
        files.push((
            "stub.sh".to_owned(),
            format!(
                r#"echo "env $CLAUDE_MEMORY_PIPELINE $SANDMAN_NO_TAKE" >> "$0.log"
case $(tee -a "$0.log" | sed -n '/^# bank$/{{n;p;}}' | tail -n 1) in
  alpha) printf '%s' '{{"ops":[{keep},{merge},{prunes}]}}';;
  exit) printf 'sample: broke\n' >&2; exit 3;;
  junk) printf 'not json\n';;
  shape) printf '{{"ops":{{}}}}';;
  verb) printf '{{"ops":[{{"op":"forget","trace":"feedback_t.md"}}]}}';;
  field) printf '{{"ops":[{{"op":"keep","trace":"feedback_t.md","type":"opinion","name":"x","description":"x","body":"x"}}]}}';;
  ghost) printf '{{"ops":[{{"op":"prune","trace":"feedback_nope.md","reason":"x"}}]}}';;
  twice) printf '{{"ops":[{{"op":"prune","trace":"feedback_t.md","reason":"x"}},{{"op":"prune","trace":"feedback_t.md","reason":"x"}}]}}';;
  listed) printf '{{"ops":[{{"op":"merge","trace":"feedback_t.md","memory":"project_m.md","description":"x","body":"x"}}]}}';;
  again) printf '{{"ops":[{{"op":"merge","trace":"feedback_t.md","memory":"project_m.md","description":"x","body":"x"}},{{"op":"prune","memory":"project_m.md","reason":"x"}}]}}';;
  unnamed) printf '{{"ops":[]}}';;
  locked) printf '{{"ops":[{{"op":"keep","trace":"feedback_t.md","type":"user","name":"x","description":"x","body":"x"}}]}}';;
  rearm) grep -q feedback_t2 "$0.log" || {{ printf 'sample: broke\n' >&2; exit 3; }}
    printf '{{"ops":[{{"op":"prune","trace":"feedback_t.md","reason":"x"}},{{"op":"prune","trace":"feedback_t2.md","reason":"x"}}]}}';;
  slow) echo $$ >> "$0.pid"; exec sleep 30;;
esac
"#
            ),
        ));
        // two headless episodes still to encode: no bank is consolidated before both are
        let episodes = ["a", "b"].map(|letter| {
            format!(
                r#"{{"at":"2026-11-09T08:00:00Z","interactive":false,"kind":"start","machine":"studio","session":"@{letter}"}}
{{"archived":"/sample","at":"2026-11-09T08:00:30Z","kind":"end","machine":"studio","outcome":"taken","session":"@{letter}"}}"#
            )
        });
        files.push(("ledger/2026/11/09.jsonl".to_owned(), episodes.join("\n")));
        plant(
            &root,
            &files
                .iter()
                .map(|file| (file.0.as_str(), file.1.as_str()))
                .collect::<Vec<_>>(),
        )?;
        let locked = format!("{root}/memories/locked");
        fs::set_permissions(&locked, Permissions::from_mode(0o555))?;
        let consolidator = format!("sh {stub} --model sample-model");
        let envs = [
            ("SANDMAN_CONSOLIDATE_BYTES", "600"),
            ("SANDMAN_CONSOLIDATE_QUIET_MS", "0"),
            ("SANDMAN_CONSOLIDATOR", consolidator.as_str()),
            ("SANDMAN_TICK_MS", "10"),
        ];
        let read = |path: &str| fs::read_to_string(path).unwrap_or_default();
        let (log, today) = (
            format!("{root}/run/serve.log"),
            format!("{root}/ledger/{}.jsonl", date(&["+%Y/%m/%d"])?),
        );
        let stop = |mut daemon: Daemon| -> Outcome {
            same(
                &format!(
                    "{} {:?} {}",
                    kill("-TERM", &daemon.0.id().to_string())?,
                    daemon.0.wait()?.code(),
                    read(&log)
                        .lines()
                        .last()
                        .and_then(|line| line.get(28..))
                        .unwrap_or_default()
                ),
                r#"true Some(0) ,"event":"exit","reason":"signal TERM"}"#,
            )
        };
        let (daemon, _) = serve(
            &root,
            &[&envs[..], &[("SANDMAN_CONSOLIDATE_MS", "60000")]].concat(),
        )?;
        until(|| !read(&format!("{stub}.pid")).is_empty())?;
        stop(daemon)?;
        let rearm: Value = serde_json::from_str(
            read(&today)
                .lines()
                .find(|line| line.contains(r#""bank":"rearm""#))
                .ok_or("rearm")?,
        )?;
        let failed = rearm["at"].as_str().unwrap_or_default();
        until(|| date(&["+%Y-%m-%dT%H:%M:%SZ"]).is_ok_and(|now| now.as_str() > failed))?;
        plant(
            &root,
            &[(
                "traces/rearm/feedback_t2.md",
                &trace("T2", &date(&["+%Y-%m-%dT%H:%M:%SZ"])?, "f"),
            )],
        )?;
        let (again, _) = serve(
            &root,
            &[&envs[..], &[("SANDMAN_CONSOLIDATE_MS", "300")]].concat(),
        )?;
        until(|| read(&today).contains(r#""bank":"slow""#))?;
        stop(again)?;
        fs::set_permissions(&locked, Permissions::from_mode(0o755))?;
        let lines = read(&today)
            .lines()
            .map(|line| {
                let mut entry: Value = serde_json::from_str(line)?;
                entry["at"] = json!(entry["at"].as_str().map(str::len));
                Ok(entry.to_string())
            })
            .collect::<Result<Vec<_>, serde_json::Error>>()?;
        let line = |bank: &str, counts: [u8; 3], error: &str| {
            let mut entry = json!({
                "at": 20u8, "bank": bank, "kept": counts[0], "kind": "consolidate", "machine": "studio",
                "merged": counts[1], "outcome": "consolidated", "pruned": counts[2],
            });
            if !error.is_empty() {
                (entry["error"], entry["outcome"]) = (error.into(), "failed".into());
            }
            entry.to_string()
        };
        let op = |text: &str| format!("output: not an op: {text}");
        let mut want: Vec<String> = ["a", "b"]
            .map(|letter| {
                format!(
                    r#"{{"at":20,"kind":"encode","machine":"studio","outcome":"skipped","session":"{ID}{letter}","traces":0}}"#
                )
            })
            .into();
        want.extend([
            line("alpha", [1, 1, 2], ""),
            line("exit", [0; 3], "exit status: 3: sample: broke"),
            line("junk", [0; 3], "output: EOF while parsing a value at line 1 column 1"),
            line("shape", [0; 3], &op(r#"{"ops":{}}"#)),
            line("verb", [0; 3], &op(r#"{"op":"forget","trace":"feedback_t.md"}"#)),
            line(
                "field",
                [0; 3],
                &op(
                    r#"{"body":"x","description":"x","name":"x","op":"keep","trace":"feedback_t.md","type":"opinion"}"#,
                ),
            ),
            line("ghost", [0; 3], &op(r#"{"op":"prune","reason":"x","trace":"feedback_nope.md"}"#)),
            line("twice", [0; 3], &op(r#"{"op":"prune","reason":"x","trace":"feedback_t.md"}"#)),
            line(
                "listed",
                [0; 3],
                &op(
                    r#"{"body":"x","description":"x","memory":"project_m.md","op":"merge","trace":"feedback_t.md"}"#,
                ),
            ),
            line("again", [0; 3], &op(r#"{"memory":"project_m.md","op":"prune","reason":"x"}"#)),
            line("unnamed", [0; 3], "output: trace not named: feedback_t.md"),
            line("locked", [0; 3], "Permission denied (os error 13)"),
            line("rearm", [0; 3], "exit status: 3: sample: broke"),
            line("rearm", [0, 0, 2], ""),
            line("slow", [0; 3], "timed out after 300 ms"),
        ]);
        same(&lines.join("\n"), &want.join("\n"))?;
        // what the stub read, run by run: the first whole, the rest by their bank
        let seen = read(&format!("{stub}.log"));
        let runs: Vec<&str> = seen.split("env 1 1\n").skip(1).collect();
        let banks: Vec<&str> = runs
            .iter()
            .map(|run| {
                run.rsplit_once("\n# bank\n")
                    .and_then(|(_, rest)| rest.lines().next())
                    .unwrap_or_default()
            })
            .collect();
        let block = |tag: &str, path: &str| {
            let file = alpha
                .iter()
                .find(|held| held.0 == path)
                .map_or("", |held| held.1.as_str());
            format!(
                "<{tag} file=\"{}\">\n{}\n</{tag}>\n",
                path.rsplit('/').next().unwrap_or_default(),
                file.trim_end().replace('@', ID)
            )
        };
        let mut order = vec!["alpha"];
        order.extend(failing);
        order.extend(["rearm", "slow"]);
        same(
            &format!("{}\n{banks:?}", runs.first().unwrap_or(&"")),
            &format!(
                "{}\n{}\n# bank\nalpha\n\n# traces\n{}{}{}\n# memories\n{}{}\n# listed\n- user_m_listed.md \u{b7} M listed \u{b7} sample: M listed\n\n{order:?}",
                include_str!("../src/prompts/consolidate.md"),
                include_str!("../src/prompts/consolidate.contract.md"),
                block("trace", "traces/alpha/feedback_t_keep.md"),
                block("trace", "traces/alpha/feedback_t_merge.md"),
                block("trace", "traces/alpha/feedback_t_prune.md"),
                block("memory", "memories/alpha/project_m_merge.md"),
                block("memory", "memories/alpha/reference_m_prune.md"),
            ),
        )?;
        for pid in read(&format!("{stub}.pid")).lines() {
            same(&kill("-0", pid)?.to_string(), "false")?;
        }
        // every file left, `.pruned/<day>/<HHMMSS>-` cut to `.pruned/`
        let aside_dir = format!(".pruned/{}/", date(&["+%Y/%m/%d"])?);
        let found = Command::new("find")
            .args(["memories", "traces", ".pruned", "-type", "f"])
            .current_dir(&root)
            .output()?;
        let mut left: Vec<String> = String::from_utf8(found.stdout)?
            .lines()
            .map(|path| {
                path.strip_prefix(&aside_dir)
                    .and_then(|rest| rest.get(7..))
                    .map_or_else(|| path.to_owned(), |rest| format!(".pruned/{rest}"))
            })
            .collect();
        left.sort();
        let mut kept: Vec<String> = [
            ".pruned/alpha-feedback_t_keep.md",
            ".pruned/alpha-feedback_t_merge.md",
            ".pruned/alpha-feedback_t_prune.md",
            ".pruned/alpha-ops.json",
            ".pruned/alpha-project_m_merge.md",
            ".pruned/alpha-reference_m_prune.md",
            ".pruned/rearm-feedback_t.md",
            ".pruned/rearm-feedback_t2.md",
            ".pruned/rearm-ops.json",
            "memories/again/project_m.md",
            "memories/alpha/project_m_merge.md",
            "memories/alpha/project_sample_kept.md",
            "memories/alpha/user_m_listed.md",
            "memories/listed/project_m.md",
            "memories/locked/project_m.md",
            "traces/quiet/feedback_new.md",
            "traces/quiet/feedback_old.md",
        ]
        .map(str::to_owned)
        .into();
        kept.extend(
            failing
                .iter()
                .filter(|bank| !["rearm"].contains(bank))
                .map(|bank| format!("traces/{bank}/feedback_t.md")),
        );
        kept.sort();
        same(&format!("{left:?}"), &format!("{kept:?}"))?;
        let bank = |file: &str| read(&format!("{root}/memories/alpha/{file}"));
        let made = bank("project_sample_kept.md");
        let now = made
            .lines()
            .find_map(|said| said.strip_prefix("source: consolidate "))
            .and_then(|rest| rest.split(' ').next())
            .unwrap_or_default();
        let aside = |file: &str| -> io::Result<String> {
            let dir = format!("{root}/{aside_dir}");
            let path = fs::read_dir(&dir)?
                .flatten()
                .map(|entry| entry.path())
                .find(|path| path.to_string_lossy().ends_with(file))
                .unwrap_or_default();
            fs::read_to_string(path)
        };
        let source = |letter: &str| {
            format!("source: consolidate <now> \u{b7} {ID}{letter} \u{b7} model=sample-model")
        };
        same(
            &format!(
                "{}{}{}{}{}",
                made.replace(now, "<now>"),
                bank("project_m_merge.md").replace(now, "<now>"),
                aside("alpha-project_m_merge.md")?,
                aside("alpha-feedback_t_keep.md")?,
                aside("alpha-ops.json")?
            ),
            &format!(
                "---\nid: 0a0a0a0a0a0a\nname: Sample: kept\ndescription: sample: kept\ntype: project\ncreated: 2026-11-01T01:00:00Z\nabout: /work/alpha\nsession: {ID}a\nfrom: 1b1b1b1b1b1b\nsessions: {ID}a\n{}\n---\n\nsample: kept body\n---\nid: 2c2c2c2c2c2c\nname: M merge\ndescription: sample: merged\ntype: project\ncreated: 2026-10-01T00:00:00Z\nrecall: pin\nsessions: {ID}b {ID}c\n{}\nupdated: <now>\n---\n\nsample: merged body\n{}{}{}\n",
                source("a"),
                source("b"),
                alpha[3].1.replace('@', ID),
                alpha[0].1.replace('@', ID),
                serde_json::from_str::<Value>(&format!("{{\"ops\":[{keep},{merge},{prunes}]}}"))?
            ),
        )?;
        let chart: Value = serde_json::from_str(
            sandman(
                &[("SANDMAN_MACHINE", "studio"), ("SANDMAN_ROOT", &root)],
                &["chart"],
                "",
            )?
            .get(2..)
            .unwrap_or_default(),
        )?;
        let sources: Vec<String> = chart["memories"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|memory| memory["bank"] == "alpha")
            .map(|memory| {
                format!(
                    "{} {} {} {}",
                    memory["file"], memory["source"], memory["session"], memory["sessions"]
                )
            })
            .collect();
        same(
            &format!("{sources:?}"),
            &format!(
                "{:?}",
                [
                    format!(r#""project_m_merge.md" "consolidate" "{ID}b" ["{ID}b","{ID}c"]"#),
                    format!(r#""project_sample_kept.md" "consolidate" "{ID}a" ["{ID}a"]"#),
                    r#""user_m_listed.md" "other" null []"#.to_owned(),
                ]
            ),
        )
    }

    /// The knowledge store end to end. One episode's three traces split: the memory into the bank
    /// of its cwd, the knowledge and the playbook into the topic's `traces/@launchd/`; a knowledge
    /// type without a topic fails its episode. The topic consolidates as a bank does, into
    /// `knowledge/launchd/`: keeps start its pages, then a later episode's playbook trace merges
    /// into the page, rewritten whole. `remember` writes a playbook trace by topic (a topic alone
    /// means knowledge) and refuses a bad topic; recall ranks pages and their traces under the
    /// topic's label; `show` reads a page; `chart` lists it.
    #[test]
    fn knowledge() -> Outcome {
        let root = fs::canonicalize(temp("knowledge")?)?
            .to_str()
            .ok_or("root")?
            .to_owned();
        let (work, encoder, consolidator) = (
            format!("{root}/work"),
            format!("{root}/encode.sh"),
            format!("{root}/consolidate.sh"),
        );
        fs::create_dir_all(&work)?;
        let card = |kind: &str, name: &str, body: &str, topic: &str| {
            json!({"type": kind, "name": name, "description": format!("sample: {name}"), "body": body, "topic": topic}).to_string()
        };
        let (fact, steps, fixed) = (
            card(
                "knowledge",
                "Launchd ExitTimeOut kills",
                "sample: past ExitTimeOut the job is SIGKILLed.",
                "launchd",
            ),
            card(
                "playbook",
                "Swap a launchd daemon",
                "When: sample.\n\nSteps:\n1. sample: bootout, then bootstrap\n\nWatch for: sample.",
                "launchd",
            ),
            card(
                "playbook",
                "Swap a launchd daemon",
                "When: sample.\n\nSteps:\n1. sample: kickstart -k\n\nWatch for: sample: bootout raced KeepAlive (2026-11-10).",
                "launchd",
            ),
        );
        let memory = json!({"type": "feedback", "name": "Sample: terse", "description": "sample: terse", "body": "sample: terse", "about": null, "topic": "ignored"});
        let page = r#"{"op":"keep","trace":"playbook_swap_a_launchd_daemon.md","type":"playbook","name":"Swap a launchd daemon","description":"launchd: swap a running daemon","body":"When: sample.\n\nSteps:\n1. sample: bootout, then bootstrap\n\nWatch for: sample."}"#;
        let merge = r#"{"op":"merge","trace":"playbook_swap_a_launchd_daemon.md","memory":"playbook_swap_a_launchd_daemon.md","description":"launchd: swap a running daemon with kickstart -k","body":"When: sample.\n\nSteps:\n1. sample: kickstart -k\n\nWatch for: sample: bootout raced KeepAlive (2026-11-10)."}"#;
        for (path, text) in [
            (
                encoder.clone(),
                format!(
                    "case $(sed -n 's/^user: case //p') in\n  a) printf '%s' '{{\"traces\":[{memory},{fact},{steps}]}}';;\n  b) printf '%s' '{{\"traces\":[{fixed}]}}';;\n  c) printf '%s' '{{\"traces\":[{{\"type\":\"knowledge\",\"name\":\"x\",\"description\":\"x\",\"body\":\"x\"}}]}}';;\nesac\n"
                ),
            ),
            (
                consolidator.clone(),
                format!(
                    "input=$(cat)\ncase $(printf '%s\\n' \"$input\" | sed -n '/^# bank$/{{n;p;}}' | tail -n 1) in\n  @launchd) case ${{input##*'# bank'}} in *'# memories'*) printf '%s' '{{\"ops\":[{merge}]}}';; *) printf '%s' '{{\"ops\":[{{\"op\":\"keep\",\"trace\":\"knowledge_launchd_exittimeout_kills.md\",\"type\":\"knowledge\",\"name\":\"Launchd ExitTimeOut\",\"description\":\"launchd: a job past ExitTimeOut is killed\",\"body\":\"sample: SIGKILL past ExitTimeOut.\"}},{page}]}}';; esac;;\n  *) printf '%s' '{{\"ops\":[{{\"op\":\"prune\",\"trace\":\"feedback_sample_terse.md\",\"reason\":\"x\"}}]}}';;\nesac\n"
                ),
            ),
        ] {
            fs::write(path, text)?;
        }
        let episode = |letter: &str, minute: u8| -> io::Result<String> {
            let archived = format!("{root}/archive/{letter}.jsonl");
            fs::create_dir_all(format!("{root}/archive"))?;
            fs::write(
                &archived,
                json!({"type": "user", "cwd": work, "message": {"content": format!("case {letter}")}}).to_string(),
            )?;
            Ok(format!(
                "{{\"at\":\"2026-11-09T08:{minute:02}:00Z\",\"interactive\":true,\"kind\":\"start\",\"machine\":\"studio\",\"session\":\"{ID}{letter}\"}}\n{{\"archived\":\"{archived}\",\"at\":\"2026-11-09T08:{minute:02}:30Z\",\"kind\":\"end\",\"machine\":\"studio\",\"outcome\":\"taken\",\"session\":\"{ID}{letter}\"}}\n"
            ))
        };
        fs::create_dir_all(format!("{root}/ledger/2026/11"))?;
        fs::write(
            format!("{root}/ledger/2026/11/09.jsonl"),
            [episode("a", 0)?, episode("c", 1)?].concat(),
        )?;
        let (encoding, consolidating) = (format!("sh {encoder}"), format!("sh {consolidator}"));
        let (mut daemon, _) = serve(
            &root,
            &[
                ("HOME", root.as_str()),
                ("SANDMAN_CONSOLIDATE_QUIET_MS", "0"),
                ("SANDMAN_CONSOLIDATOR", consolidating.as_str()),
                ("SANDMAN_ENCODER", encoding.as_str()),
                ("SANDMAN_TICK_MS", "10"),
            ],
        )?;
        let read = |path: &str| fs::read_to_string(path).unwrap_or_default();
        let today = format!("{root}/ledger/{}.jsonl", date(&["+%Y/%m/%d"])?);
        let consolidated = |count: usize| read(&today).matches("\"consolidate\"").count() == count;
        until(|| consolidated(2))?;
        // the next episode, once the first is consolidated
        fs::write(format!("{root}/ledger/2026/11/10.jsonl"), episode("b", 2)?)?;
        until(|| consolidated(3))?;
        // TERM, not the guard's KILL: a killed daemon writes no coverage
        same(
            &format!(
                "{} {:?}",
                kill("-TERM", &daemon.0.id().to_string())?,
                daemon.0.wait()?.code()
            ),
            "true Some(0)",
        )?;
        let lines = read(&today)
            .lines()
            .map(|line| {
                let mut entry: Value = serde_json::from_str(line)?;
                entry["at"] = "A".into();
                Ok(entry.to_string())
            })
            .collect::<Result<Vec<_>, serde_json::Error>>()?;
        let bank = work.replace(|letter: char| !letter.is_ascii_alphanumeric(), "-");
        let line = |key: &str, counts: [u8; 3]| {
            format!(
                r#"{{"at":"A","bank":"{key}","kept":{},"kind":"consolidate","machine":"studio","merged":{},"outcome":"consolidated","pruned":{}}}"#,
                counts[0], counts[1], counts[2]
            )
        };
        same(
            &lines.join("\n"),
            &[
                format!(r#"{{"at":"A","kind":"encode","machine":"studio","outcome":"encoded","session":"{ID}a","traces":3}}"#),
                format!(
                    r#"{{"at":"A","error":"output: not a trace: {{\"body\":\"x\",\"description\":\"x\",\"name\":\"x\",\"type\":\"knowledge\"}}","kind":"encode","machine":"studio","outcome":"failed","session":"{ID}c","traces":0}}"#
                ),
                line(&bank, [0, 0, 1]),
                line("@launchd", [2, 0, 0]),
                format!(r#"{{"at":"A","kind":"encode","machine":"studio","outcome":"encoded","session":"{ID}b","traces":1}}"#),
                line("@launchd", [0, 1, 0]),
            ]
            .join("\n"),
        )?;
        // every file the pipeline left, `.pruned/<day>/<HHMMSS>-[<n>-]` cut to `.pruned/`
        let aside_dir = format!(".pruned/{}/", date(&["+%Y/%m/%d"])?);
        let found = Command::new("find")
            .args(["knowledge", "memories", "traces", ".pruned", "-type", "f"])
            .current_dir(&root)
            .output()?;
        let mut left: Vec<String> = String::from_utf8(found.stdout)?
            .lines()
            .map(|path| {
                path.strip_prefix(&aside_dir)
                    .and_then(|rest| rest.get(7..))
                    .map_or_else(
                        || path.to_owned(),
                        |rest| {
                            let unclashed = rest
                                .split_once('-')
                                .filter(|pair| pair.0.parse::<u8>().is_ok());
                            format!(".pruned/{}", unclashed.map_or(rest, |pair| pair.1))
                        },
                    )
            })
            .collect();
        left.sort();
        let mut want = [
            ".pruned/@launchd-knowledge_launchd_exittimeout_kills.md",
            ".pruned/@launchd-ops.json",
            ".pruned/@launchd-ops.json",
            ".pruned/@launchd-playbook_swap_a_launchd_daemon.md",
            ".pruned/@launchd-playbook_swap_a_launchd_daemon.md",
            ".pruned/@launchd-playbook_swap_a_launchd_daemon.md",
            "knowledge/launchd/knowledge_launchd_exittimeout.md",
            "knowledge/launchd/playbook_swap_a_launchd_daemon.md",
        ]
        .map(str::to_owned)
        .to_vec();
        want.extend([
            format!(".pruned/{bank}-feedback_sample_terse.md"),
            format!(".pruned/{bank}-ops.json"),
        ]);
        want.sort();
        same(&format!("{left:?}"), &format!("{want:?}"))?;
        let playbook = read(&format!(
            "{root}/knowledge/launchd/playbook_swap_a_launchd_daemon.md"
        ));
        let stamp = |field: &str| {
            playbook
                .lines()
                .find_map(|said| said.strip_prefix(field))
                .unwrap_or_default()
                .to_owned()
        };
        let (created, updated) = (stamp("created: "), stamp("updated: "));
        same(
            &masked(
                &playbook
                    .replace(&format!("created: {created}"), "created: <created>")
                    .replace(&updated, "<updated>"),
            ),
            &format!(
                "---\nid: <id>\nname: Swap a launchd daemon\ndescription: launchd: swap a running daemon with kickstart -k\ntype: playbook\ncreated: <created>\ntopic: launchd\nsession: {ID}a\narchived: {root}/archive/a.jsonl\nsessions: {ID}a {ID}b\nsource: consolidate <updated> \u{b7} {ID}b \u{b7} model=sh\nupdated: <updated>\n---\n\nWhen: sample.\n\nSteps:\n1. sample: kickstart -k\n\nWatch for: sample: bootout raced KeepAlive (2026-11-10).\n"
            ),
        )?;
        let envs = [
            ("HOME", root.as_str()),
            ("SANDMAN_MACHINE", "studio"),
            ("SANDMAN_ROOT", &root),
        ];
        let mut out = String::new();
        for args in [
            &[
                "remember",
                "sample: drain on SIGTERM",
                "--type",
                "playbook",
                "--topic",
                "launchd",
                "--name",
                "Drain a daemon",
            ][..],
            &[
                "remember",
                "sample: bootout before bootstrap",
                "--topic",
                "launchd",
            ],
            &["remember", "x", "--type", "knowledge"],
            &["remember", "x", "--topic", "Launch_D"],
            &["remember", "x", "--type", "playbook", "--topic", "-launchd"],
        ] {
            out.push_str(&ran(command(&envs, args).current_dir(&work), "")?);
        }
        let refused = "2\nsandman remember: takes a non-empty \"<body>\" first and --type user|feedback|project|reference, or knowledge|playbook with --topic <slug>\n";
        same(
            &out,
            &format!(
                "0\nsandman show traces/@launchd/playbook_drain_a_daemon.md\n0\nsandman show traces/@launchd/knowledge_sample_bootout_before_bootstrap.md\n{}",
                refused.repeat(3)
            ),
        )?;
        let remembered = read(&format!(
            "{root}/traces/@launchd/playbook_drain_a_daemon.md"
        ));
        let now = remembered
            .lines()
            .find_map(|said| said.strip_prefix("created: "))
            .unwrap_or_default();
        same(
            &masked(&remembered.replace(now, "<now>")),
            "---\nid: <id>\nname: Drain a daemon\ndescription: sample: drain on SIGTERM\ntype: playbook\ncreated: <now>\ntopic: launchd\norigin: hand\nsource: remember <now> \u{b7} -\n---\n\nsample: drain on SIGTERM\n",
        )?;
        // recall: the pages and their traces under the topic's label, no repository's bonus
        let path = stub(&root)?;
        fs::write(
            format!("{root}/answers"),
            r#"{"answers":{"0":{"noul":0.9},"1":{"noul":0.8},"2":{"noul":0.2},"3":{"noul":0.1}},"usage":{"input_tokens":1}}"#,
        )?;
        let recalled = sandman(
            &[&envs[..], &[("PATH", path.as_str())]].concat(),
            &[
                "recall",
                "--cue",
                "swap the launchd daemon",
                "--about",
                &work,
            ],
            "",
        )?;
        same(
            recalled
                .split_once("recall: scorer")
                .map_or("", |pair| pair.0),
            "0\nrecall \u{b7} swap the launchd daemon \u{b7} 4 of 4 \u{b7} pplx\n1.00 Swap a launchd daemon (playbook \u{b7} knowledge/launchd/playbook_swap_a_launchd_daemon.md) \u{2014} launchd: swap a running daemon with kickstart -k\n.98 Launchd ExitTimeOut (knowledge \u{b7} knowledge/launchd/knowledge_launchd_exittimeout.md) \u{2014} launchd: a job past ExitTimeOut is killed\n.97 ~Drain a daemon (playbook \u{b7} traces/@launchd/playbook_drain_a_daemon.md) \u{2014} sample: drain on SIGTERM\n.95 ~sample: bootout before bootstrap (knowledge \u{b7} traces/@launchd/knowledge_sample_bootout_before_bootstrap.md) \u{2014} sample: bootout before bootstrap\nread one in full: sandman show <path>\n",
        )?;
        same(
            &read(&format!("{root}/pplx.in"))
                .contains(
                    "\"instructions\":\"Memory (knowledge \u{b7} launchd): Swap a launchd daemon",
                )
                .to_string(),
            "true",
        )?;
        same(
            &sandman(
                &envs,
                &[
                    "show",
                    "knowledge/launchd/playbook_swap_a_launchd_daemon.md",
                ],
                "",
            )?
            .replace(&updated, "<updated>"),
            "0\nSwap a launchd daemon (playbook) \u{b7} <updated>\nWhen: sample.\n\nSteps:\n1. sample: kickstart -k\n\nWatch for: sample: bootout raced KeepAlive (2026-11-10).\n",
        )?;
        let chart: Value =
            serde_json::from_str(sandman(&envs, &["chart"], "")?.get(2..).unwrap_or_default())?;
        same(
            &chart["knowledge"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|listed| {
                    format!(
                        "{} {} {} {}",
                        listed["topic"], listed["file"], listed["type"], listed["source"]
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            "\"launchd\" \"knowledge_launchd_exittimeout.md\" \"knowledge\" \"consolidate\"\n\"launchd\" \"playbook_swap_a_launchd_daemon.md\" \"playbook\" \"consolidate\"",
        )
    }

    /// The weekly review. With too little evidence it never runs; then, one review an idle tick: a
    /// failed run, a reply of the wrong shape, an oversize guidance, nothing proposed, a proposal
    /// the replay judges worse and one whose judge gives no verdict leave `prompts/` alone; one
    /// judged no worse is adopted for both jobs, the defaults kept under `prompts/.history/`; a
    /// stop mid-run writes no line. The reflector reads the guidance in force and the evidence since
    /// the start, the consolidator a past bank rebuilt from `.pruned/`; the next encode runs on the
    /// adopted guidance, and no review is due again within the week.
    #[test]
    fn reflect() -> Outcome {
        let root = temp("reflect")?;
        let note = |name: &str, extra: &str, body: &str| {
            format!(
                "---\nname: {name}\ndescription: sample: {name}\ntype: feedback\n{extra}---\n\n{body}\n"
            )
        };
        let born = |at: &str, letter: &str| format!("created: {at}\nsession: @{letter}\n");
        let runs = [
            json!({"at": "2026-11-09T08:01:00Z", "kind": "encode", "machine": "studio", "outcome": "encoded", "session": "@a", "traces": 0u8}),
            json!({"at": "2026-11-09T08:02:00Z", "kind": "encode", "machine": "studio", "outcome": "encoded", "session": "@b", "traces": 0u8}),
            json!({"at": "2026-11-09T08:03:00Z", "bank": "-sample", "kept": 1u8, "kind": "consolidate", "machine": "studio", "merged": 1u8, "outcome": "consolidated", "pruned": 0u8}),
            json!({"at": "2026-11-09T08:04:00Z", "error": "sample: broke", "kind": "encode", "machine": "studio", "outcome": "failed", "session": "@c", "traces": 0u8}),
        ];
        let sent = |letter: &str, files: &[&str]| json!({"at": "2026-11-09T09:00:00Z", "kind": "recall", "machine": "studio", "sent": files.iter().map(|file| json!({"file": file, "score": 1u8})).collect::<Vec<_>>(), "session": format!("@{letter}")});
        let mut ledger: Vec<Value> = ["a", "b", "d"]
            .iter()
            .flat_map(|letter| {
                [
                    json!({"at": "2026-11-09T08:00:00Z", "interactive": *letter != "d", "kind": "start", "machine": "studio", "session": format!("@{letter}")}),
                    json!({"archived": format!("{root}/archive/{letter}.jsonl"), "at": "2026-11-09T08:00:30Z", "kind": "end", "machine": "studio", "outcome": "taken", "session": format!("@{letter}")}),
                ]
            })
            .collect();
        ledger.extend(runs.clone());
        ledger.extend([
            json!({"at": "2026-11-09T08:05:00Z", "kind": "encode", "machine": "laptop", "outcome": "encoded", "session": "@f", "traces": 0u8}),
            json!({"at": "2026-11-09T09:00:00Z", "file": "traces/-sample/feedback_t.md", "kind": "remember", "machine": "studio", "session": "@a", "type": "feedback"}),
            sent("a", &["-sample/x.md", "-sample/y.md"]),
            sent("b", &["-sample/x.md", "-sample/y.md"]),
            sent("c", &["-sample/x.md", "-sample/y.md", "-sample/z.md"]),
            json!({"at": "2026-11-09T09:01:00Z", "file": "-sample/y.md", "kind": "show", "machine": "studio", "session": "@b"}),
        ]);
        let day = |date: &str, file: &str| format!(".pruned/2026/11/{date}/120000--sample-{file}");
        let merge = |trace: &str| {
            format!(
                r#"{{"op":"merge","trace":"{trace}","memory":"project_m.md","description":"sample: merged","body":"sample: body"}}"#
            )
        };
        let files = [
            ("archive/a.jsonl".to_owned(), json!({"type": "user", "message": {"content": "case a"}}).to_string()),
            ("ledger/2026/11/09.jsonl".to_owned(), ledger.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")),
            (day("01", "ops.json"), format!(r#"{{"ops":[{},{{"op":"prune","trace":"feedback_u.md","reason":"sample: the session's story"}}]}}"#, merge("feedback_t.md"))),
            (day("01", "feedback_t.md"), note("T", &format!("{}source: remember 2026-11-01T11:00:00Z \u{b7} @a\n", born("2026-11-01T11:00:00Z", "a")), "sample: t")),
            (day("01", "feedback_u.md"), note("U", &born("2026-11-01T11:00:00Z", "a"), "sample: u")),
            (day("01", "project_m.md"), note("M", "", "sample: m first")),
            (day("02", "ops.json"), format!(r#"{{"ops":[{},{{"op":"keep","trace":"feedback_w.md","type":"project","name":"Sample: kept","description":"sample: kept","body":"sample: kept"}}]}}"#, merge("feedback_v.md"))),
            (day("02", "feedback_v.md"), note("V", &born("2026-11-02T10:00:00Z", "a"), "sample: v")),
            (day("02", "feedback_w.md"), note("W", &born("2026-11-02T11:00:00Z", "b"), "sample: w")),
            (day("02", "project_m.md"), note("M", "", "sample: m second")),
            ("memories/-sample/project_m.md".to_owned(), note("M", "", "sample: m now")),
            ("memories/-sample/project_sample_kept.md".to_owned(), note("Sample: kept", &born("2026-11-02T11:00:00Z", "b"), "sample: kept")),
            ("memories/-sample/user_other.md".to_owned(), note("Other", "", "sample: other")),
            ("reflector.sh.cases".to_owned(), "exit\nshape\nbig\nnone\nworse\nverdict\nadopt\nslow\n".to_owned()),
            ("reflector.sh".to_owned(), r#"cat > "$0.stdin"
if grep -qx '# b' "$0.stdin"; then
  a=$(sed -n '/^# a$/{n;p;}' "$0.stdin")
  case $(cat "$0.case") in
    worse) case $a in *old*) echo '{"better":"a"}';; *) echo '{"better":"b"}';; esac;;
    verdict) echo '{"better":"c"}';;
    adopt) if grep -qx '# bank' "$0.stdin"; then echo '{"better":"same"}'
      else case $a in *new*) echo '{"better":"a"}';; *) echo '{"better":"b"}';; esac; fi;;
  esac
  exit
fi
c=$(head -n 1 "$0.cases"); tail -n +2 "$0.cases" > "$0.rest"; mv "$0.rest" "$0.cases"; echo "$c" > "$0.case"
case $c in
  exit) cp "$0.stdin" "$0.first"; echo 'sample: broke' >&2; exit 3;;
  shape) echo '{"encode":1,"consolidate":null,"reason":"x"}';;
  big) echo '{"encode":"'$(printf '%0101d' 0)'","consolidate":null,"reason":"x"}';;
  none) echo '{"encode":null,"consolidate":null,"reason":"sample: nothing repeats"}';;
  worse) echo '{"encode":"sample: new worse","consolidate":null,"reason":"sample: worse"}';;
  verdict) echo '{"encode":"sample: new verdict","consolidate":null,"reason":"x"}';;
  adopt) echo '{"encode":"sample: new encode","consolidate":"sample: new consolidate","reason":"sample: both"}';;
  slow) echo $$ > "$0.pid"; exec sleep 30;;
esac
"#.to_owned()),
            ("encoder.sh".to_owned(), r#"g=$(head -n 1); echo "$g" >> "$0.log"
case $g in 'sample: new'*) u=new;; *) u=old;; esac
printf '{"traces":[],"under":"%s"}\n' $u
"#.to_owned()),
            ("consolidator.sh".to_owned(), r#"cat > "$0.in"
case $(head -n 1 "$0.in") in 'sample: new'*) u=new;; *) u=old;; esac
printf '{"ops":[],"under":"%s"}\n' $u
"#.to_owned()),
        ];
        plant(
            &root,
            &files
                .iter()
                .map(|file| (file.0.as_str(), file.1.as_str()))
                .collect::<Vec<_>>(),
        )?;
        let stub = |name: &str| format!("sh {root}/{name}.sh");
        let (encoder, consolidator, reflector) =
            (stub("encoder"), stub("consolidator"), stub("reflector"));
        let envs = [
            ("SANDMAN_CONSOLIDATOR", consolidator.as_str()),
            ("SANDMAN_ENCODER", encoder.as_str()),
            ("SANDMAN_GUIDANCE_BYTES", "100"),
            ("SANDMAN_REFLECT_REPLAY", "1"),
            ("SANDMAN_REFLECTOR", reflector.as_str()),
            ("SANDMAN_TICK_MS", "10"),
        ];
        let read = |path: &str| fs::read_to_string(format!("{root}/{path}")).unwrap_or_default();
        let today = format!("ledger/{}.jsonl", date(&["+%Y/%m/%d"])?);
        let stop = |mut daemon: Daemon| -> Outcome {
            same(
                &format!(
                    "{} {:?}",
                    kill("-TERM", &daemon.0.id().to_string())?,
                    daemon.0.wait()?.code()
                ),
                "true Some(0)",
            )
        };
        // due by the clock, short of evidence: the tick passes the headless episode and stops there
        let (first, _) = serve(&root, &[&envs[..], &[("SANDMAN_REFLECT_MS", "0")]].concat())?;
        until(|| read(&today).contains("skipped"))?;
        thread::sleep(Duration::from_millis(200));
        stop(first)?;
        same(
            &format!(
                "{} {}",
                read(&today).lines().count(),
                read("reflector.sh.case").is_empty()
            ),
            "1 true",
        )?;
        let (second, _) = serve(
            &root,
            &[
                &envs[..],
                &[("SANDMAN_REFLECT_MIN", "0"), ("SANDMAN_REFLECT_MS", "0")],
            ]
            .concat(),
        )?;
        until(|| !read("reflector.sh.pid").is_empty())?;
        stop(second)?;
        same(
            &kill("-0", read("reflector.sh.pid").trim())?.to_string(),
            "false",
        )?;
        let reviews = || -> Result<Vec<String>, Box<dyn Error>> {
            read(&today)
                .lines()
                .filter(|line| line.contains(r#""kind":"reflect""#))
                .map(|line| {
                    let mut entry: Value = serde_json::from_str(line)?;
                    entry["at"] = json!(entry["at"].as_str().map(str::len));
                    for path in entry["history"]
                        .as_object_mut()
                        .into_iter()
                        .flat_map(|history| history.values_mut())
                    {
                        *path = format!(
                            "prompts/.history/<stamp>{}",
                            path.as_str()
                                .and_then(|held| held.get(34..))
                                .unwrap_or_default()
                        )
                        .into();
                    }
                    Ok(entry.to_string())
                })
                .collect()
        };
        let review =
            |rest: &str| format!(r#"{{"at":20,{rest}"kind":"reflect","machine":"studio","#);
        let failed = |error: &str| {
            format!(
                r#"{}"outcome":"failed"}}"#,
                review(&format!(r#""error":{},"history":{{}},"#, json!(error)))
            )
        };
        let kept = |reason: &str, replay: &str| {
            format!(
                r#"{}"outcome":"kept","reason":"{reason}","replay":{{{replay}}}}}"#,
                review(r#""history":{},"#)
            )
        };
        let want = [
            failed("exit status: 3: sample: broke"),
            failed(r#"output: not a proposal: {"consolidate":null,"encode":1,"reason":"x"}"#),
            failed("output: encode guidance over 100 bytes"),
            kept("sample: nothing repeats", ""),
            kept("sample: worse", r#""encode":{"new":0,"old":1,"same":0}"#),
            failed(r#"output: not a verdict: {"better":"c"}"#),
            format!(
                r#"{}"outcome":"adopted","reason":"sample: both","replay":{{"consolidate":{{"new":0,"old":0,"same":1}},"encode":{{"new":1,"old":0,"same":0}}}}}}"#,
                review(
                    r#""history":{"consolidate":"prompts/.history/<stamp>-consolidate.md","encode":"prompts/.history/<stamp>-encode.md"},"#
                )
            ),
        ];
        same(&reviews()?.join("\n"), &want.join("\n"))?;
        // what the reflector read first: the guidance in force, then the evidence since the start
        let [encode, consolidate] = ["encode", "consolidate"].map(|job| {
            [
                ("encode", include_str!("../src/prompts/encode.md")),
                ("consolidate", include_str!("../src/prompts/consolidate.md")),
            ]
            .iter()
            .find(|prompt| prompt.0 == job)
            .map_or("", |prompt| prompt.1)
        });
        let evidence = format!(
            "# runs\n{}\n# ops\n- -sample: {{\"description\":\"sample: merged\",\"memory\":\"project_m.md\",\"op\":\"merge\",\"trace\":\"feedback_t.md\"}}\n- -sample: {{\"op\":\"prune\",\"reason\":\"sample: the session's story\",\"trace\":\"feedback_u.md\"}}\n- -sample: {{\"description\":\"sample: merged\",\"memory\":\"project_m.md\",\"op\":\"merge\",\"trace\":\"feedback_v.md\"}}\n- -sample: {{\"description\":\"sample: kept\",\"name\":\"Sample: kept\",\"op\":\"keep\",\"trace\":\"feedback_w.md\",\"type\":\"project\"}}\n\n# merged again\n- -sample/project_m.md \u{b7} merged 2 times\n\n# remembered by hand\n- T \u{2014} sample: T (traces/-sample/feedback_t.md)\n\n# sent, never read\n- -sample/x.md \u{b7} sent 3 times, never shown\n\n",
            runs.iter()
                .map(|run| ["- ", &run.to_string(), "\n"].concat())
                .collect::<String>()
        );
        same(
            &read("reflector.sh.first"),
            &format!(
                "{}\n{}\n<guidance job=\"encode\">\n{}\n</guidance>\n<guidance job=\"consolidate\">\n{}\n</guidance>\n\n# since\nthe start\n\n{}",
                include_str!("../src/prompts/reflect.md"),
                include_str!("../src/prompts/reflect.contract.md"),
                encode.trim_end(),
                consolidate.trim_end(),
                evidence.replace('@', ID)
            ),
        )?;
        // the last consolidate replay, under the new guidance: the 11/02 run's traces and the
        // memory it merged as it stood, beside the bank's other memory; its keep left out
        same(
            &read("consolidator.sh.in"),
            &format!(
                "sample: new consolidate\n{}\n# bank\n-sample\n\n# traces\n<trace file=\"feedback_v.md\">\n{}</trace>\n<trace file=\"feedback_w.md\">\n{}</trace>\n\n# memories\n<memory file=\"project_m.md\">\n{}</memory>\n<memory file=\"user_other.md\">\n{}</memory>\n",
                include_str!("../src/prompts/consolidate.contract.md"),
                files[7].1.replace('@', ID),
                files[8].1.replace('@', ID),
                files[9].1,
                files[12].1
            ),
        )?;
        let mut history: Vec<String> =
            fs::read_dir(format!("{root}/prompts/.history/{}", date(&["+%Y/%m/%d"])?))?
                .map(|entry| fs::read_to_string(entry?.path()))
                .collect::<io::Result<_>>()?;
        history.sort();
        let mut defaults = [consolidate, encode].map(str::to_owned);
        defaults.sort();
        same(
            &format!(
                "{history:?} {} {}",
                read("prompts/encode.md"),
                read("prompts/consolidate.md")
            ),
            &format!("{defaults:?} sample: new encode sample: new consolidate"),
        )?;
        // the next episode encodes under the adopted guidance; the next review is a week off
        let grown = format!(
            "{}\n{}\n{}",
            read("ledger/2026/11/09.jsonl"),
            json!({"at": "2026-11-09T10:00:00Z", "interactive": true, "kind": "start", "machine": "studio", "session": format!("{ID}e")}),
            json!({"archived": format!("{root}/archive/a.jsonl"), "at": "2026-11-09T10:00:30Z", "kind": "end", "machine": "studio", "outcome": "taken", "session": format!("{ID}e")})
        );
        fs::write(format!("{root}/ledger/2026/11/09.jsonl"), grown)?;
        let (third, _) = serve(
            &root,
            &[&envs[..], &[("SANDMAN_REFLECT_MIN", "0")]].concat(),
        )?;
        until(|| read(&today).contains(&format!("{ID}e")))?;
        thread::sleep(Duration::from_millis(200));
        stop(third)?;
        let old = encode.lines().next().unwrap_or_default();
        same(
            &format!("{}\n{}", reviews()?.len(), read("encoder.sh.log")),
            &format!(
                "7\n{}\n",
                [
                    old,
                    "sample: new worse",
                    old,
                    "sample: new verdict",
                    old,
                    "sample: new encode",
                    "sample: new encode"
                ]
                .join("\n")
            ),
        )
    }

    /// An adoption whose job's next two model runs both fail is rolled back once: the earlier
    /// version restored from `prompts/.history/`, the failing one kept there beside it; a job
    /// whose next run held is left.
    #[test]
    fn revert() -> Outcome {
        let root = temp("revert")?;
        let lines = [
            json!({"at": "2026-11-01T00:00:00Z", "history": {"consolidate": "prompts/.history/c.md", "encode": "prompts/.history/e.md"}, "kind": "reflect", "machine": "studio", "outcome": "adopted"}),
            json!({"at": "2026-11-01T00:01:00Z", "bank": "-x", "kind": "consolidate", "machine": "studio", "outcome": "consolidated"}),
            json!({"at": "2026-11-01T00:02:00Z", "kind": "encode", "machine": "studio", "outcome": "skipped", "session": "@p"}),
            json!({"at": "2026-11-01T00:03:00Z", "kind": "encode", "machine": "studio", "outcome": "failed", "session": "@p"}),
            json!({"at": "2026-11-01T00:04:00Z", "kind": "encode", "machine": "studio", "outcome": "failed", "session": "@q"}),
        ];
        plant(
            &root,
            &[
                (
                    "ledger/2026/11/01.jsonl",
                    &lines
                        .iter()
                        .map(Value::to_string)
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                ("prompts/.history/e.md", "sample: earlier\n"),
                ("prompts/consolidate.md", "sample: consolidate\n"),
                ("prompts/encode.md", "sample: adopted\n"),
            ],
        )?;
        let (mut daemon, _) = serve(&root, &[("SANDMAN_TICK_MS", "10")])?;
        let read = |path: &str| fs::read_to_string(format!("{root}/{path}")).unwrap_or_default();
        let today = format!("ledger/{}.jsonl", date(&["+%Y/%m/%d"])?);
        until(|| !read(&today).is_empty())?;
        thread::sleep(Duration::from_millis(200));
        same(
            &format!(
                "{} {:?}",
                kill("-TERM", &daemon.0.id().to_string())?,
                daemon.0.wait()?.code()
            ),
            "true Some(0)",
        )?;
        let mut line: Value = serde_json::from_str(&read(&today))?;
        let kept = line["history"]["encode"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        (line["at"], line["history"]["encode"]) = (
            json!(line["at"].as_str().map(str::len)),
            json!(
                kept.strip_prefix("prompts/.history/")
                    .and_then(|file| file.get(17..))
            ),
        );
        same(
            &format!(
                "{} {line} {}{}",
                read(&today).lines().count(),
                read(&kept),
                read("prompts/encode.md")
            ),
            "1 {\"at\":20,\"history\":{\"encode\":\"-encode.md\"},\"kind\":\"reflect\",\"machine\":\"studio\",\"outcome\":\"reverted\",\"reason\":\"its next two runs failed\"} sample: adopted\nsample: earlier\n",
        )
    }

    /// `chart` into a pipe whose reader is gone ends quietly: exit 0, nothing on stderr.
    #[test]
    fn closed() -> Outcome {
        let (reader, writer) = io::pipe()?;
        drop(reader);
        let out = Command::new(env!("CARGO_BIN_EXE_sandman"))
            .arg("chart")
            .env("SANDMAN_MACHINE", "studio")
            .env("SANDMAN_ROOT", temp("closed")?)
            .stdout(writer)
            .output()?;
        same(
            &format!("{}{}", out.status.success(), String::from_utf8(out.stderr)?),
            "true",
        )
    }

    /// Without `SANDMAN_MACHINE`, `chart` mints `<root>/machine` once and reads it after; an
    /// absent root charts as empty lists.
    #[test]
    fn machine() -> Outcome {
        let root = format!("{}/absent", temp("machine")?);
        let envs = [("SANDMAN_ROOT", root.as_str())];
        let runs = [
            sandman(&envs, &["chart"], "")?,
            sandman(&envs, &["chart"], "")?,
        ];
        let id = fs::read_to_string(format!("{root}/machine"))?;
        let got = runs.iter().map(|run| {
            let chart: Value = serde_json::from_str(run.get(2..).unwrap_or_default())?;
            Ok(format!(
                "{} {} {} {}",
                chart["active"], chart["machine"], chart["memories"], chart["taken"]
            ))
        });
        same(
            &format!(
                "{} {}",
                id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()),
                got.collect::<Result<Vec<_>, serde_json::Error>>()?
                    .join(" ")
            ),
            &format!("true [] \"{id}\" [] [] [] \"{id}\" [] []"),
        )
    }

    /// `mcp` over stdio: a modern `server/discover` refused so Claude Code falls back to
    /// `initialize`, the client's version echoed when known, else the latest; notifications
    /// unanswered; `tools/list` the seven tools; each call the verb's stdout, a failure its stderr
    /// line with `isError` (`due` with and without `all`, a close, an unknown id, a push);
    /// protocol errors for an unknown tool and for a line that is no request; EOF exits 0. The
    /// session is `$CLAUDE_CODE_SESSION_ID`, a call's `session`, else one minted for the process,
    /// and a pull repeats nothing the session was sent.
    #[test]
    fn mcp() -> Outcome {
        let root = fs::canonicalize(temp("mcp")?)?
            .to_str()
            .ok_or("root")?
            .to_owned();
        plant(&root, &CORPUS)?;
        let work = format!("{root}/work");
        fs::create_dir_all(&work)?;
        let call = |id: u8, name: &str, arguments: Value| {
            json!({"id": id, "jsonrpc": "2.0", "method": "tools/call", "params": {"arguments": arguments, "name": name}}).to_string()
        };
        let cue = json!({"budget": 1000u16, "cue": "add a clock to the alpha tests"});
        let lines = [
            r#"{"id":"probe","jsonrpc":"2.0","method":"server/discover","params":{}}"#.to_owned(),
            r#"{"id":0,"jsonrpc":"2.0","method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#.to_owned(),
            r#"{"id":1,"jsonrpc":"2.0","method":"initialize","params":{"protocolVersion":"1900-01-01"}}"#.to_owned(),
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_owned(),
            r#"{"id":2,"jsonrpc":"2.0","method":"ping"}"#.to_owned(),
            r#"{"id":3,"jsonrpc":"2.0","method":"tools/list"}"#.to_owned(),
            call(4, "recall", cue.clone()),
            call(5, "recall", cue),
            call(6, "recall", json!({"cue": "x", "session": "nope"})),
            call(7, "show", json!({"path": "-work-alpha/project_vpn.md"})),
            call(8, "show", Value::Null),
            call(9, "remember", json!({"body": "sample: by tool"})),
            call(10, "remember", json!({})),
            call(11, "nap", json!({})),
            call(12, "due", json!({"all": true})),
            call(13, "done", json!({"id": "0a1b", "note": "sample: by tool"})),
            call(14, "due", json!({"all": false})),
            call(15, "drop", json!({"id": "zz"})),
            call(16, "defer", json!({"id": "6e7f", "when": "+1w"})),
            "not json".to_owned(),
            "[]".to_owned(),
            String::new(),
        ];
        let session = format!("{ID}c");
        let envs = [
            ("CLAUDE_CODE_SESSION_ID", session.as_str()),
            ("SANDMAN_MACHINE", "studio"),
            ("SANDMAN_ROOT", &root),
            ("SANDMAN_SCORERS", ""),
            ("SANDMAN_TODAY", "2026-11-14"),
        ];
        let out = ran(
            command(&envs, &["mcp"]).current_dir(&work),
            &lines.join("\n"),
        )?;
        let mut got = out.lines().take(1).map(str::to_owned).collect::<Vec<_>>();
        for line in out.lines().skip(1) {
            let mut reply: Value = serde_json::from_str(line)?;
            if reply["id"] == 3u8 {
                // the descriptions are prose; the names and their arguments are the contract
                let tools = reply["result"]["tools"]
                    .as_array()
                    .ok_or("tools")?
                    .iter()
                    .map(|tool| {
                        let schema = &tool["inputSchema"];
                        format!(
                            "{} {} {}",
                            tool["name"],
                            schema["required"],
                            schema["properties"]
                                .as_object()
                                .map(|keys| keys.keys().cloned().collect::<Vec<_>>().join(","))
                                .unwrap_or_default()
                        )
                    })
                    .collect::<Vec<_>>();
                reply["result"] = tools.join(" | ").into();
            }
            got.push(reply.to_string());
        }
        // no session in the env: one minted, the same for every call; a call's own wins
        let thanks = call(0, "recall", json!({"cue": "thanks"}));
        let own = call(
            1,
            "recall",
            json!({"cue": "thanks", "session": format!("{ID}d")}),
        );
        drop(sandman(
            &envs[1..],
            &["mcp"],
            &[thanks.as_str(), &thanks, &own, ""].join("\n"),
        )?);
        let mut minted = BTreeSet::new();
        for line in ledger(&root)?.lines() {
            let entry: Value = serde_json::from_str(line)?;
            let owner = entry["session"].as_str().ok_or("session")?;
            let shown = owner.strip_prefix(ID).unwrap_or_else(|| {
                let _: bool = minted.insert(owner.to_owned());
                "minted"
            });
            got.push(format!("{} {shown}", entry["kind"]));
        }
        let key = root.replace(|letter: char| !letter.is_ascii_alphanumeric(), "-");
        same(
            &format!("{:?}", minted.iter().map(String::len).collect::<Vec<_>>()),
            "[36]",
        )?;
        same(
            &got.join("\n")
                .replace(&key, "<key>")
                .replace(&root, "<root>"),
            "0\n{\"error\":{\"code\":-32601,\"message\":\"Method not found\"},\"id\":\"probe\",\"jsonrpc\":\"2.0\"}\n{\"id\":0,\"jsonrpc\":\"2.0\",\"result\":{\"capabilities\":{\"tools\":{}},\"protocolVersion\":\"2025-06-18\",\"serverInfo\":{\"name\":\"sandman\",\"version\":\"0.1.0\"}}}\n{\"id\":1,\"jsonrpc\":\"2.0\",\"result\":{\"capabilities\":{\"tools\":{}},\"protocolVersion\":\"2025-11-25\",\"serverInfo\":{\"name\":\"sandman\",\"version\":\"0.1.0\"}}}\n{\"id\":2,\"jsonrpc\":\"2.0\",\"result\":{}}\n{\"id\":3,\"jsonrpc\":\"2.0\",\"result\":\"\\\"defer\\\" [\\\"what\\\",\\\"when\\\"] about,id,session,what,when | \\\"done\\\" [\\\"id\\\"] id,note,session | \\\"drop\\\" [\\\"id\\\"] id,note,session | \\\"due\\\" [] all,session | \\\"recall\\\" [\\\"cue\\\"] about,budget,cue,session | \\\"remember\\\" [\\\"body\\\"] about,body,session,topic,type | \\\"show\\\" [\\\"path\\\"] path,session\"}\n{\"id\":4,\"jsonrpc\":\"2.0\",\"result\":{\"content\":[{\"text\":\"recall \u{b7} add a clock to the alpha tests \u{b7} 5 of 6 \u{b7} lexical: embed failed\\n1.00 Pass a clock in (feedback \u{b7} -work-alpha/feedback_clock.md) \u{2014} Tests take time from an injected clock, never a mocked one: a mock hides the drift a real clock shows within a day, and every fixture pins its dates \u{2014} na\u{ef}ve caf\\n.25 Alpha deploys need the vpn (project \u{b7} -work-alpha/project_vpn.md) \u{2014} the deploy host is private\\n.19 The grammar lives in one file (reference \u{b7} -work-alpha/reference_grammar.md) \u{2014} src/grammar.pest\\n.00 ~Beta retries back off twice (feedback \u{b7} traces/-work-beta/feedback_retry.md) \u{2014} a failed beta deploy retries twice, then stops\\n.00 Beta ties break by date (feedback \u{b7} -work-beta/feedback_tie.md) \u{2014} newer first\\nread one in full: sandman show <path>\\n\",\"type\":\"text\"}],\"isError\":false}}\n{\"id\":5,\"jsonrpc\":\"2.0\",\"result\":{\"content\":[{\"text\":\"recall \u{b7} add a clock to the alpha tests \u{b7} 1 of 6 \u{b7} lexical: embed failed\\n.00 Beta pins its node version (project \u{b7} -work-beta/project_node.md) \u{2014} node 22 from mise\\nread one in full: sandman show <path>\\n\",\"type\":\"text\"}],\"isError\":false}}\n{\"id\":6,\"jsonrpc\":\"2.0\",\"result\":{\"content\":[{\"text\":\"sandman: --session takes a session id\\n\",\"type\":\"text\"}],\"isError\":true}}\n{\"id\":7,\"jsonrpc\":\"2.0\",\"result\":{\"content\":[{\"text\":\"Alpha deploys need the vpn (project) \u{b7} 2026-11-05T00:00:00Z\\nConnect first.\\n\",\"type\":\"text\"}],\"isError\":false}}\n{\"id\":8,\"jsonrpc\":\"2.0\",\"result\":{\"content\":[{\"text\":\"sandman show: no memory  (want [traces/]<bank>/<file>.md or knowledge/<topic>/<file>.md under <root>)\\n\",\"type\":\"text\"}],\"isError\":true}}\n{\"id\":9,\"jsonrpc\":\"2.0\",\"result\":{\"content\":[{\"text\":\"sandman show traces/<key>-work/feedback_sample_by_tool.md\\n\",\"type\":\"text\"}],\"isError\":false}}\n{\"id\":10,\"jsonrpc\":\"2.0\",\"result\":{\"content\":[{\"text\":\"sandman remember: takes a non-empty \\\"<body>\\\" first and --type user|feedback|project|reference, or knowledge|playbook with --topic <slug>\\n\",\"type\":\"text\"}],\"isError\":true}}\n{\"error\":{\"code\":-32602,\"message\":\"Unknown tool: nap\"},\"id\":11,\"jsonrpc\":\"2.0\"}\n{\"id\":12,\"jsonrpc\":\"2.0\",\"result\":{\"content\":[{\"text\":\"due \u{b7} deferred work: raise it with the person, never start it unasked\\n  \u{b7} 0a1b2c3d 2026-11-03 \u{2014} Rerun the alpha bench on the new box (~/work/alpha)\\n  \u{b7} 6e7f8a9b 2026-11-10 \u{2014} Draft the beta release notes\\n  \u{b7} 4e5f6a7b waiting: after grove ships \u{2014} Cut sandman over to the rewrite\\n  \u{b7} 8c9d0e1f 2099-01-01 \u{2014} Renew the signing key\\n\",\"type\":\"text\"}],\"isError\":false}}\n{\"id\":13,\"jsonrpc\":\"2.0\",\"result\":{\"content\":[{\"text\":\"0a1b2c3d done\\n\",\"type\":\"text\"}],\"isError\":false}}\n{\"id\":14,\"jsonrpc\":\"2.0\",\"result\":{\"content\":[{\"text\":\"due \u{b7} deferred work: raise it with the person, never start it unasked\\n  \u{b7} 6e7f8a9b 2026-11-10 \u{2014} Draft the beta release notes\\n  \u{b7} 4e5f6a7b waiting: after grove ships \u{2014} Cut sandman over to the rewrite\\n\",\"type\":\"text\"}],\"isError\":false}}\n{\"id\":15,\"jsonrpc\":\"2.0\",\"result\":{\"content\":[{\"text\":\"sandman drop: zz is not one intention's id (0 match)\\n\",\"type\":\"text\"}],\"isError\":true}}\n{\"id\":16,\"jsonrpc\":\"2.0\",\"result\":{\"content\":[{\"text\":\"6e7f8a9b 2026-11-21\\n\",\"type\":\"text\"}],\"isError\":false}}\n{\"error\":{\"code\":-32700,\"message\":\"Parse error\"},\"id\":null,\"jsonrpc\":\"2.0\"}\n{\"error\":{\"code\":-32700,\"message\":\"Parse error\"},\"id\":null,\"jsonrpc\":\"2.0\"}\n\"recall\" c\n\"recall\" c\n\"show\" c\n\"remember\" c\n\"done\" c\n\"push\" c\n\"recall\" minted\n\"recall\" minted\n\"recall\" d",
        )
    }

    /// `forget` erases every store's copy of one session and records nothing: its transcripts and
    /// sidecars in every project, archived copies, pointer, its traces, memories and knowledge pages
    /// (by `session:`) and their embeddings, its pruned copies and the ops on them, the ledger and
    /// log lines naming it, encode's run files when they hold it and any other naming it; a merged
    /// memory's source and an intention keep themselves, without it. Again: nothing to forget. A bad
    /// id is exit 2, a failed removal exit 1.
    #[test]
    fn forget() -> Outcome {
        let root = temp("forget")?;
        let claude = format!("{root}/claude");
        let envs = [
            ("CLAUDE_CONFIG_DIR", claude.as_str()),
            ("SANDMAN_ROOT", root.as_str()),
        ];
        let note = |session: &str, source: &str| {
            format!(
                "---\nname: sample\ntype: feedback\nsession: {session}\nsource: {source}\n---\n\nsample body\n"
            )
        };
        let (mine, other) = (
            note("@a", "remember 2026-11-09T09:00:00Z \u{b7} @a"),
            note("@b", "encode \u{b7} @b"),
        );
        // the id comes off a `sessions:` line, the line with it when it was the only one
        let merged = note(
            "@b",
            "consolidate 2026-11-10T04:00:00Z \u{b7} @a \u{b7} model=opus",
        )
        .replace("source:", "sessions: @a @c\nsource:");
        let backfilled = other.replace("source:", "sessions: @a\nsource:");
        let ledger = [
            r#"{"at":"2026-11-09T08:00:00Z","kind":"start","session":"@a"}"#,
            r#"{"at":"2026-11-09T08:01:00Z","kind":"start","session":"@b"}"#,
            r#"{"archived":"/r/.archive/claude/2026/11/09/093005-projects--work-alpha-@a.jsonl","at":"2026-11-09T09:30:00Z","kind":"end","outcome":"taken","session":"@a"}"#,
            r#"{"at":"2026-11-09T09:31:00Z","kind":"encode","outcome":"encoded","session":"@a"}"#,
            r#"{"at":"2026-11-09T09:32:00Z","kind":"guard","rationale":"sample","session":"@a","tool":null}"#,
            r#"{"at":"2026-11-09T09:40:00Z","kind":"consolidate","outcome":"consolidated"}"#,
        ]
        .join("\n");
        let embeddings = "{\"h\":\"1\",\"path\":\"traces/-work-alpha/feedback_a.md\"}\n{\"h\":\"2\",\"path\":\"traces/-work-alpha/feedback_b.md\"}\n{\"h\":\"3\",\"path\":\"-work-alpha/feedback_kept.md\"}\n{\"h\":\"4\",\"path\":\"knowledge/launchd/playbook_a.md\"}\n{\"h\":\"5\",\"path\":\"traces/@launchd/knowledge_a.md\"}\nnot json\n";
        let ops = r#"{"ops":[{"op":"keep","trace":"feedback_a.md"},{"op":"merge","memory":"feedback_merged.md","trace":"feedback_b.md"},{"memory":"project_old.md","op":"prune","reason":"stale"}]}"#;
        plant(
            &root,
            &[
                ("claude/projects/-work-alpha/@a.jsonl", "{}\n"),
                (
                    "claude/projects/-work-alpha/@a/subagents/agent.jsonl",
                    "{}\n",
                ),
                ("claude/projects/-work-alpha/@b.jsonl", "{}\n"),
                ("claude/projects/-work-beta/@a.jsonl", "{}\n"),
                (
                    ".archive/claude/2026/11/09/093005-projects--work-alpha-@a.jsonl",
                    "{}\n",
                ),
                (
                    ".archive/claude/2026/11/09/093005-projects--work-alpha-@a-dir/agent.jsonl",
                    "{}\n",
                ),
                (
                    ".archive/claude/2026/11/09/120000-projects--work-alpha-@b.jsonl",
                    "{}\n",
                ),
                (".pruned/2026/11/10/040000--work-alpha-feedback_a.md", &mine),
                (
                    ".pruned/2026/11/10/040000--work-alpha-feedback_b.md",
                    &other,
                ),
                (".pruned/2026/11/10/040000--work-alpha-ops.json", ops),
                (
                    ".pruned/2026/11/10/040001--work-alpha-feedback_merged.md",
                    &merged,
                ),
                ("embeddings.jsonl", embeddings),
                (
                    "intentions/1f2e3d4c.md",
                    "---\ndue: 2026-11-12\nsession: @a\nstate: open\n---\n\nsample\n",
                ),
                (
                    "intentions/9c8b7a6d.md",
                    "---\ndue: 2026-11-12\nsession: @b\nstate: open\n---\n\nsample\n",
                ),
                ("knowledge/launchd/playbook_a.md", &mine),
                ("ledger/2026/11/09.jsonl", &ledger),
                (
                    "ledger/2026/11/10.jsonl",
                    r#"{"at":"2026-11-10T08:00:00Z","kind":"end","session":"@b"}"#,
                ),
                ("memories/.recent/@a.json", "{}\n"),
                ("memories/.recent/@b.json", "{}\n"),
                ("memories/-work-alpha/feedback_kept.md", &mine),
                ("memories/-work-alpha/feedback_merged.md", &merged),
                ("memories/-work-alpha/project_other.md", &backfilled),
                ("run/consolidate.in", "session: @a\n"),
                ("run/consolidate.out", "{}\n"),
                ("run/encode.err", ""),
                ("run/encode.in", "# episode\n"),
                ("run/encode.out", "{}\n"),
                ("run/reflect.in", "sample\n"),
                (
                    "run/serve.log",
                    "{\"event\":\"take\",\"session\":\"@a\"}\n{\"event\":\"take\",\"session\":\"@b\"}\n",
                ),
                (
                    "run/serve.log.1",
                    "{\"event\":\"take\",\"session\":\"@b\"}\n",
                ),
                ("run/serve.pid", "1"),
                ("traces/-work-alpha/feedback_a.md", &mine),
                ("traces/-work-alpha/feedback_b.md", &other),
                ("traces/@launchd/knowledge_a.md", &mine),
            ],
        )?;
        let forget = |letter: &str| sandman(&envs, &["forget", &format!("{ID}{letter}")], "");
        same(&forget("a")?, "0\nforgot: 15 removed, 8 rewritten\n")?;
        // what is left, each file under the root by path, and the rewritten ones' text
        let mut left = Vec::new();
        let mut dirs = vec![PathBuf::from(&root)];
        while let Some(dir) = dirs.pop() {
            for entry in fs::read_dir(dir)?.flatten() {
                if entry.file_type()?.is_dir() {
                    dirs.push(entry.path());
                } else {
                    left.push(
                        entry
                            .path()
                            .to_string_lossy()
                            .replace(&format!("{root}/"), "")
                            .replace(ID, "@"),
                    );
                }
            }
        }
        left.sort();
        let read = |path: &str| {
            fs::read_to_string(format!("{root}/{path}")).map(|held| held.replace(ID, "@"))
        };
        same(
            &format!(
                "{}\n{}{}{}{}{}{}{}{}",
                left.join("\n"),
                read("ledger/2026/11/09.jsonl")?,
                read(".pruned/2026/11/10/040000--work-alpha-ops.json")?,
                read(".pruned/2026/11/10/040001--work-alpha-feedback_merged.md")?,
                read("memories/-work-alpha/feedback_merged.md")?,
                read("memories/-work-alpha/project_other.md")?,
                read("intentions/1f2e3d4c.md")?,
                read("embeddings.jsonl")?,
                read("run/serve.log")?,
            ),
            &format!(
                "{}\n{}\n{}{}{}{}{}{}{}",
                [
                    ".archive/claude/2026/11/09/120000-projects--work-alpha-@b.jsonl",
                    ".pruned/2026/11/10/040000--work-alpha-feedback_b.md",
                    ".pruned/2026/11/10/040000--work-alpha-ops.json",
                    ".pruned/2026/11/10/040001--work-alpha-feedback_merged.md",
                    "claude/projects/-work-alpha/@b.jsonl",
                    "embeddings.jsonl",
                    "intentions/1f2e3d4c.md",
                    "intentions/9c8b7a6d.md",
                    "ledger/2026/11/09.jsonl",
                    "ledger/2026/11/10.jsonl",
                    "memories/-work-alpha/feedback_merged.md",
                    "memories/-work-alpha/project_other.md",
                    "memories/.recent/@b.json",
                    "run/consolidate.out",
                    "run/reflect.in",
                    "run/serve.log",
                    "run/serve.log.1",
                    "run/serve.pid",
                    "traces/-work-alpha/feedback_b.md",
                ]
                .join("\n"),
                r#"{"at":"2026-11-09T08:01:00Z","kind":"start","session":"@b"}"#,
                r#"{"at":"2026-11-09T09:40:00Z","kind":"consolidate","outcome":"consolidated"}"#,
                r#"{"ops":[{"memory":"feedback_merged.md","op":"merge","trace":"feedback_b.md"},{"memory":"project_old.md","op":"prune","reason":"stale"}]}"#,
                note(
                    "@b",
                    "consolidate 2026-11-10T04:00:00Z \u{b7} - \u{b7} model=opus"
                )
                .replace("source:", "sessions: @c\nsource:")
                .repeat(2),
                other,
                "---\ndue: 2026-11-12\nstate: open\n---\n\nsample\n",
                "{\"h\":\"2\",\"path\":\"traces/-work-alpha/feedback_b.md\"}\nnot json\n",
                "{\"event\":\"take\",\"session\":\"@b\"}\n",
            ),
        )?;
        same(&forget("a")?, "0\nnothing to forget\n")?;
        same(
            &sandman(&envs, &["forget", "a"], "")?,
            "2\nsandman forget: takes a session id\n",
        )?;
        let locked = format!("{claude}/projects/-work-alpha");
        fs::set_permissions(&locked, Permissions::from_mode(0o555))?;
        let failed = forget("b");
        fs::set_permissions(&locked, Permissions::from_mode(0o755))?;
        same(
            &failed?,
            "1\nsandman forget: Permission denied (os error 13)\n",
        )
    }

    /// `forget` rewriting today's ledger while other processes append to it loses none of their
    /// lines: each waits on the lock and writes to the file that replaced it.
    #[test]
    fn forget_race() -> Outcome {
        let root = temp("forget-race")?;
        let envs = [
            ("SANDMAN_MACHINE", "studio"),
            ("SANDMAN_ROOT", root.as_str()),
        ];
        let mark = |letter: &str| {
            sandman(
                &envs,
                &["mark"],
                &format!(r#"{{"kind":"start","session":"{ID}{letter}"}}"#),
            )
        };
        let marks = thread::scope(|scope| -> Result<usize, Box<dyn Error>> {
            let writers = [(); 3].map(|()| {
                scope.spawn(|| {
                    (0u8..30)
                        .filter(|_| mark("b").is_ok_and(|out| out == "0\n"))
                        .count()
                })
            });
            for _ in 0u8..20 {
                same(&mark("a")?, "0\n")?;
                same(
                    sandman(&envs, &["forget", &format!("{ID}a")], "")?
                        .get(..9)
                        .unwrap_or_default(),
                    "0\nforgot:",
                )?;
            }
            Ok(writers
                .into_iter()
                .map(|writer| writer.join().unwrap_or_default())
                .sum())
        })?;
        let lines = ledger(&root)?;
        same(
            &format!(
                "{marks} {} {}",
                lines.matches(&format!("{ID}b")).count(),
                lines.matches(&format!("{ID}a")).count()
            ),
            "90 90 0",
        )
    }

    /// A `mark` waiting on the lock of a ledger file that is replaced meanwhile (as `forget`
    /// replaces it) opens the replacement and writes there, every time: the race `forget_race`
    /// only makes likely, forced. An attempt whose `mark` opened the file after the swap (it then
    /// writes at once, unlocked) proves nothing and is run again.
    #[test]
    fn forget_reopen() -> Outcome {
        let root = temp("forget-reopen")?;
        let today = format!("{root}/ledger/{}.jsonl", date(&["+%Y/%m/%d"])?);
        fs::create_dir_all(PathBuf::from(&today).parent().ok_or("parent")?)?;
        let reopened = (0u8..5).try_fold(false, |done, _| -> Result<bool, Box<dyn Error>> {
            if done {
                return Ok(true);
            }
            let held = File::create(&today)?;
            held.lock()?;
            let mut mark = command(
                &[("SANDMAN_MACHINE", "studio"), ("SANDMAN_ROOT", &root)],
                &["mark"],
            )
            .stdin(Stdio::piped())
            .spawn()?;
            mark.stdin
                .take()
                .ok_or("stdin")?
                .write_all(format!(r#"{{"kind":"start","session":"{ID}a"}}"#).as_bytes())?;
            thread::sleep(Duration::from_millis(300));
            fs::write(format!("{today}.new"), "")?;
            fs::rename(format!("{today}.new"), &today)?;
            thread::sleep(Duration::from_millis(100));
            let waited = fs::read_to_string(&today)?.is_empty();
            drop(held);
            same(&format!("{:?}", mark.wait()?.code()), "Some(0)")?;
            Ok(waited && fs::read_to_string(&today)?.contains(&format!("{ID}a")))
        })?;
        same(&reopened.to_string(), "true")
    }

    /// `mark` refuses bad stdin with exit 2 and no write (a guard without a rationale too), appends
    /// a good mark to today's file, a guard's rationale one-lined to 1,000 characters, and exits 1
    /// when the root cannot hold one.
    #[test]
    fn mark() -> Outcome {
        let root = temp("mark")?;
        let envs = [
            ("SANDMAN_MACHINE", "studio"),
            ("SANDMAN_ROOT", root.as_str()),
        ];
        for bad in [
            "not json",
            "{}",
            &format!(r#"{{"kind":"nap","session":"{ID}a"}}"#),
            r#"{"kind":"start","session":"short"}"#,
            &format!(r#"{{"kind":"guard","session":"{ID}a","rationale":7}}"#),
        ] {
            same(
                sandman(&envs, &["mark"], bad)?.get(..2).unwrap_or_default(),
                "2\n",
            )?;
        }
        same(&fs::read_dir(&root)?.count().to_string(), "0")?;
        let clock = || {
            Command::new("date")
                .args(["-u", "+%Y-%m-%dT%H:%M"])
                .output()
                .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        };
        let before = clock()?;
        let start = format!(
            r#"{{"cwd":"/w","interactive":true,"kind":"start","reason":"startup","resume":"r","session":"{ID}a","surface":"cli"}}"#
        );
        same(
            &format!(
                "{}{}",
                sandman(&envs, &["mark"], &start)?,
                sandman(
                    &envs,
                    &["mark"],
                    &format!(r#"{{"kind":"end","session":"{ID}a"}}"#)
                )?
            ),
            "0\n0\n",
        )?;
        let denied = json!({"kind": "guard", "rationale": format!("never\n  force-push {}", "x".repeat(1200)), "session": format!("{ID}a"), "tool": "Bash"});
        same(
            &format!(
                "{}{}",
                sandman(&envs, &["mark"], &denied.to_string())?,
                sandman(
                    &envs,
                    &["mark"],
                    &format!(r#"{{"kind":"guard","rationale":"ask first","session":"{ID}a"}}"#)
                )?
            ),
            "0\n0\n",
        )?;
        let after = clock()?;
        let lines = fs::read_to_string(format!(
            "{root}/ledger/{}.jsonl",
            before.get(..10).ok_or("date")?.replace('-', "/")
        ))?;
        let stamps = lines
            .lines()
            .map(|line| {
                serde_json::from_str::<Value>(line)
                    .map(|mark| mark["at"].as_str().unwrap_or_default().to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        same(
            &stamps
                .iter()
                .all(|stamp| {
                    stamp.len() == 20
                        && stamp.ends_with('Z')
                        && [&before, &after]
                            .iter()
                            .any(|minute| stamp.starts_with(minute.as_str()))
                })
                .to_string(),
            "true",
        )?;
        same(
            &lines,
            &format!(
                "{{\"at\":\"{}\",\"cwd\":\"/w\",\"interactive\":true,\"kind\":\"start\",\"machine\":\"studio\",\"reason\":\"startup\",\"resume\":\"r\",\"session\":\"{ID}a\",\"surface\":\"cli\"}}\n{{\"at\":\"{}\",\"cwd\":null,\"interactive\":null,\"kind\":\"end\",\"machine\":\"studio\",\"reason\":null,\"resume\":null,\"session\":\"{ID}a\",\"surface\":null}}\n{{\"at\":\"{}\",\"kind\":\"guard\",\"machine\":\"studio\",\"rationale\":\"never force-push {}\",\"session\":\"{ID}a\",\"tool\":\"Bash\"}}\n{{\"at\":\"{}\",\"kind\":\"guard\",\"machine\":\"studio\",\"rationale\":\"ask first\",\"session\":\"{ID}a\",\"tool\":null}}\n",
                stamps.first().ok_or("start")?,
                stamps.get(1).ok_or("end")?,
                stamps.get(2).ok_or("guard")?,
                "x".repeat(983),
                stamps.get(3).ok_or("guard")?
            ),
        )?;
        fs::write(format!("{root}/file"), "")?;
        let out = sandman(
            &[
                ("SANDMAN_MACHINE", "studio"),
                ("SANDMAN_ROOT", &format!("{root}/file")),
            ],
            &["mark"],
            &start,
        )?;
        same(out.get(..16).unwrap_or_default(), "1\nsandman mark: ")
    }

    /// The seed then the pulls of one session, through the stub: the seed's stdout for a prompt
    /// with matches is `docs/data/recall-seed.txt`, its start mark the first ledger line, its recall
    /// line the always-set; the query embedded alone, then every candidate missing from the cache
    /// in lexical order, their lines appended (the key the reference's SHA-256); the fused top ten
    /// to pplx in one request, keyed in fused order; the decision's order and the fused one fused
    /// again, a tie newer first, then the decision's order; a later pull of the session repeats
    /// nothing sent, embeds only its query; a pull's stdout under a budget is
    /// `docs/data/recall-digest.txt`, and `$SANDMAN_RECALL_CAP` caps its lines.
    #[test]
    fn recall() -> Outcome {
        let root = temp("recall")?;
        plant(&root, &CORPUS)?;
        let path = stub(&root)?;
        fs::write(
            format!("{root}/answers"),
            r#"{"answers":{"0":{"noul":0.8},"1":{"noul":0.9},"2":{"noul":0.2},"3":{"noul":0.7},"4":{"noul":0.7},"5":{"noul":0.1}},"usage":{"input_tokens":300}}"#,
        )?;
        let session = format!("{ID}a");
        let run = |args: &[&str], stdin: &str, envs: &[(&str, &str)]| {
            let out = sandman(
                &[
                    [
                        ("HOME", root.as_str()),
                        ("PATH", &path),
                        ("SANDMAN_MACHINE", "studio"),
                        ("SANDMAN_ROOT", &root),
                    ]
                    .as_slice(),
                    envs,
                ]
                .concat(),
                args,
                stdin,
            )?;
            let (head, tail) = out.split_once(" ms=").ok_or("no ms")?;
            Ok::<_, Box<dyn Error>>(format!(
                "{head} ms=N{}",
                tail.trim_start_matches(|digit: char| digit.is_ascii_digit())
            ))
        };
        let stats = |sent: u8, cut: u8| {
            format!(
                "recall: scorer=\"pplx\" embedded=true ask=true about=null ms=N tokens=300 considered=6 sent={sent} cut={cut}\n"
            )
        };
        let clocked = "add a clock to the alpha tests";
        same(
            &run(
                &["recall"],
                &format!(
                    r#"{{"session":"{session}","cwd":"/work/alpha","surface":"cli","interactive":true,"prompt":"{clocked}"}}"#
                ),
                &[],
            )?,
            &format!(
                "0\n{}{}",
                include_str!("../docs/data/recall-seed.txt"),
                stats(6, 0)
            ),
        )?;
        let query = |cue: &str| {
            format!(
                r#"{{"input":["Instruct: Given what an AI coding agent is about to do, retrieve the memory notes it needs to do it correctly\nQuery: {cue}"],"model":"qwen3-embedding:0.6b","options":{{"num_ctx":1024}},"truncate":true}}"#
            )
        };
        let clock = "/work/alpha \u{b7} Pass a clock in\nTests take time from an injected clock, never a mocked one: a mock hides the drift a real clock shows within a day, and every fixture pins its dates \u{2014} na\u{ef}ve caf\u{e9} clocks lose\nInject the clock.";
        let excerpts = [
            clock,
            "/work/alpha \u{b7} Alpha deploys need the vpn\nthe deploy host is private\nConnect first.",
            "/work/alpha \u{b7} The grammar lives in one file\nsrc/grammar.pest\nOne file.",
            "/work/beta \u{b7} Beta ties break by date\nnewer first\nNewer first.",
            "/work/beta \u{b7} Beta pins its node version\nnode 22 from mise\nmise use node 22.",
            "/work/beta \u{b7} Beta retries back off twice\na failed beta deploy retries twice, then stops\nTwice, then stop.",
        ];
        let first = fs::read_to_string(format!("{root}/embed.in"))?;
        same(
            &first,
            &format!(
                "{}\n{}\n",
                query(clocked),
                json!({"input": excerpts, "model": "qwen3-embedding:0.6b", "options": {"num_ctx": 1024u16}, "truncate": true})
            ),
        )?;
        let cache = fs::read_to_string(format!("{root}/embeddings.jsonl"))?;
        same(
            cache.lines().next().unwrap_or_default(),
            r#"{"h":"07d90899acde5df83bab6d918500f882157ee06dba25045e7698fc308431a60f","path":"-work-alpha/feedback_clock.md","v":[0.7071067811865475,0.0,0.0,0.7071067811865475]}"#,
        )?;
        let asked = [
            ("/work/alpha", "Pass a clock in \u{2014} Tests take time from an injected clock, never a mocked one: a mock hides the drift a real clock shows within a day, and every fixture pins its dates \u{2014} na\u{ef}ve caf"),
            ("/work/alpha", "Alpha deploys need the vpn \u{2014} the deploy host is private"),
            ("/work/alpha", "The grammar lives in one file \u{2014} src/grammar.pest"),
            ("/work/beta", "Beta ties break by date \u{2014} newer first"),
            ("/work/beta", "Beta pins its node version \u{2014} node 22 from mise"),
            ("/work/beta", "Beta retries back off twice \u{2014} a failed beta deploy retries twice, then stops"),
        ]
        .iter()
        .enumerate()
        .map(|(id, &(repository, memory))| format!(r#""{id}":{{"instructions":"Memory ({repository}): {memory}. Does the agent need this memory to carry out the request correctly?","type":"noul"}}"#))
        .collect::<Vec<_>>()
        .join(",");
        same(
            fs::read_to_string(format!("{root}/pplx.in"))?
                .lines()
                .next()
                .unwrap_or_default(),
            &format!(
                r#"{{"model":"pplx-decider-flash:4b","questions":{{{asked}}},"state":"{clocked}"}}"#
            ),
        )?;
        same(
            &run(
                &[
                    "recall",
                    "--cue",
                    "then the beta release",
                    "--session",
                    &session,
                ],
                "",
                &[],
            )?,
            &format!(
                "0\nrecall \u{b7} then the beta release \u{b7} 0 of 6 \u{b7} pplx\nnothing recalled\n{}",
                stats(0, 0)
            ),
        )?;
        same(
            &run(&["recall", "--cue", clocked, "--budget", "560"], "", &[])?,
            &format!(
                "0\n{}{}",
                include_str!("../docs/data/recall-digest.txt"),
                stats(3, 3)
            ),
        )?;
        same(
            &run(
                &["recall", "--cue", clocked],
                "",
                &[("SANDMAN_RECALL_CAP", "1")],
            )?,
            &format!(
                "0\nrecall \u{b7} {clocked} \u{b7} 1 of 6 \u{b7} pplx\n.99 Alpha deploys need the vpn (project \u{b7} -work-alpha/project_vpn.md) \u{2014} the deploy host is private\nread one in full: sandman show <path>\n{}",
                stats(1, 5)
            ),
        )?;
        // the cache read: every later pass embeds its query alone
        same(
            fs::read_to_string(format!("{root}/embed.in"))?
                .get(first.len()..)
                .unwrap_or_default(),
            &[
                query("then the beta release"),
                query(clocked),
                query(clocked),
                String::new(),
            ]
            .join("\n"),
        )?;
        let sent = |files: &[&str]| {
            files
                .iter()
                .map(|file| {
                    let score = [
                        ("-work-alpha/project_vpn.md", "0.99"),
                        ("-work-alpha/feedback_clock.md", "0.99"),
                        ("-work-beta/feedback_tie.md", "0.96"),
                        ("-work-alpha/reference_grammar.md", "0.95"),
                        ("-work-beta/project_node.md", "0.95"),
                        ("traces/-work-beta/feedback_retry.md", "0.92"),
                    ]
                    .iter()
                    .find(|pair| pair.0 == *file)
                    .map_or("", |pair| pair.1);
                    format!(r#"{{"file":"{file}","score":{score}}}"#)
                })
                .collect::<Vec<_>>()
                .join(",")
        };
        let (vpn, clock_file, tie, grammar, node, retry) = (
            "-work-alpha/project_vpn.md",
            "-work-alpha/feedback_clock.md",
            "-work-beta/feedback_tie.md",
            "-work-alpha/reference_grammar.md",
            "-work-beta/project_node.md",
            "traces/-work-beta/feedback_retry.md",
        );
        let line = |cue: &str, cut: u8, files: &[&str], id: &str| {
            format!(
                r#"{{"ask":true,"at":"A","considered":6,"cue":"{cue}","cut":{cut},"embedded":true,"kind":"recall","machine":"studio","ms":0,"scorer":"pplx","sent":[{}],"session":{id},"tokens":300}}"#,
                sent(files)
            )
        };
        same(
            &ledger(&root)?,
            &[
                format!(
                    r#"{{"at":"A","cwd":"/work/alpha","interactive":true,"kind":"start","machine":"studio","reason":null,"resume":null,"session":"{session}","surface":"cli"}}"#
                ),
                format!(
                    r#"{{"always":["common/project_pinned.md","common/user_short.md","traces/common/user_terse.md"],"ask":true,"at":"A","considered":6,"cue":"{clocked}","cut":0,"embedded":true,"kind":"recall","machine":"studio","ms":0,"scorer":"pplx","seed":true,"sent":[{}],"session":"{session}","tokens":300}}"#,
                    sent(&[vpn, clock_file, tie, grammar, node, retry])
                ),
                line("then the beta release", 0, &[], &format!("\"{session}\"")),
                line(clocked, 3, &[vpn, clock_file, tie], "null"),
                line(clocked, 5, &[vpn], "null"),
            ]
            .join("\n"),
        )
    }

    /// The seed never fails the session's first prompt: with no prompt, or one of steering words
    /// alone, it asks no model and prints no ranked part; when every transport outlasts the seed's
    /// bound it prints the lexical rank, its top five, and records why; a pipeline session gets
    /// nothing. Only an interactive session's start mark is written, first; a headless one is never
    /// marked, so its `take` writes nothing.
    #[test]
    fn seed() -> Outcome {
        let root = temp("seed")?;
        plant(&root, &CORPUS)?;
        let path = stub(&root)?;
        for name in ["embed", "pplx", "kev"] {
            fs::write(format!("{root}/{name}.fail"), "sleep 5\n")?;
        }
        // the sample seed's instructions and always-set, for another session
        let opening = |letter: &str| {
            let sample = include_str!("../docs/data/recall-seed.txt");
            sample
                .get(..sample.find("recall \u{b7}").unwrap_or_default())
                .unwrap_or_default()
                .replace(&format!("{ID}a"), &format!("{ID}{letter}"))
        };
        let foot = "read one in full: sandman show <path>\n";
        let lexical = "recall \u{b7} fix it \u{b7} 5 of 6 \u{b7} lexical: embed and pplx and kev and jev failed\n.00 ~Beta retries back off twice (feedback \u{b7} traces/-work-beta/feedback_retry.md) \u{2014} a failed beta deploy retries twice, then stops\n.00 Alpha deploys need the vpn (project \u{b7} -work-alpha/project_vpn.md) \u{2014} the deploy host is private\n.00 Beta ties break by date (feedback \u{b7} -work-beta/feedback_tie.md) \u{2014} newer first\n.00 Pass a clock in (feedback \u{b7} -work-alpha/feedback_clock.md) \u{2014} Tests take time from an injected clock, never a mocked one: a mock hides the drift a real clock shows within a day, and every fixture pins its dates \u{2014} na\u{ef}ve caf\n.00 The grammar lives in one file (reference \u{b7} -work-alpha/reference_grammar.md) \u{2014} src/grammar.pest\n";
        let error = "embed: timed out after 300 ms; pplx: timed out after 300 ms; kev: timed out after 300 ms; jev: no API key: set TYPESAFE_API_KEY or write it to ~/.config/typesafe/api_key";
        let mut want = Vec::new();
        for (letter, prompt, ranked, sent) in [
            ("b", "", "", ""),
            ("c", "hello there", "", ""),
            (
                "d",
                "fix it",
                lexical,
                r#"{"file":"traces/-work-beta/feedback_retry.md","score":0.0},{"file":"-work-alpha/project_vpn.md","score":0.0},{"file":"-work-beta/feedback_tie.md","score":0.0},{"file":"-work-alpha/feedback_clock.md","score":0.0},{"file":"-work-alpha/reference_grammar.md","score":0.0}"#,
            ),
        ] {
            let session = format!("{ID}{letter}");
            let ranks = !ranked.is_empty();
            let out = sandman(
                &[
                    ("HOME", root.as_str()),
                    ("PATH", &path),
                    ("SANDMAN_MACHINE", "studio"),
                    ("SANDMAN_ROOT", &root),
                    ("SANDMAN_SEED_MS", "300"),
                ],
                &["recall"],
                &format!(r#"{{"session":"{session}","interactive":{ranks},"prompt":"{prompt}"}}"#),
            )?;
            let (head, tail) = out.split_once(" ms=").ok_or("no ms")?;
            let stderr = if ranks {
                format!("sandman recall: {error}\n")
            } else {
                String::new()
            };
            same(
                &format!(
                    "{head} ms=N{}",
                    tail.trim_start_matches(|digit: char| digit.is_ascii_digit())
                ),
                &format!(
                    "0\n{}{ranked}{foot}{stderr}recall: scorer=null embedded=false ask={ranks} about=null ms=N tokens=0 considered=6 sent={} cut={}\n",
                    opening(letter),
                    u8::from(ranks) * 5,
                    u8::from(ranks)
                ),
            )?;
            want.extend(ranks.then(|| format!(
                r#"{{"at":"A","cwd":null,"interactive":true,"kind":"start","machine":"studio","reason":null,"resume":null,"session":"{session}","surface":null}}"#
            )));
            want.push(format!(
                r#"{{"always":["common/project_pinned.md","common/user_short.md","traces/common/user_terse.md"],"ask":{ranks},"at":"A","considered":6,"cue":"{prompt}","cut":{},"embedded":false,{}"kind":"recall","machine":"studio","ms":0,"scorer":null,"seed":true,"sent":[{sent}],"session":"{session}","tokens":0}}"#,
                u8::from(ranks),
                if ranks {
                    format!(r#""error":"{error}","#)
                } else {
                    String::new()
                }
            ));
        }
        same(
            &sandman(
                &[
                    ("CLAUDE_MEMORY_PIPELINE", "1"),
                    ("SANDMAN_MACHINE", "studio"),
                    ("SANDMAN_ROOT", &root),
                ],
                &["recall"],
                &format!(r#"{{"session":"{ID}f","prompt":"x"}}"#),
            )?,
            "0\n",
        )?;
        same(
            &sandman(
                &[("SANDMAN_MACHINE", "studio"), ("SANDMAN_ROOT", &root)],
                &["take"],
                &format!(r#"{{"session":"{ID}b"}}"#),
            )?,
            "0\n",
        )?;
        same(&ledger(&root)?, &want.join("\n"))?;
        // invoked as sandman-next, every command it gives the agent names sandman-next
        let next = ran(
            command(&[("SANDMAN_ROOT", &format!("{root}/next"))], &["recall"])
                .arg0(format!("{root}/bin/sandman-next")),
            &format!(r#"{{"session":"{ID}a"}}"#),
        )?;
        same(
            &format!(
                "{} {}",
                next.matches("`sandman-next ").count(),
                next.lines().nth(4).unwrap_or_default()
            ),
            "5 read one in full: sandman-next show <path>",
        )
    }

    /// `recall` refuses a seed's bad stdin and a bad session id (exit 2). Each stage degrades in
    /// the same bound, never an error to the agent, every failure on stderr and in the ledger: the
    /// embeddings down, the decision on the lexical order; pplx down, Kev; Kev down too, Jev with
    /// the key as curl config on its stdin (from the key file, then the environment over it), the
    /// request on fd 3, its argv and environment never holding the key; Jev out of credit, held down
    /// for 6 hours, 60 s past any other failure, skipped while it holds; no decision model, the
    /// hybrid cut; nothing at all, the lexical rank. `$SANDMAN_SCORERS` orders them, an unknown
    /// name skipped, none the hybrid.
    #[test]
    fn recall_fail() -> Outcome {
        let root = temp("recall-fail")?;
        plant(&root, &CORPUS)?;
        let path = stub(&root)?;
        fs::write(
            format!("{root}/answers"),
            r#"{"answers":{"0":{"noul":0.9},"1":{"noul":0.3},"2":{"noul":0.8},"3":{"noul":0.45},"4":{"noul":0.2},"5":{"noul":0.1}},"usage":{"input_tokens":300}}"#,
        )?;
        let envs = |more: &[(&'static str, &str)]| {
            let mut all = vec![
                ("HOME", root.clone()),
                ("PATH", path.clone()),
                ("SANDMAN_MACHINE", "studio".to_owned()),
                ("SANDMAN_RECALL_MS", "2000".to_owned()),
                ("SANDMAN_ROOT", root.clone()),
            ];
            all.extend(more.iter().map(|pair| (pair.0, pair.1.to_owned())));
            all
        };
        let pull = |given: &[(&str, String)], args: &[&str]| {
            let pairs: Vec<(&str, &str)> =
                given.iter().map(|pair| (pair.0, pair.1.as_str())).collect();
            let out = sandman(&pairs, args, "")?;
            // the stdout and stderr, the stats cut at their time
            Ok::<_, Box<dyn Error>>(
                out.split_once(" ms=")
                    .map_or_else(|| out.clone(), |cut| cut.0.to_owned()),
            )
        };
        let read = |file: &str| fs::read_to_string(format!("{root}/{file}")).unwrap_or_default();
        let fail = |name: &str, script: &str| fs::write(format!("{root}/{name}.fail"), script);
        let mend = |name: &str| fs::remove_file(format!("{root}/{name}.fail"));
        let clocked = ["recall", "--cue", "add a clock to the alpha tests"];
        let head = |out: &str| out.lines().nth(1).unwrap_or_default().to_owned();
        same(
            &pull(&envs(&[]), &["recall"])?,
            "2\nsandman recall: stdin is not {\"session\":\"<uuid>\",...}\n",
        )?;
        same(
            &pull(&envs(&[]), &["recall", "--cue", "x", "--session", "nope"])?,
            "2\nsandman: --session takes a session id\n",
        )?;
        // the embeddings down: the decision over the lexical order
        fail("embed", "echo 'curl: (7) Failed to connect' >&2; exit 7\n")?;
        same(
            &pull(&envs(&[]), &clocked)?,
            "0\nrecall \u{b7} add a clock to the alpha tests \u{b7} 6 of 6 \u{b7} pplx: embed failed\n1.00 Pass a clock in (feedback \u{b7} -work-alpha/feedback_clock.md) \u{2014} Tests take time from an injected clock, never a mocked one: a mock hides the drift a real clock shows within a day, and every fixture pins its dates \u{2014} na\u{ef}ve caf\n.98 The grammar lives in one file (reference \u{b7} -work-alpha/reference_grammar.md) \u{2014} src/grammar.pest\n.97 Alpha deploys need the vpn (project \u{b7} -work-alpha/project_vpn.md) \u{2014} the deploy host is private\n.96 Beta ties break by date (feedback \u{b7} -work-beta/feedback_tie.md) \u{2014} newer first\n.94 Beta pins its node version (project \u{b7} -work-beta/project_node.md) \u{2014} node 22 from mise\n.92 ~Beta retries back off twice (feedback \u{b7} traces/-work-beta/feedback_retry.md) \u{2014} a failed beta deploy retries twice, then stops\nread one in full: sandman show <path>\nsandman recall: embed: exit status: 7: curl: (7) Failed to connect\nrecall: scorer=\"pplx\" embedded=false ask=true about=null",
        )?;
        same(
            &read("embed.argv"),
            "-q\n-sS\n--fail-with-body\n--max-time\n2\n-H\nContent-Type: application/json\n--data-binary\n@-\nhttp://ollama.test/api/embed\n",
        )?;
        same(
            &read("pplx.argv"),
            "-q\n-sS\n--fail-with-body\n--max-time\n2\n-H\nContent-Type: application/json\n--data-binary\n@-\nhttp://ollama.test/v1/systemone\n",
        )?;
        mend("embed")?;
        // pplx down: Kev, the same request but its model
        fail("pplx", "exit 7\n")?;
        same(
            &head(&pull(&envs(&[]), &clocked)?),
            "recall \u{b7} add a clock to the alpha tests \u{b7} 6 of 6 \u{b7} kev: pplx failed",
        )?;
        same(
            read("kev.in").lines().last().unwrap_or_default(),
            &read("pplx.in")
                .lines()
                .last()
                .unwrap_or_default()
                .replace("pplx-decider-flash:4b", "kev-latest"),
        )?;
        same(
            &format!("{}{}", read("kev.argv"), read("kev.env")),
            "-q\n-sS\n--fail-with-body\n--max-time\n2\n-H\nContent-Type: application/json\n--data-binary\n@-\nhttp://127.0.0.1:1703/v1/systemone\n[]",
        )?;
        // Kev down too: Jev, the key never in an argv or the environment curl sees
        fail("kev", "exit 7\n")?;
        plant(&root, &[(".config/typesafe/api_key", "file-key\n")])?;
        for (key, want) in [(None, "file-key"), (Some("env-key"), "env-key")] {
            let keyed = envs(
                &key.map(|set| ("TYPESAFE_API_KEY", set))
                    .into_iter()
                    .collect::<Vec<_>>(),
            );
            same(
                &head(&pull(&keyed, &clocked)?),
                "recall \u{b7} add a clock to the alpha tests \u{b7} 6 of 6 \u{b7} jev: pplx and kev failed",
            )?;
            same(
                &read("jev.config"),
                &format!("header = \"Authorization: Bearer {want}\"\n"),
            )?;
            same(
                &format!("{} {}", read("jev.argv").contains(want), read("jev.env")),
                "false []",
            )?;
        }
        same(
            &read("jev.argv"),
            "-q\n-K\n-\n-sS\n--fail-with-body\n--max-time\n2\n-H\nContent-Type: application/json\n--data-binary\n@/dev/fd/3\nhttps://api.typesafe.ai/v1/systemone\n",
        )?;
        let now = || {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_millis())
        };
        let downed = |want: u128, error: &str| {
            let down: Value = serde_json::from_str(&read("jev-down.json"))?;
            let left = u128::from(down["until"].as_u64().ok_or("until")?).saturating_sub(now());
            same(
                &format!(
                    "{} {}",
                    down["error"],
                    left > want.saturating_sub(5000) && left <= want
                ),
                &format!("{error} true"),
            )
        };
        // out of credit: the hybrid cut, Jev held down for 6 hours, then skipped
        fail(
            "jev",
            "echo '{\"error\":{\"type\":\"billing_error\"}}'; echo 'curl: (22) The requested URL returned error: 402' >&2; exit 22\n",
        )?;
        same(
            &pull(&envs(&[]), &clocked)?,
            "0\nrecall \u{b7} add a clock to the alpha tests \u{b7} 6 of 6 \u{b7} hybrid, no decision model: pplx and kev and jev failed\n1.00 Pass a clock in (feedback \u{b7} -work-alpha/feedback_clock.md) \u{2014} Tests take time from an injected clock, never a mocked one: a mock hides the drift a real clock shows within a day, and every fixture pins its dates \u{2014} na\u{ef}ve caf\n.98 Alpha deploys need the vpn (project \u{b7} -work-alpha/project_vpn.md) \u{2014} the deploy host is private\n.98 The grammar lives in one file (reference \u{b7} -work-alpha/reference_grammar.md) \u{2014} src/grammar.pest\n.95 Beta ties break by date (feedback \u{b7} -work-beta/feedback_tie.md) \u{2014} newer first\n.94 Beta pins its node version (project \u{b7} -work-beta/project_node.md) \u{2014} node 22 from mise\n.92 ~Beta retries back off twice (feedback \u{b7} traces/-work-beta/feedback_retry.md) \u{2014} a failed beta deploy retries twice, then stops\nread one in full: sandman show <path>\nsandman recall: pplx: exit status: 7:; kev: exit status: 7:; jev: exit status: 22: curl: (22) The requested URL returned error: 402\nrecall: scorer=null embedded=true ask=true about=null",
        )?;
        downed(
            21_600_000,
            "\"exit status: 22: curl: (22) The requested URL returned error: 402\"",
        )?;
        let asked = read("jev.in").lines().count();
        same(
            &head(&pull(&envs(&[]), &clocked)?),
            "recall \u{b7} add a clock to the alpha tests \u{b7} 6 of 6 \u{b7} hybrid, no decision model: pplx and kev failed",
        )?;
        same(
            &read("jev.in").lines().count().to_string(),
            &asked.to_string(),
        )?;
        // any other failure: down for 60 s
        fs::remove_file(format!("{root}/jev-down.json"))?;
        fail("jev", "exit 7\n")?;
        drop(pull(&envs(&[]), &clocked)?);
        downed(60_000, "\"exit status: 7:\"")?;
        fs::remove_file(format!("{root}/jev-down.json"))?;
        fs::remove_file(format!("{root}/.config/typesafe/api_key"))?;
        // nothing answers: the lexical rank, its top five
        fail("embed", "exec echo '{\"embeddings\":[]}'\n")?;
        same(
            &pull(&envs(&[]), &clocked)?,
            "0\nrecall \u{b7} add a clock to the alpha tests \u{b7} 5 of 6 \u{b7} lexical: embed and pplx and kev and jev failed\n1.00 Pass a clock in (feedback \u{b7} -work-alpha/feedback_clock.md) \u{2014} Tests take time from an injected clock, never a mocked one: a mock hides the drift a real clock shows within a day, and every fixture pins its dates \u{2014} na\u{ef}ve caf\n.25 Alpha deploys need the vpn (project \u{b7} -work-alpha/project_vpn.md) \u{2014} the deploy host is private\n.19 The grammar lives in one file (reference \u{b7} -work-alpha/reference_grammar.md) \u{2014} src/grammar.pest\n.00 ~Beta retries back off twice (feedback \u{b7} traces/-work-beta/feedback_retry.md) \u{2014} a failed beta deploy retries twice, then stops\n.00 Beta ties break by date (feedback \u{b7} -work-beta/feedback_tie.md) \u{2014} newer first\nread one in full: sandman show <path>\nsandman recall: embed: reply: not one embedding per text; pplx: exit status: 7:; kev: exit status: 7:; jev: no API key: set TYPESAFE_API_KEY or write it to ~/.config/typesafe/api_key\nrecall: scorer=null embedded=false ask=true about=null",
        )?;
        mend("embed")?;
        // the order is one setting: none, the hybrid; an unknown name skipped
        same(
            &head(&pull(&envs(&[("SANDMAN_SCORERS", "")]), &clocked)?),
            "recall \u{b7} add a clock to the alpha tests \u{b7} 6 of 6 \u{b7} hybrid, no decision model",
        )?;
        fail("kev", "exec echo '{}'\n")?;
        same(
            &pull(&envs(&[("SANDMAN_SCORERS", "x,kev")]), &clocked)?
                .lines()
                .skip(7)
                .collect::<Vec<_>>()
                .join("\n"),
            ".92 ~Beta retries back off twice (feedback \u{b7} traces/-work-beta/feedback_retry.md) \u{2014} a failed beta deploy retries twice, then stops\nread one in full: sandman show <path>\nsandman recall: kev: reply: no noul for question 0\nrecall: scorer=null embedded=true ask=true about=null",
        )?;
        let errors: Vec<String> = ledger(&root)?
            .lines()
            .map(|text| {
                let line: Value = serde_json::from_str(text)?;
                Ok(format!(
                    "{} {} {}",
                    line["scorer"], line["embedded"], line["error"]
                ))
            })
            .collect::<Result<_, Box<dyn Error>>>()?;
        same(
            &errors.join("\n"),
            "\"pplx\" false \"embed: exit status: 7: curl: (7) Failed to connect\"\n\"kev\" true \"pplx: exit status: 7:\"\n\"jev\" true \"pplx: exit status: 7:; kev: exit status: 7:\"\n\"jev\" true \"pplx: exit status: 7:; kev: exit status: 7:\"\nnull true \"pplx: exit status: 7:; kev: exit status: 7:; jev: exit status: 22: curl: (22) The requested URL returned error: 402\"\nnull true \"pplx: exit status: 7:; kev: exit status: 7:\"\nnull true \"pplx: exit status: 7:; kev: exit status: 7:; jev: exit status: 7:\"\nnull false \"embed: reply: not one embedding per text; pplx: exit status: 7:; kev: exit status: 7:; jev: no API key: set TYPESAFE_API_KEY or write it to ~/.config/typesafe/api_key\"\nnull true null\nnull true \"kev: reply: no noul for question 0\"",
        )
    }

    /// A candidate's repository is its bank's label over a temp home and grove root: home, the
    /// grove root, a repository and a worktree bank folded into it, an owner, a repository gone
    /// from disk, a job's scratch dir, a dir under home and one matching nothing. `--about`, and a
    /// seed's cwd inside a grove repository (not the grove root), lead the state; a repository the
    /// cue names is the about without leading it. With the embeddings down the decision sees the
    /// BM25 order: 4 in the about's bank, 1 in home's and the grove root's, none for a memory that
    /// claims its own importance, a tie in reader order. A cue of steering words or single
    /// characters asks no model and recalls nothing, for a pull and a seed.
    #[test]
    fn recall_repository() -> Outcome {
        let home = temp("recall-repository")?;
        let grove = format!("{home}/.grove/code");
        fs::create_dir_all(format!("{grove}/own/app"))?;
        let key = |path: &str| -> String {
            path.chars()
                .map(|letter| {
                    if letter.is_ascii_alphanumeric() {
                        letter
                    } else {
                        '-'
                    }
                })
                .collect()
        };
        let banks = [
            (home.clone(), "home (~)"),
            (grove.clone(), "grove root (every repository)"),
            (format!("{grove}/own/app"), "own/app"),
            (format!("{grove}/own/app/rewrite"), "own/app"),
            (format!("{grove}/own"), "own (owner: all its repositories)"),
            (format!("{grove}/own/old-tool"), "own/old-tool"),
            (format!("{home}/.claude/jobs/a1"), "scratch dir"),
            (format!("{home}/notes"), "~/notes"),
            ("/work/alpha".to_owned(), "/work/alpha"),
        ];
        for (index, bank) in banks.iter().enumerate() {
            plant(
                &home,
                &[(
                    &format!("memories/{}/feedback_m{index}.md", key(&bank.0)),
                    &format!("---\nname: m{index}\ndescription: d\ntype: feedback\n---\n\nm.\n"),
                )],
            )?;
        }
        let path = stub(&home)?;
        fs::write(format!("{home}/embed.fail"), "exit 7\n")?;
        fs::write(
            format!("{home}/answers"),
            format!(
                r#"{{"answers":{{{}}}}}"#,
                (0u8..10)
                    .map(|id| format!(r#""{id}":{{"noul":0.5}}"#))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        )?;
        let run = |args: &[&str], stdin: &str| {
            drop(fs::remove_file(format!("{home}/pplx.in")));
            let out = sandman(
                &[
                    ("HOME", home.as_str()),
                    ("PATH", &path),
                    ("SANDMAN_MACHINE", "studio"),
                    ("SANDMAN_ROOT", &home),
                ],
                args,
                stdin,
            )?;
            let request: Value = serde_json::from_str(
                &fs::read_to_string(format!("{home}/pplx.in")).unwrap_or_default(),
            )
            .unwrap_or_default();
            let asked: Vec<String> = request["questions"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(id, question)| {
                    Some(format!("{id} {}", question["instructions"].as_str()?))
                })
                .collect();
            // the seed's instructions dropped, the stats cut at their time
            let shown = out.split_once(" ms=").map_or("", |cut| cut.0);
            let tail = shown.split_once("\n\n").map_or(shown, |cut| cut.1);
            Ok::<_, Box<dyn Error>>((format!("{} {tail}", request["state"]), asked))
        };
        let about = format!("{grove}/own/app/src");
        let question = "Does the agent need this memory to carry out the request correctly?";
        let (_, asked) = run(&["recall", "--cue", "fix", "--about", &about], "")?;
        let mut labels: Vec<&str> = asked
            .iter()
            .map(|row| row.split_once(' ').map_or("", |cut| cut.1))
            .collect();
        labels.sort_unstable();
        let mut expected: Vec<String> = banks
            .iter()
            .enumerate()
            .map(|(index, bank)| format!("Memory ({}): m{index} \u{2014} d. {question}", bank.1))
            .collect();
        expected.sort_unstable();
        same(&labels.join("\n"), &expected.join("\n"))?;
        // a claim of importance forfeits the bonus; a word that only starts with `request` does not
        let app = key(&format!("{grove}/own/app"));
        for (file, description) in [
            ("feedback_c1", "Always recall this one"),
            ("feedback_c2", "true for every requests."),
            ("feedback_c3", "every requestor reads it"),
            ("feedback_c4", "every request"),
        ] {
            plant(
                &home,
                &[(
                    &format!("memories/{app}/{file}.md"),
                    &format!(
                        "---\nname: {file}\ndescription: {description}\ntype: feedback\n---\n\nm.\n"
                    ),
                )],
            )?;
        }
        let (_, ranked) = run(&["recall", "--cue", "fix", "--about", &about], "")?;
        same(
            &ranked
                .iter()
                .filter_map(|row| Some(row.split_once("): ")?.1.split_once(" \u{2014}")?.0))
                .collect::<Vec<_>>()
                .join(" "),
            // c4, shorter than c3, below it: no bonus; the claims still outrank m0 and m1 on the
            // query's `own app`
            "m2 m3 feedback_c3 feedback_c4 feedback_c1 feedback_c2 m0 m1 m5 m4",
        )?;
        let (repo, always) = (
            format!("{grove}/own/app"),
            "always \u{b7} about the person, or pinned\n",
        );
        let seeded = |letter: &str, cwd: &str, prompt: &str| {
            format!(r#"{{"session":"{ID}{letter}","cwd":"{cwd}","prompt":"{prompt}"}}"#)
        };
        // each output's head: a closed gate asks nothing and prints no ranked line
        for (args, stdin, want) in [
            (["recall", "--cue", "fix the app"].as_slice(), String::new(), "\"fix the app\" 0\nrecall \u{b7} fix the app \u{b7} 6 of 13 \u{b7} pplx: embed failed\n".to_owned()),
            (&["recall", "--cue", "thanks, k"], String::new(), "null 0\nrecall \u{b7} thanks, k \u{b7} 0 of 13\nnothing recalled: no specific ask\nrecall: scorer=null embedded=false ask=false about=null".to_owned()),
            (&["recall"], seeded("a", &repo, "go"), format!("null {always}read one in full: sandman show <path>\nrecall: scorer=null embedded=false ask=false about=\"own/app\"")),
            (&["recall"], seeded("b", &repo, "fix"), format!("\"repository: own/app\\nfix\" {always}recall \u{b7} fix \u{b7} 6 of 13 \u{b7} pplx: embed failed\n")),
            (&["recall"], seeded("c", &grove, "fix"), format!("\"fix\" {always}recall \u{b7} fix \u{b7} 6 of 13 \u{b7} pplx: embed failed\n")),
        ] {
            let (got, _) = run(args, &stdin)?;
            same(got.get(..want.len()).unwrap_or(&got), &want)?;
        }
        let recalls: Vec<String> = ledger(&home)?
            .lines()
            .filter_map(|text| {
                let line: Value = serde_json::from_str(text).ok()?;
                (line["kind"] == "recall").then(|| {
                    format!(
                        "{} {} {}",
                        line["about"],
                        line["ask"],
                        line["sent"].as_array().map_or(0, Vec::len)
                    )
                })
            })
            .collect();
        same(
            &recalls.join("|"),
            "\"own/app\" true 6|\"own/app\" true 6|\"own/app\" true 6|null false 0|\"own/app\" false 0|\"own/app\" true 6|null true 6",
        )
    }

    /// The embedding cache: a pull embeds at most `$SANDMAN_RECALL_EMBED` candidates it lacks,
    /// the lexically best, appending them; a failure there is an error beside the cosine rank of
    /// the rest. Serve's tick rewrites the cache to its live lines once past twice their size, a
    /// hash's last line winning, then embeds what it lacks, `$SANDMAN_EMBED_TICK` a tick, each
    /// batch or failure a `serve.log` line; a pull after it embeds only its query.
    #[test]
    fn embed() -> Outcome {
        let root = temp("embed")?;
        plant(&root, &CORPUS)?;
        let path = stub(&root)?;
        fs::write(
            format!("{root}/answers"),
            r#"{"answers":{"0":{"noul":0.9},"1":{"noul":0.3},"2":{"noul":0.8},"3":{"noul":0.45},"4":{"noul":0.2},"5":{"noul":0.1}}}"#,
        )?;
        let envs = [
            ("HOME", root.as_str()),
            ("PATH", &path),
            ("SANDMAN_MACHINE", "studio"),
            ("SANDMAN_ROOT", &root),
        ];
        let pull = |more: &[(&str, &str)]| {
            sandman(
                &[envs.as_slice(), more].concat(),
                &["recall", "--cue", "add a clock to the alpha tests"],
                "",
            )
        };
        let (cache, fail) = (
            format!("{root}/embeddings.jsonl"),
            format!("{root}/embed.fail"),
        );
        let read = |file: &str| fs::read_to_string(file).unwrap_or_default();
        let held = || {
            read(&cache)
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .map(|line| format!("{} {}", line["path"], line["v"]))
                .collect::<Vec<_>>()
                .join("\n")
        };
        drop(pull(&[("SANDMAN_RECALL_EMBED", "2")])?);
        same(
            &held(),
            "\"-work-alpha/feedback_clock.md\" [0.7071067811865475,0.0,0.0,0.7071067811865475]\n\"-work-alpha/project_vpn.md\" [0.0,0.7071067811865475,0.0,0.7071067811865475]",
        )?;
        // the query embeds, the candidates do not
        fs::write(&fail, "case $body in *Instruct*) ;; *) exit 7;; esac\n")?;
        let out = pull(&[])?;
        same(
            &out.lines()
                .filter(|line| line.contains("embed"))
                .map(|line| line.split(" ms=").next().unwrap_or_default())
                .collect::<Vec<_>>()
                .join("\n"),
            "recall \u{b7} add a clock to the alpha tests \u{b7} 6 of 6 \u{b7} pplx: embed failed\nsandman recall: embed: exit status: 7:\nrecall: scorer=\"pplx\" embedded=true ask=true about=null",
        )?;
        fs::remove_file(&fail)?;
        // a stale line per byte of the live ones, an earlier line for a live hash
        let live = read(&cache);
        let first: Value = serde_json::from_str(live.lines().next().unwrap_or_default())?;
        let stale = format!(
            "{}\n",
            json!({"h": "gone", "path": "gone.md", "v": [1.0f64]})
        );
        fs::write(
            &cache,
            format!(
                "{}\n{live}{}",
                json!({"h": first["h"], "path": first["path"], "v": [9.0f64]}),
                stale.repeat(live.len().div_euclid(stale.len()).saturating_add(1))
            ),
        )?;
        fs::write(&fail, "exit 7\n")?;
        let log = format!("{root}/run/serve.log");
        let (mut daemon, _) = serve(
            &root,
            &[
                ("HOME", root.as_str()),
                ("PATH", &path),
                ("SANDMAN_EMBED_TICK", "3"),
                ("SANDMAN_TICK_MS", "50"),
            ],
        )?;
        until(|| read(&log).contains(r#""event":"embed","reason""#))?;
        fs::remove_file(&fail)?;
        until(|| held().lines().count() == 6)?;
        // TERM, so the daemon's coverage counts
        same(
            &format!(
                "{} {:?}",
                kill("-TERM", &daemon.0.id().to_string())?,
                daemon.0.wait()?.code()
            ),
            "true Some(0)",
        )?;
        same(
            &held(),
            "\"-work-alpha/feedback_clock.md\" [0.7071067811865475,0.0,0.0,0.7071067811865475]\n\"-work-alpha/project_vpn.md\" [0.0,0.7071067811865475,0.0,0.7071067811865475]\n\"-work-alpha/reference_grammar.md\" [0.0,0.0,0.0,1.0]\n\"-work-beta/feedback_tie.md\" [0.0,0.0,0.7071067811865475,0.7071067811865475]\n\"-work-beta/project_node.md\" [0.0,0.0,0.7071067811865475,0.7071067811865475]\n\"traces/-work-beta/feedback_retry.md\" [0.0,0.5773502691896258,0.5773502691896258,0.5773502691896258]",
        )?;
        let batches: Vec<Value> = read(&log)
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|line| line["event"] == "embed")
            .map(|mut line| {
                line["at"] = "A".into();
                line
            })
            .collect();
        same(
            &format!(
                "{} {}",
                batches.first().map(Value::to_string).unwrap_or_default(),
                batches
                    .iter()
                    .filter(|line| line["embedded"].is_number())
                    .map(Value::to_string)
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            "{\"at\":\"A\",\"event\":\"embed\",\"reason\":\"exit status: 7: \"} {\"at\":\"A\",\"embedded\":3,\"event\":\"embed\"} {\"at\":\"A\",\"embedded\":1,\"event\":\"embed\"}",
        )?;
        // a cosine whose sum needs Neumaier's compensation, as Python 3.12's `sum`
        let asked = read(&format!("{root}/embed.in")).lines().count();
        drop(sandman(
            &envs,
            &["recall", "--cue", "deploy the beta clock"],
            "",
        )?);
        same(
            &read(&format!("{root}/embed.in"))
                .lines()
                .count()
                .saturating_sub(asked)
                .to_string(),
            "1",
        )
    }

    /// `remember` writes one trace at once and prints how to read it back: defaults from the body,
    /// every option, homed by `--about`'s directory (relative resolved), else the cwd, else home
    /// (a cwd gone); `--about` naming no directory is kept but homes by the cwd. Each refusal is exit
    /// 2 with nothing written, a failed write exit 1.
    #[test]
    fn remember() -> Outcome {
        let root = fs::canonicalize(temp("remember")?)?
            .to_str()
            .ok_or("root")?
            .to_owned();
        let (work, home) = (format!("{root}/work"), format!("{root}/home"));
        fs::create_dir_all(format!("{work}/repo"))?;
        let envs = [
            ("HOME", home.as_str()),
            ("SANDMAN_MACHINE", "studio"),
            ("SANDMAN_ROOT", &root),
        ];
        let key = |dir: &str| dir.replace(|letter: char| !letter.is_ascii_alphanumeric(), "-");
        let session = format!("{ID}a");
        let missing = format!("{root}/missing");
        let mut out = String::new();
        for args in [
            &[
                "remember",
                "sample: the person says pass a clock in, never a mock\n  Why: a mock hid a bug ",
            ][..],
            &[
                "remember",
                "sample: terse",
                "--type",
                "user",
                "--name",
                "Sample: terse commits",
                "--description",
                "sample: one line",
                "--about",
                "repo",
                "--session",
                &session,
            ],
            &["remember", "sample: elsewhere", "--about", &missing],
            &["remember"],
            &["remember", " \n "],
            &["remember", "x", "--type", "opinion"],
            &["remember", "x", "--session", "nope"],
        ] {
            out.push_str(&ran(command(&envs, args).current_dir(&work), "")?);
        }
        // a cwd gone homes by $HOME
        out.push_str(&ran(
            Command::new("sh")
                .args(["-c", r#"mkdir gone && cd gone && rmdir ../gone && exec "$0" remember "sample: homeless""#, env!("CARGO_BIN_EXE_sandman")])
                .envs(envs)
                .current_dir(&work),
            "",
        )?);
        plant(&root, &[("blocked/traces", "")])?;
        out.push_str(&sandman(
            &[("SANDMAN_ROOT", &format!("{root}/blocked"))],
            &["remember", "x"],
            "",
        )?);
        let refused = "2\nsandman remember: takes a non-empty \"<body>\" first and --type user|feedback|project|reference, or knowledge|playbook with --topic <slug>\n";
        let files = [
            (
                key(&work),
                "feedback_sample_the_person_says_pass_a_clock_in.md",
            ),
            (key(&format!("{work}/repo")), "user_sample_terse_commits.md"),
            (key(&work), "feedback_sample_elsewhere.md"),
            (key(&home), "feedback_sample_homeless.md"),
        ]
        .map(|(bank, file)| format!("traces/{bank}/{file}"));
        let shown = |at: usize| {
            format!(
                "0\nsandman show {}\n",
                files.get(at).map_or("", String::as_str)
            )
        };
        same(
            &out,
            &[
                shown(0),
                shown(1),
                shown(2),
                refused.to_owned(),
                refused.to_owned(),
                refused.to_owned(),
                "2\nsandman: --session takes a session id\n".to_owned(),
                shown(3),
                "1\nsandman remember: Not a directory (os error 20)\n".to_owned(),
            ]
            .concat(),
        )?;
        // the four traces and nothing else
        let found = Command::new("find")
            .args(["traces", "-type", "f"])
            .current_dir(&root)
            .output()?;
        same(
            &String::from_utf8(found.stdout)?.lines().count().to_string(),
            "4",
        )?;
        // each file's own `created`: under load the four writes can straddle a second
        let mut written = String::new();
        for file in &files {
            let held = fs::read_to_string(format!("{root}/{file}"))?;
            let now = held
                .lines()
                .find_map(|line| line.strip_prefix("created: "))
                .unwrap_or_default();
            written.push_str(&masked(&held.replace(now, "<now>")));
        }
        // name, description and type; what follows `created`; the source's session (none: the
        // origin is `hand`); the body
        let note = |head: [&str; 3], rest: &str, id: &str, body: &str| {
            format!(
                "---\nid: <id>\nname: {}\ndescription: {}\ntype: {}\ncreated: <now>\n{rest}{}source: remember <now> \u{b7} {id}\n---\n\n{body}\n",
                head[0],
                head[1],
                head[2],
                if id == "-" { "origin: hand\n" } else { "" }
            )
        };
        let (elsewhere, homeless) = ("sample: elsewhere", "sample: homeless");
        same(
            &written,
            &[
                note(["sample: the person says pass a clock in,", "sample: the person says pass a clock in, never a mock", "feedback"], "", "-", "sample: the person says pass a clock in, never a mock\n  Why: a mock hid a bug"),
                note(["Sample: terse commits", "sample: one line", "user"], &format!("about: {work}/repo\nsession: {session}\n"), &session, "sample: terse"),
                note([elsewhere, elsewhere, "feedback"], &format!("about: {missing}\n"), "-", elsewhere),
                note([homeless, homeless, "feedback"], "", "-", homeless),
            ]
            .concat(),
        )?;
        same(
            &ledger(&root)?,
            &files
                .iter()
                .zip(["feedback", "user", "feedback", "feedback"])
                .map(|(file, kind)| {
                    let id = if kind == "user" { format!("\"{session}\"") } else { "null".to_owned() };
                    format!(r#"{{"at":"A","file":"{file}","kind":"remember","machine":"studio","session":{id},"type":"{kind}"}}"#)
                })
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    /// A daemon killed with SIGKILL leaves its pid file and socket, which read as down; the next
    /// start replaces them, and a shutdown request answers, refuses the next connect and exits 0.
    #[test]
    fn recover() -> Outcome {
        let root = temp("recover")?;
        let envs = [("SANDMAN_ROOT", root.as_str())];
        let down = "1\n{\"code\":\"down\",\"ok\":false}\n";
        // the guard's drop is the SIGKILL
        drop(serve(&root, &[])?);
        let stale = ["pid", "sock"].map(|kind| fs::exists(format!("{root}/run/serve.{kind}")));
        same(&format!("{stale:?}"), "[Ok(true), Ok(true)]")?;
        same(&sandman(&envs, &["status"], "")?, down)?;
        let (mut daemon, ready) = serve(&root, &[])?;
        same(&ask(&root, "{\"op\":\"health\"}\n")?, &ready)?;
        same(&ask(&root, "{\"op\":\"shutdown\"}\n")?, "{\"ok\":true}\n")?;
        same(&sandman(&envs, &["status"], "")?, down)?;
        same(&format!("{:?}", daemon.0.wait()?.code()), "Some(0)")?;
        let log = fs::read_to_string(format!("{root}/run/serve.log"))?;
        same(
            log.lines()
                .last()
                .and_then(|line| line.get(28..))
                .unwrap_or_default(),
            r#","event":"exit","reason":"shutdown request"}"#,
        )
    }

    /// `serve` refuses a socket path over 103 bytes and a `run/` others can reach; a panic (here
    /// the ready line into a closed stdout) is logged before the process dies.
    #[test]
    fn refuse() -> Outcome {
        let root = temp("refuse")?;
        let deep = format!("{root}/{}", "x".repeat(80));
        same(
            &sandman(&[("SANDMAN_ROOT", &deep)], &["serve"], "")?,
            &format!(
                "1\nsandman serve: {deep}/run/serve.sock is over a unix socket's 103 bytes; shorten SANDMAN_ROOT\n"
            ),
        )?;
        let run = format!("{root}/run");
        fs::create_dir_all(&run)?;
        fs::set_permissions(&run, Permissions::from_mode(0o755))?;
        same(
            &sandman(&[("SANDMAN_ROOT", &root)], &["serve"], "")?,
            &format!("1\nsandman serve: {run} must be this user's, mode 0700\n"),
        )?;
        fs::set_permissions(&run, Permissions::from_mode(0o700))?;
        let (reader, writer) = io::pipe()?;
        drop(reader);
        let out = command(
            &[("SANDMAN_MACHINE", "studio"), ("SANDMAN_ROOT", &root)],
            &["serve"],
        )
        .stdout(writer)
        .output()?;
        let log = fs::read_to_string(format!("{run}/serve.log"))?;
        same(
            &format!(
                "{:?} {} {}",
                out.status.code(),
                log.contains(r#","event":"panic","reason":"panicked at "#),
                log.contains("failed printing to stdout")
            ),
            "Some(101) true true",
        )
    }

    /// The daemon's life: ready on start with `run/` 0700, a socket and its pid; a second instance
    /// refused untouched; typed errors; a silent or vanished client survived; SIGTERM drains to
    /// exit 0 with its line logged, pid file and socket gone, and the log rotated at its cap.
    #[test]
    fn serve_life() -> Outcome {
        let root = temp("serve")?;
        let envs = [
            ("SANDMAN_MACHINE", "studio"),
            ("SANDMAN_ROOT", root.as_str()),
        ];
        let (mut daemon, ready) = serve(&root, &[("SANDMAN_LOG_CAP", "1")])?;
        let (pid, version) = (daemon.0.id(), VERSION);
        let health: Value = serde_json::from_str(&ready)?;
        same(
            &format!(
                "{} {} {} {} {}",
                health["machine"],
                health["pid"],
                health["state"],
                health["version"],
                health["since"].as_str().map_or(0, str::len)
            ),
            &format!(r#""studio" {pid} "ready" "{version}" 20"#),
        )?;
        same(&sandman(&envs, &["status"], "")?, &format!("0\n{ready}"))?;
        same(
            &sandman(&envs, &["serve"], "")?,
            "1\nsandman serve: already running\n",
        )?;
        let run = format!("{root}/run");
        same(
            &format!(
                "{:o} {} {}",
                fs::metadata(&run)?.mode() & 0o777,
                fs::metadata(format!("{run}/serve.sock"))?
                    .file_type()
                    .is_socket(),
                fs::read_to_string(format!("{run}/serve.pid"))?
            ),
            &format!("700 true {pid}"),
        )?;
        same(
            &format!(
                "{}{}{}",
                ask(&root, "not json\n")?,
                ask(&root, "{\"op\":\"nap\"}\n")?,
                ask(&root, "")?
            ),
            "{\"code\":\"bad_request\",\"ok\":false}\n{\"code\":\"unknown_op\",\"ok\":false}\n",
        )?;
        // gone before the reply: the write meets EPIPE, which must not kill the daemon
        UnixStream::connect(format!("{run}/serve.sock"))?.write_all(b"{\"op\":\"health\"}\n")?;
        same(&ask(&root, "{\"op\":\"health\"}\n")?, &ready)?;
        same(
            &format!(
                "{} {:?}",
                kill("-TERM", &pid.to_string())?,
                daemon.0.wait()?.code()
            ),
            "true Some(0)",
        )?;
        let logs =
            ["serve.log", "serve.log.1"].map(|file| fs::read_to_string(format!("{run}/{file}")));
        same(
            &logs
                .iter()
                .map(|log| {
                    log.as_ref()
                        .map_or("", |text| text.get(28..).unwrap_or_default())
                })
                .collect::<String>(),
            &format!(
                ",\"event\":\"exit\",\"reason\":\"signal TERM\"}}\n,\"event\":\"start\",\"pid\":{pid},\"version\":\"{version}\"}}\n"
            ),
        )?;
        let left = ["pid", "sock"].map(|kind| fs::exists(format!("{run}/serve.{kind}")));
        same(&format!("{left:?}"), "[Ok(false), Ok(false)]")
    }

    /// The tick's first job ends what never ended, of this machine's sessions started before the
    /// grace with no end since and no job naming them: one without a transcript `gone`, 20 a tick;
    /// one idle past the window taken, one a tick; each a `reconcile` log line. A live transcript, a
    /// job, an end since, a start inside the grace or another machine's session is left alone; a
    /// later start reads as running again in `chart`.
    #[test]
    fn reconcile() -> Outcome {
        let root = temp("reconcile")?;
        let claude = format!("{root}/claude");
        let envs = [
            ("CLAUDE_CONFIG_DIR", claude.as_str()),
            ("SANDMAN_MACHINE", "studio"),
            ("SANDMAN_ROOT", root.as_str()),
        ];
        // 21 without a transcript; a and b idle, c live, d a job's, e ended, f the laptop's
        let lost = |n: u8| format!("5b000000-0000-4000-8000-0000000000{n:02x}");
        let start = |session: &str, machine: &str| {
            format!(
                r#"{{"at":"2026-11-01T00:00:00Z","cwd":"/w","interactive":false,"kind":"start","machine":"{machine}","session":"{session}"}}"#
            )
        };
        let mut lines: Vec<String> = (0..21).map(|n| start(&lost(n), "studio")).collect();
        lines.extend(
            ["a", "b", "c", "d", "e"].map(|letter| start(&format!("{ID}{letter}"), "studio")),
        );
        lines.push(start(&format!("{ID}f"), "laptop"));
        lines.push(format!(
            r#"{{"at":"2026-11-01T01:00:00Z","kind":"end","machine":"studio","outcome":"gone","session":"{ID}e"}}"#
        ));
        plant(
            &root,
            &[
                ("ledger/2026/11/01.jsonl", &lines.join("\n")),
                ("claude/projects/-w/@a.jsonl", "{}\n"),
                ("claude/projects/-w/@b.jsonl", "{}\n"),
                ("claude/projects/-w/@c.jsonl", "{}\n"),
                ("claude/jobs/j/state.json", r#"{"sessionId":"@d"}"#),
            ],
        )?;
        for letter in ["a", "b"] {
            File::open(format!("{claude}/projects/-w/{ID}{letter}.jsonl"))?
                .set_modified(UNIX_EPOCH + Duration::from_secs(1_767_225_600))?;
        }
        // 0 starts now, inside the grace
        let fresh = format!(r#"{{"kind":"start","session":"{ID}0"}}"#);
        same(&sandman(&envs, &["mark"], &fresh)?, "0\n")?;
        let (mut daemon, _) = serve(&root, &[("SANDMAN_TICK_MS", "1000")])?;
        let (log, day) = (format!("{root}/run/serve.log"), date(&["+%Y/%m/%d"])?);
        let prefix = format!("{root}/.archive/claude/{day}/");
        let read = |path: &str| fs::read_to_string(path).unwrap_or_default();
        // whole lines only: the daemon may be mid-write
        let pick = |file: &str, kind: &str| -> Vec<String> {
            read(file)
                .lines()
                .filter(|line| line.contains(kind))
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .map(|mut entry| {
                    entry["at"] = json!(entry["at"].as_str().map(str::len));
                    if let Some(path) = entry["archived"].as_str() {
                        entry["archived"] =
                            json!(path.strip_prefix(&prefix).and_then(|rest| rest.get(6..)));
                    }
                    entry.to_string()
                })
                .collect()
        };
        let reconciled = || pick(&log, r#""event":"reconcile""#);
        until(|| reconciled().len() >= 21)?;
        let first = reconciled();
        until(|| reconciled().len() >= 23)?;
        same(
            &format!(
                "{} {:?}",
                kill("-TERM", &daemon.0.id().to_string())?,
                daemon.0.wait()?.code()
            ),
            "true Some(0)",
        )?;
        let line = |session: &str, event: &str, tail: &str| {
            format!(
                r#"{{{tail}"at":20,"cwd":"/w",{event}"kind":"end","machine":"studio","outcome":{}"session":"{session}"}}"#,
                if tail.is_empty() {
                    r#""gone","reason":"no end recorded","#
                } else {
                    r#""taken","reason":"idle","#
                }
            )
        };
        let want = |event: &str| {
            let idle = |letter: &str| {
                line(
                    &format!("{ID}{letter}"),
                    event,
                    &format!(r#""archived":"-projects--w-{ID}{letter}.jsonl","#),
                )
            };
            iter::once(idle("a"))
                .chain((0..20).map(|n| line(&lost(n), event, "")))
                .chain([idle("b"), line(&lost(20), event, "")])
                .collect::<Vec<_>>()
        };
        let logged = want(r#""event":"reconcile","#);
        same(
            &format!(
                "{first:?}\n{:?}\n{:?}",
                reconciled(),
                pick(&format!("{root}/ledger/{day}.jsonl"), r#""kind":"end""#)
            ),
            &format!(
                "{:?}\n{logged:?}\n{:?}",
                logged.iter().take(21).collect::<Vec<_>>(),
                want("")
            ),
        )?;
        // lost 00 resumes: running again; 01 stays gone
        let resumed = format!(r#"{{"kind":"start","session":"{}"}}"#, lost(0));
        same(&sandman(&envs, &["mark"], &resumed)?, "0\n")?;
        let out = sandman(&envs, &["chart"], "")?;
        let chart: Value =
            serde_json::from_str(out.strip_prefix("0\n").ok_or_else(|| out.clone())?)?;
        let rows: Vec<String> = chart["active"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|row| {
                [lost(0), lost(1)]
                    .iter()
                    .any(|id| row["session"] == id.as_str())
            })
            .map(|row| {
                format!(
                    "{} {} {}",
                    row["end"].is_null(),
                    row["outcome"],
                    row["interactive"]
                )
            })
            .collect();
        same(
            &format!("{rows:?}"),
            r#"["true null false", "false \"gone\" false"]"#,
        )
    }

    /// The tick settles a held session once no job names it and its transcript has been quiet for
    /// the window, never before: a job still naming it or a touch inside the window holds it; a take
    /// that fails is logged and retried next tick; SIGTERM after still exits 0 with its exit line.
    #[test]
    fn settle() -> Outcome {
        let root = temp("settle")?;
        let claude = format!("{root}/claude");
        let envs = [
            ("CLAUDE_CONFIG_DIR", claude.as_str()),
            ("SANDMAN_MACHINE", "studio"),
            ("SANDMAN_ROOT", root.as_str()),
        ];
        // 1 keeps its job, 2 is touched inside the window, 3 cannot leave its project at first
        for (letter, project) in [("1", "-w"), ("2", "-w"), ("3", "-locked"), ("4", "-w")] {
            plant(
                &claude,
                &[
                    (&format!("projects/{project}/@{letter}.jsonl"), "{}\n"),
                    (
                        &format!("jobs/{letter}/state.json"),
                        &format!(r#"{{"sessionId":"{ID}{letter}"}}"#),
                    ),
                ],
            )?;
            let stdin = format!(r#"{{"kind":"start","reason":"logout","session":"{ID}{letter}"}}"#);
            same(
                &format!(
                    "{}{}",
                    sandman(&envs, &["mark"], &stdin)?,
                    sandman(&envs, &["take"], &stdin)?
                ),
                "0\n0\n",
            )?;
            if letter != "2" {
                File::open(format!("{claude}/projects/{project}/{ID}{letter}.jsonl"))?
                    .set_modified(UNIX_EPOCH + Duration::from_secs(1_794_216_600))?;
            }
            if letter != "1" {
                fs::remove_dir_all(format!("{claude}/jobs/{letter}"))?;
            }
        }
        // a recall for 4 after its held end does not hide it from the settle
        let recalled = format!("{ID}4");
        let out = sandman(&envs, &["recall", "--cue", "x", "--session", &recalled], "")?;
        same(out.lines().next().unwrap_or_default(), "0")?;
        let locked = format!("{claude}/projects/-locked");
        fs::set_permissions(&locked, Permissions::from_mode(0o555))?;
        let (mut daemon, _) = serve(
            &root,
            &[("SANDMAN_QUIET_MS", "60000"), ("SANDMAN_TICK_MS", "10")],
        )?;
        let (log, day) = (format!("{root}/run/serve.log"), date(&["+%Y/%m/%d"])?);
        let read = |path: &str| fs::read_to_string(path).unwrap_or_default();
        until(|| {
            read(&log).contains(&format!("{ID}4")) && read(&log).contains("Permission denied")
        })?;
        fs::set_permissions(&locked, Permissions::from_mode(0o755))?;
        let ledger = format!("{root}/ledger/{day}.jsonl");
        until(|| read(&ledger).contains(&format!(r#""reason":"job gone","session":"{ID}3""#)))?;
        same(
            &format!(
                "{} {:?}",
                kill("-TERM", &daemon.0.id().to_string())?,
                daemon.0.wait()?.code()
            ),
            "true Some(0)",
        )?;
        let prefix = format!("{root}/.archive/claude/{day}/");
        let norm = |line: &str| -> Result<String, serde_json::Error> {
            let mut entry: Value = serde_json::from_str(line)?;
            entry["at"] = json!(entry["at"].as_str().map(str::len));
            if let Some(path) = entry["archived"].as_str() {
                entry["archived"] =
                    json!(path.strip_prefix(&prefix).and_then(|rest| rest.get(6..)));
            }
            Ok(entry.to_string())
        };
        let logged = read(&log)
            .lines()
            .filter(|line| line.contains(r#""event":"take""#))
            .map(norm)
            .collect::<Result<BTreeSet<_>, _>>()?;
        let latest = read(&ledger)
            .lines()
            .filter(|line| line.contains(r#""kind":"end""#))
            .map(norm)
            .collect::<Result<Vec<_>, _>>()?;
        let taken = |letter: &str, project: &str, event: &str| {
            format!(
                r#"{{"archived":"-projects-{project}-{ID}{letter}.jsonl","at":20,"cwd":null,{event}"kind":"end","machine":"studio","outcome":"taken","reason":"job gone","session":"{ID}{letter}"}}"#
            )
        };
        let held = |letter: &str, job: &str| {
            format!(
                r#"{{"at":20,"cwd":null,"job":"{job}","kind":"end","machine":"studio","outcome":"held","reason":"logout","session":"{ID}{letter}"}}"#
            )
        };
        same(
            &format!(
                "{logged:?}\n{latest:?}\n{}",
                // the 10 ms tick may encode a taken session before the TERM lands
                read(&log)
                    .lines()
                    .rfind(|line| !line.contains(r#""event":"encode""#))
                    .and_then(|line| line.get(28..))
                    .unwrap_or_default()
            ),
            &format!(
                "{:?}\n{:?}\n{}",
                BTreeSet::from([
                    format!(
                        r#"{{"at":20,"event":"take","reason":"Permission denied (os error 13)","session":"{ID}3"}}"#
                    ),
                    taken("3", "-locked", r#""event":"take","#),
                    taken("4", "-w", r#""event":"take","#),
                ]),
                [
                    held("1", "1"),
                    held("2", "2"),
                    held("3", "3"),
                    held("4", "4"),
                    taken("4", "-w", ""),
                    taken("3", "-locked", ""),
                ],
                r#","event":"exit","reason":"signal TERM"}"#
            ),
        )
    }

    /// `setup --print` prints the launchd plist for `dev.gigantic.sandman` on hotswap's link and
    /// writes nothing; `setup`, against a stub `launchctl` (`launchctl.log` its calls, `loaded/`
    /// its jobs, a bootout letting go one `print` later) over a HOME hotswap already laid out,
    /// boots out the old sandman's agents and removes their plists, has hotswap link
    /// `~/.local/bin/sandman` and load the upgrade agent, writes the plist and bootstraps it,
    /// kickstarts the same plist loaded, reloads a changed one, fails on a failed bootstrap or a job
    /// still loaded past `$SANDMAN_BOOTOUT_MS`, and without a `launchctl` fails before writing
    /// anything; `upgrade` swaps under the manifest, green once the restarted daemon answers with
    /// the new version; `--remove` has hotswap forget the layout and the upgrade agent, then boots
    /// the daemon out and removes its plist and the link.
    #[test]
    fn setup() -> Outcome {
        let (root, path) = (temp("setup")?, env::var("PATH")?);
        let (home, bin) = (format!("{root}/home"), format!("{root}/bin"));
        let (agents, link) = (
            format!("{home}/Library/LaunchAgents"),
            format!("{home}/.local/bin/sandman"),
        );
        let plist = |searched: &str| {
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>EnvironmentVariables</key>
  <dict><key>PATH</key><string>{searched}</string><key>SANDMAN_ROOT</key><string>{root}</string></dict>
  <key>ExitTimeOut</key>
  <integer>2</integer>
  <key>KeepAlive</key>
  <true/>
  <key>Label</key>
  <string>dev.gigantic.sandman</string>
  <key>ProgramArguments</key>
  <array><string>{link}</string><string>serve</string></array>
  <key>RunAtLoad</key>
  <true/>
  <key>StandardErrorPath</key>
  <string>{root}/serve.err</string>
  <key>ThrottleInterval</key>
  <integer>10</integer>
</dict>
</plist>
"#
            )
        };
        same(
            &sandman(
                &[("HOME", &home), ("PATH", &path), ("SANDMAN_ROOT", &root)],
                &["setup", "--print"],
                "",
            )?,
            &format!("0\n{}", plist(&path)),
        )?;
        same(&fs::read_dir(&root)?.count().to_string(), "0")?;
        // without launchctl (Linux), nothing is written
        same(
            &sandman(
                &[
                    ("HOME", &home),
                    ("PATH", "/usr/bin"),
                    ("SANDMAN_ROOT", &root),
                ],
                &["setup"],
                "",
            )?,
            "1\nsandman setup: launchctl: No such file or directory (os error 2) (setup installs a launchd agent, macOS only; --print prints the plist)\n",
        )?;
        same(&fs::read_dir(&root)?.count().to_string(), "0")?;
        plant(
            &root,
            &[
                (
                    "bin/curl",
                    "#!/bin/sh\nprintf https://github.com/jgeschwendt/sandman/releases/tag/v1\n",
                ),
                (
                    "bin/launchctl",
                    &format!(
                        r#"#!/bin/sh
printf '%s\n' "$*" >> {root}/launchctl.log
job=${{2##*/}}
case $1 in
print) [ -f {root}/loaded/$job.going ] && {{ rm {root}/loaded/$job.going; exit 0; }}; [ -f {root}/loaded/$job ] || exit 113;;
bootout) [ -f {root}/stuck ] || mv {root}/loaded/$job {root}/loaded/$job.going;;
bootstrap) [ -f {root}/fail ] && {{ echo 'Bootstrap failed: 5: Input/output error' >&2; exit 5; }}; touch {root}/loaded/$(basename $3 .plist);;
esac
"#
                    ),
                ),
                (
                    "home/Library/LaunchAgents/dev.gigantic.sandman.breathe.plist",
                    "",
                ),
                (
                    "home/Library/LaunchAgents/dev.gigantic.sandman.report.plist",
                    "",
                ),
                ("home/Library/LaunchAgents/io.jlg.kev.plist", ""),
                ("loaded/dev.gigantic.sandman.report", ""),
            ],
        )?;
        for tool in ["curl", "launchctl"] {
            fs::set_permissions(format!("{bin}/{tool}"), Permissions::from_mode(0o755))?;
        }
        // hotswap's layout, its version 1 this build
        let layout = format!("{home}/.local/share/hotswap/sandman");
        let stubbed = format!("{bin}:{path}");
        let gui = format!(
            "gui/{}",
            String::from_utf8(Command::new("id").arg("-u").output()?.stdout)?.trim()
        );
        let label = format!("{gui}/dev.gigantic.sandman");
        let file = format!("{agents}/dev.gigantic.sandman.plist");
        let upgrade = format!("{agents}/dev.gigantic.sandman.upgrade.plist");
        let linked = format!(
            "linked {link} \u{2192} {}\n",
            fs::canonicalize(env!("CARGO_BIN_EXE_sandman"))?.display()
        );
        let swapped = format!(
            "print {gui}/dev.gigantic.sandman.upgrade\nprint-disabled {gui}\nbootstrap {gui} {upgrade}\n"
        );
        let run = |searched: &str, bound: &str, args: &[&str]| {
            sandman(
                &[
                    ("HOME", &home),
                    ("HOTSWAP_TEST_HOME", &home),
                    ("PATH", searched),
                    ("SANDMAN_BOOTOUT_MS", bound),
                    ("SANDMAN_ROOT", &root),
                ],
                args,
                "",
            )
        };
        let setup = |searched: &str, bound: &str, args: &[&str]| {
            run(searched, bound, &[&["setup"], args].concat())
        };
        let lay = || -> Outcome {
            fs::create_dir_all(format!("{layout}/1"))?;
            symlink(env!("CARGO_BIN_EXE_sandman"), format!("{layout}/1/sandman"))?;
            Ok(symlink("1", format!("{layout}/current"))?)
        };
        lay()?;
        let calls = || {
            let log = fs::read_to_string(format!("{root}/launchctl.log"))?;
            fs::remove_file(format!("{root}/launchctl.log")).map(|()| log)
        };
        // a bootout waits out one print that still finds the job
        same(
            &format!("{}{}", setup(&stubbed, "30000", &[])?, calls()?),
            &format!(
                "0
removed {agents}/dev.gigantic.sandman.breathe.plist
launchctl bootout {gui}/dev.gigantic.sandman.report
removed {agents}/dev.gigantic.sandman.report.plist
{linked}wrote {file}
launchctl bootstrap {gui} {file}
print {gui}/dev.gigantic.sandman.breathe
print {gui}/dev.gigantic.sandman.breathe
print {gui}/dev.gigantic.sandman.reflect
print {gui}/dev.gigantic.sandman.reflect
print {gui}/dev.gigantic.sandman.report
bootout {gui}/dev.gigantic.sandman.report
print {gui}/dev.gigantic.sandman.report
print {gui}/dev.gigantic.sandman.report
{swapped}print {label}
print {label}
bootstrap {gui} {file}
"
            ),
        )?;
        let kept: BTreeSet<String> = fs::read_dir(&agents)?
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        same(
            &format!(
                "{kept:?}{}{}",
                fs::read_to_string(&file)? == plist(&stubbed),
                fs::read_link(&link)?.display()
            ),
            &format!(
                r#"{{"dev.gigantic.sandman.plist", "dev.gigantic.sandman.upgrade.plist", "io.jlg.kev.plist"}}true{layout}/current/sandman"#
            ),
        )?;
        // the same plist loaded: one kickstart; a changed one (another PATH) reloaded
        let retired = format!(
            "print {gui}/dev.gigantic.sandman.breathe\nprint {gui}/dev.gigantic.sandman.breathe\nprint {gui}/dev.gigantic.sandman.reflect\nprint {gui}/dev.gigantic.sandman.reflect\nprint {gui}/dev.gigantic.sandman.report\nprint {gui}/dev.gigantic.sandman.report\n"
        );
        same(
            &format!("{}{}", setup(&stubbed, "0", &[])?, calls()?),
            &format!(
                "0\n{linked}wrote {file}\nlaunchctl kickstart -k {label}\n{retired}{swapped}print {label}\nkickstart -k {label}\n"
            ),
        )?;
        // the swap: setup (hotswap's PATH leads with ~/.local/bin, so the plist changed and is
        // reloaded), the daemon's kickstart, then the smoke against the daemon it restarted
        let (_daemon, _) = serve(&root, &[])?;
        same(
            &format!("{}{}", run(&stubbed, "30000", &["upgrade"])?, calls()?),
            &format!(
                "0\nsandman 1: green, 2 smoke probes\n{retired}{swapped}print {label}\nbootout {label}\nprint {label}\nprint {label}\nbootstrap {gui} {file}\nkickstart -k {label}\n"
            ),
        )?;
        same(
            &run(&stubbed, "0", &["upgrade", "--x"])?,
            "2\nusage: sandman upgrade [--check | --back | --retry]\n",
        )?;
        let moved = format!("{bin}:/moved:{path}");
        same(
            &setup(&moved, "30000", &[])?,
            &format!(
                "0\n{linked}wrote {file}\nlaunchctl bootout {label}\nlaunchctl bootstrap {gui} {file}\n"
            ),
        )?;
        fs::write(format!("{root}/stuck"), "")?;
        same(
            &setup(&moved, "0", &["--remove"])?,
            &format!(
                "1\nlaunchctl bootout {label}\nsandman setup: dev.gigantic.sandman still loaded 0 ms after its bootout\n"
            ),
        )?;
        fs::remove_file(format!("{root}/stuck"))?;
        same(
            &setup(&moved, "30000", &["--remove"])?,
            &format!("0\nlaunchctl bootout {label}\nremoved {file}\nremoved {link}\n"),
        )?;
        same(
            &format!("{}{}", fs::exists(&upgrade)?, fs::exists(&layout)?),
            "falsefalse",
        )?;
        // nothing loaded, every bootstrap failing: hotswap's refused load is one line, the
        // daemon's fails setup
        lay()?;
        fs::write(format!("{root}/fail"), "")?;
        same(
            &setup(&moved, "0", &[])?,
            &format!(
                "1\n{linked}wrote {file}\nhotswap: {gui}/dev.gigantic.sandman.upgrade not loaded from {upgrade}: Bootstrap failed: 5: Input/output error\nsandman setup: launchctl bootstrap {gui} {file}: Bootstrap failed: 5: Input/output error\n"
            ),
        )
    }

    /// `show` prints a memory's or a trace's body under its name, type (a trace says so) and date,
    /// and logs the read; it refuses a path that escapes the banks or names no memory or trace file.
    #[test]
    fn show() -> Outcome {
        let root = temp("show")?;
        plant(&root, &CORPUS)?;
        let envs = [
            ("SANDMAN_MACHINE", "studio"),
            ("SANDMAN_ROOT", root.as_str()),
        ];
        let session = format!("{ID}b");
        same(
            &sandman(
                &envs,
                &["show", "-work-alpha/project_vpn.md", "--session", &session],
                "",
            )?,
            "0\nAlpha deploys need the vpn (project) \u{b7} 2026-11-05T00:00:00Z\nConnect first.\n",
        )?;
        same(
            &sandman(&envs, &["show", "-work-alpha/feedback_clock.md"], "")?,
            "0\nPass a clock in (feedback) \u{b7} 2026-11-02T00:00:00Z\nInject the clock.\n",
        )?;
        same(
            &sandman(&envs, &["show", "common/user_short.md"], "")?,
            "0\nPrefers short replies (user) \u{b7} undated\nShort.\n",
        )?;
        same(
            &sandman(&envs, &["show", "traces/-work-beta/feedback_retry.md"], "")?,
            "0\nBeta retries back off twice (feedback \u{b7} trace) \u{b7} 2026-11-06T00:00:00Z\nTwice, then stop.\n",
        )?;
        plant(
            &root,
            &[
                ("secret.md", "---\nname: outside\n---\n"),
                ("memories/-work-alpha/MEMORY.md", "index"),
            ],
        )?;
        for wanted in [
            "../secret.md",
            &format!("{root}/secret.md"),
            "-work-alpha",
            "-work-alpha/MEMORY.md",
            "-work-alpha/../../secret.md",
            "-work-beta/feedback_retry.md",
            "traces/-work-alpha/project_vpn.md",
            "",
        ] {
            same(
                &sandman(&envs, &["show", wanted], "")?,
                &format!(
                    "1\nsandman show: no memory {wanted} (want [traces/]<bank>/<file>.md or knowledge/<topic>/<file>.md under {root})\n"
                ),
            )?;
        }
        same(
            &ledger(&root)?,
            &format!(
                "{{\"at\":\"A\",\"file\":\"-work-alpha/project_vpn.md\",\"kind\":\"show\",\"machine\":\"studio\",\"session\":\"{session}\"}}\n{{\"at\":\"A\",\"file\":\"-work-alpha/feedback_clock.md\",\"kind\":\"show\",\"machine\":\"studio\",\"session\":null}}\n{{\"at\":\"A\",\"file\":\"common/user_short.md\",\"kind\":\"show\",\"machine\":\"studio\",\"session\":null}}\n{{\"at\":\"A\",\"file\":\"traces/-work-beta/feedback_retry.md\",\"kind\":\"show\",\"machine\":\"studio\",\"session\":null}}"
            ),
        )
    }

    /// `status` with no daemon prints down and exits 1; against a daemon of another version it
    /// passes the health through and names both versions and the fix on stderr.
    #[test]
    fn status() -> Outcome {
        let root = temp("status")?;
        same(
            &sandman(&[("SANDMAN_ROOT", &root)], &["status"], "")?,
            "1\n{\"code\":\"down\",\"ok\":false}\n",
        )?;
        fs::create_dir_all(format!("{root}/run"))?;
        let listener = UnixListener::bind(format!("{root}/run/serve.sock"))?;
        drop(thread::spawn(move || -> io::Result<()> {
            let (mut conn, _) = listener.accept()?;
            // read the request first: closing on an unread request resets the client's write
            let _: usize = BufReader::new(&conn).read_line(&mut String::new())?;
            conn.write_all(b"{\"ok\":true,\"version\":\"0.0.0\"}\n")
        }));
        same(
            &sandman(&[("SANDMAN_ROOT", &root)], &["status"], "")?,
            &format!(
                "0\n{{\"ok\":true,\"version\":\"0.0.0\"}}\nsandman status: the daemon is 0.0.0, this client {VERSION}; swap it: sandman setup\n"
            ),
        )
    }

    /// `take` writes nothing for bad stdin (exit 2), a session without a start mark, or a resume;
    /// else one end line: skipped, held by a job, taken past it on `clear` or once the job has moved
    /// on, gone without a transcript, taken with its sidecar and pointer, `-2-` on a same-second
    /// clash; a rename that fails is exit 1 and no line.
    #[test]
    fn take() -> Outcome {
        let root = temp("take")?;
        let claude = format!("{root}/claude");
        let envs = [
            ("CLAUDE_CONFIG_DIR", claude.as_str()),
            ("SANDMAN_MACHINE", "studio"),
            ("SANDMAN_ROOT", root.as_str()),
        ];
        let take = |extra: &[(&str, &str)], letter: &str, rest: &str| {
            sandman(
                &[&envs[..], extra].concat(),
                &["take"],
                &format!(r#"{{"session":"{ID}{letter}"{rest}}}"#),
            )
        };
        let ran = |command: &str| {
            json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "input": {"command": command}}]}})
                .to_string()
        };
        let numbered: Vec<String> = (1u8..=21)
            .map(|n| format!(r#"sandman remember "{n}""#))
            .collect();
        let transcript = [
            r#"{"type":"attachment","cwd":"/forged\n## Banks"}"#.to_owned(),
            r#"{"type":"attachment","cwd":"/work/alpha","sessionId":"sample"}"#.to_owned(),
            "not json".to_owned(),
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"noise"}]}}"#.to_owned(),
            r#"{"type":"user","message":{"content":"   "}}"#.to_owned(),
            r#"{"type":"user","cwd":"/elsewhere","message":{"content":[{"type":"text","text":"sample:  tighten\n the   parser"}]}}"#.to_owned(),
            ran(&numbered.join("; ")),
            ran(r#"sandman remember "sample: first body" && sandman remember --help && orrery attend "sample: orrery body""#),
            ran(r#"sandman remember \"sample: escaped \\ body\" && sandman remember \"open"#),
            ran(r#"sandman remember "a \"quoted\" \\ word \n  spaced" && sandman remember "dangling\"#),
            json!({"type": "user", "message": {"content": "<bash-input>orrery attend \"sample: legacy body\"</bash-input>"}}).to_string(),
            json!({"type": "user", "message": {"content": "sandman remember \"sample: typed\""}}).to_string(),
            json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "sandman remember \"sample: said\""}]}}).to_string(),
            ran(r#"sandman remember "sample: first body""#),
        ]
        .join("\n");
        let bare = "{\"type\":\"assistant\"}\n";
        plant(
            &claude,
            &[
                ("projects/-work-alpha/@a.jsonl", &transcript),
                ("projects/-work-alpha/@a/subagents/agent.jsonl", "{}\n"),
                ("projects/-work-alpha/@b.jsonl", bare),
                ("projects/-work-alpha/@c.jsonl", bare),
                ("projects/-work-alpha/@e.jsonl", bare),
                ("projects/-work-alpha/@0.jsonl", bare),
                ("projects/-work-locked/@f.jsonl", bare),
                // a finished job holds nothing: a's and e's end past theirs
                ("jobs/j0/state.json", r#"{"sessionId":"@e","state":"done"}"#),
                (
                    "jobs/j1/state.json",
                    r#"{"sessionId":"@b","state":"working"}"#,
                ),
                (
                    "jobs/j2/state.json",
                    r#"{"resumeSessionId":"@9","sessionId":"@c","state":"running"}"#,
                ),
                (
                    "jobs/j3/state.json",
                    r#"{"resumeSessionId":"@a","state":"stopped"}"#,
                ),
                ("jobs/pins.json", "[]"),
            ],
        )?;
        for letter in ["a", "c"] {
            File::open(format!("{claude}/projects/-work-alpha/{ID}{letter}.jsonl"))?
                .set_modified(UNIX_EPOCH + Duration::from_secs(1_794_216_600))?;
        }
        let bad = [
            sandman(&envs, &["take"], "not json")?,
            sandman(&envs, &["take"], "{}")?,
            sandman(&envs, &["take"], r#"{"session":"short"}"#)?,
        ];
        same(
            &format!("{}{bad:?}", fs::exists(format!("{root}/ledger"))?),
            &format!(
                "false{:?}",
                ["2\nsandman take: stdin is not {\"session\":\"<uuid>\",...}\n"; 3]
            ),
        )?;
        for letter in ["a", "b", "c", "d", "e", "f"] {
            same(
                &sandman(
                    &envs,
                    &["mark"],
                    &format!(r#"{{"kind":"start","session":"{ID}{letter}"}}"#),
                )?,
                "0\n",
            )?;
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        // `date -r` takes a file on GNU and BSD alike (seconds on BSD only)
        let stamp = |second: u64| -> Result<String, Box<dyn Error>> {
            let clock = format!("{root}/clock");
            File::create(&clock)?.set_modified(UNIX_EPOCH + Duration::from_secs(second))?;
            date(&["-r", &clock, "+%Y/%m/%d/%H%M%S"])
        };
        for second in now..now + 4 {
            plant(
                &root,
                &[(
                    &format!(
                        ".archive/claude/{}-projects--work-alpha-@e.jsonl",
                        stamp(second)?
                    ),
                    "earlier\n",
                )],
            )?;
        }
        let logout = r#","reason":"logout""#;
        let runs = [
            take(&[], "0", logout)?,
            take(&[], "a", r#","reason":"resume""#)?,
            take(&[("SANDMAN_NO_TAKE", "1")], "a", logout)?,
            take(&[("CLAUDE_MEMORY_PIPELINE", "1")], "a", logout)?,
            take(&[], "b", logout)?,
            take(&[], "b", r#","reason":"clear""#)?,
            take(&[], "c", r#","cwd":"/work/gamma","reason":"other""#)?,
            take(&[], "d", logout)?,
            take(&[], "a", r#","cwd":"/stdin","reason":"prompt_input_exit""#)?,
            take(&[], "e", logout)?,
        ];
        let locked = format!("{claude}/projects/-work-locked");
        fs::set_permissions(&locked, Permissions::from_mode(0o555))?;
        let failed = take(&[], "f", logout)?;
        fs::set_permissions(&locked, Permissions::from_mode(0o755))?;
        same(
            &format!("{}{failed}", runs.concat()),
            &format!(
                "{}1\nsandman take: Permission denied (os error 13)\n",
                "0\n".repeat(10)
            ),
        )?;
        let day = date(&["+%Y/%m/%d"])?;
        let prefix = format!("{root}/.archive/claude/{day}/");
        let closing = fs::read_to_string(format!("{root}/ledger/{day}.jsonl"))?
            .lines()
            .filter(|line| line.contains(r#""kind":"end""#))
            .map(|line| {
                let mut end: Value = serde_json::from_str(line)?;
                end["at"] = json!(end["at"].as_str().map(str::len));
                if let Some(path) = end["archived"].as_str() {
                    end["archived"] =
                        json!(path.strip_prefix(&prefix).and_then(|rest| rest.get(6..)));
                }
                Ok(end.to_string())
            })
            .collect::<Result<Vec<_>, serde_json::Error>>()?;
        same(&closing.join("\n"), &[
            format!(r#"{{"at":20,"cwd":null,"kind":"end","machine":"studio","outcome":"skipped","reason":"logout","session":"{ID}a"}}"#),
            format!(r#"{{"at":20,"cwd":null,"kind":"end","machine":"studio","outcome":"skipped","reason":"logout","session":"{ID}a"}}"#),
            format!(r#"{{"at":20,"cwd":null,"job":"j1","kind":"end","machine":"studio","outcome":"held","reason":"logout","session":"{ID}b"}}"#),
            format!(r#"{{"archived":"-projects--work-alpha-{ID}b.jsonl","at":20,"cwd":null,"kind":"end","machine":"studio","outcome":"taken","reason":"clear","session":"{ID}b"}}"#),
            format!(r#"{{"archived":"-projects--work-alpha-{ID}c.jsonl","at":20,"cwd":"/work/gamma","kind":"end","machine":"studio","outcome":"taken","reason":"other","session":"{ID}c"}}"#),
            format!(r#"{{"at":20,"cwd":null,"kind":"end","machine":"studio","outcome":"gone","reason":"logout","session":"{ID}d"}}"#),
            format!(r#"{{"archived":"-projects--work-alpha-{ID}a.jsonl","at":20,"cwd":"/stdin","kind":"end","machine":"studio","outcome":"taken","reason":"prompt_input_exit","session":"{ID}a"}}"#),
            format!(r#"{{"archived":"-2-projects--work-alpha-{ID}e.jsonl","at":20,"cwd":null,"kind":"end","machine":"studio","outcome":"taken","reason":"logout","session":"{ID}e"}}"#),
        ].join("\n"))?;
        let archived = |letter: &str| -> io::Result<String> {
            let suffix = format!("-projects--work-alpha-{ID}{letter}.jsonl");
            Ok(fs::read_dir(&prefix)?
                .flatten()
                .map(|entry| entry.path().to_string_lossy().into_owned())
                .find(|path| path.ends_with(&suffix) && !path.contains("-2-"))
                .unwrap_or_default())
        };
        let (alpha, gamma) = (archived("a")?, archived("c")?);
        let mut highlights: Vec<String> = (7u8..=21).map(|n| n.to_string()).collect();
        highlights.extend(
            [
                "sample: orrery body",
                r"sample: escaped \ body",
                r#"a "quoted" \ word \n spaced"#,
                "sample: legacy body",
                "sample: first body",
            ]
            .map(str::to_owned),
        );
        let pointer =
            |letter: &str| fs::read_to_string(format!("{root}/memories/.recent/{ID}{letter}.json"));
        same(
            &format!(
                "{}{}{:?}",
                pointer("a")?,
                pointer("c")?,
                [
                    format!("{claude}/projects/-work-alpha/{ID}a.jsonl"),
                    format!("{claude}/projects/-work-alpha/{ID}a"),
                    format!(
                        "{}-dir/subagents/agent.jsonl",
                        alpha.trim_end_matches(".jsonl")
                    ),
                    format!("{claude}/projects/-work-alpha/{ID}0.jsonl"),
                ]
                .map(fs::exists)
                .map(Result::ok)
            ),
            &format!(
                r#"{{"archived":{},"cwd":"/work/alpha","ended":"2026-11-09T09:30:00Z","title":"sample: tighten the parser","highlights":{}}}
{{"archived":{},"cwd":"/work/gamma","ended":"2026-11-09T09:30:00Z","title":"{ID}c","highlights":[]}}
[Some(false), Some(false), Some(true), Some(true)]"#,
                json!(alpha),
                json!(highlights),
                json!(gamma)
            ),
        )?;
        let earlier = (now..now + 4)
            .map(|second| {
                Ok(fs::read_to_string(format!(
                    "{root}/.archive/claude/{}-projects--work-alpha-{ID}e.jsonl",
                    stamp(second)?
                ))?)
            })
            .collect::<Result<String, Box<dyn Error>>>()?;
        same(&earlier, &"earlier\n".repeat(4))
    }

    /// `version` prints the release (a checkout's: the crate version) and the build (the commit, `-dirty` over uncommitted
    /// work), one word, what the daemon's health carries.
    #[test]
    fn version() -> Outcome {
        let build = VERSION
            .strip_prefix(concat!(env!("SANDMAN_RELEASE"), "-"))
            .filter(|build| !build.is_empty() && !build.contains(' '))
            .ok_or(VERSION)?;
        same(
            &sandman(&[], &["version"], "")?,
            &format!("0\n{}-{build}\n", env!("SANDMAN_RELEASE")),
        )
    }

    /// No verb, or an unknown one, prints the eleven-line usage and exits 2; `$HOME` stands in
    /// for an unset `SANDMAN_ROOT`.
    #[test]
    fn usage() -> Outcome {
        let home = temp("usage")?;
        for args in [&[][..], &["nap"]] {
            let out = sandman(&[("HOME", &home)], args, "")?;
            same(
                &format!(
                    "{} {}",
                    out.get(..2).unwrap_or_default(),
                    out.lines().count()
                ),
                "2\n 12",
            )?;
        }
        Ok(())
    }
}
