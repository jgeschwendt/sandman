//! End to end: the built binary over temp data roots. The `chart` fixture's output is
//! `docs/data/chart.json`, the sample the docs page draws; every daemon a test starts is killed
//! when its guard drops.

// the binary's dependency, unused by the suite
use signal_hook as _;

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use std::error::Error;
    use std::fs::Permissions;
    use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
    use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _, PermissionsExt as _};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio, id};
    use std::{env, fs, thread};

    /// Session ids for the fixture: `5a000000-0000-4000-8000-00000000000<letter>`.
    const ID: &str = "5a000000-0000-4000-8000-00000000000";
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

    /// `request` to the daemon over `root`, and its whole reply.
    fn ask(root: &str, request: &str) -> io::Result<String> {
        let mut stream = UnixStream::connect(format!("{root}/run/serve.sock"))?;
        stream.write_all(request.as_bytes())?;
        let mut reply = String::new();
        stream.read_to_string(&mut reply).map(|_| reply)
    }

    /// `sandman args` with `envs` in place of `HOME` and the `SANDMAN_*` vars.
    fn command(envs: &[(&str, &str)], args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sandman"));
        let _: &mut Command = command
            .args(args)
            // not env_clear: llvm-cov reaches the child through LLVM_PROFILE_FILE
            .env_remove("HOME")
            .env_remove("SANDMAN_MACHINE")
            .env_remove("SANDMAN_ROOT")
            .envs(envs.iter().copied());
        command
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
        let mut child = command(envs, args)
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

    /// `sandman serve` over `root` with `SANDMAN_LOG_CAP` at `cap`, and the health line it prints
    /// once ready.
    fn serve(root: &str, cap: &str) -> Result<(Daemon, String), Box<dyn Error>> {
        let envs = [
            ("SANDMAN_LOG_CAP", cap),
            ("SANDMAN_MACHINE", "studio"),
            ("SANDMAN_ROOT", root),
        ];
        let mut daemon = Daemon(command(&envs, &["serve"]).stdout(Stdio::piped()).spawn()?);
        let mut ready = String::new();
        let _: usize =
            BufReader::new(daemon.0.stdout.take().ok_or("stdout")?).read_line(&mut ready)?;
        Ok((daemon, ready))
    }

    /// A fresh temp dir named for the test, as a string for `SANDMAN_ROOT`.
    fn temp(name: &str) -> Result<String, Box<dyn Error>> {
        let dir = env::temp_dir().join(format!("sandman-{name}-{}", id()));
        drop(fs::remove_dir_all(&dir));
        fs::create_dir_all(&dir)?;
        Ok(dir.to_str().ok_or("temp path")?.to_owned())
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
        let marks = [
            mark(
                "2026-11-09T08:05:00Z",
                "start",
                "a",
                "/work/alpha",
                "studio",
            ),
            mark(
                "2026-11-09T08:40:00Z",
                "start",
                "a",
                "/work/reload",
                "studio",
            ),
            mark("2026-11-09T09:30:00Z", "end", "a", "/work/alpha", "studio"),
            mark("2026-11-09T10:15:00Z", "start", "b", "/work/beta", "studio"),
            mark("2026-11-09T12:40:00Z", "end", "b", "/work/beta", "studio"),
            mark(
                "2026-11-09T14:00:00Z",
                "start",
                "c",
                "/work/alpha",
                "laptop",
            ),
            mark("2026-11-09T15:10:00Z", "end", "c", "/work/alpha", "laptop"),
            "not json".to_owned(),
            r#"{"kind":"start"}"#.to_owned(),
        ];
        let files = [
            ("marks/2026/11/09.jsonl", marks.join("\n")),
            (
                "marks/2026/11/10.jsonl",
                [
                    mark("2026-11-10T06:00:00Z", "end", "e", "/work/gamma", "laptop"),
                    mark("2026-11-10T07:45:00Z", "start", "d", "/work/beta", "studio"),
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
                memory(
                    "The grammar lives in one file",
                    "reference",
                    "2026-11-09T08:40:00Z",
                    "remember 2026-11-09T08:40:00Z",
                ),
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
        ];
        for (path, text) in files {
            let file = PathBuf::from(&root).join(path.replace('@', ID));
            fs::create_dir_all(file.parent().ok_or("parent")?)?;
            fs::write(file, text)?;
        }
        let want: Value = serde_json::from_str(include_str!("../docs/data/chart.json"))?;
        let out = sandman(
            &[("SANDMAN_MACHINE", "studio"), ("SANDMAN_ROOT", &root)],
            &["chart"],
            "",
        )?;
        let mut got: Value =
            serde_json::from_str(out.strip_prefix("0\n").ok_or_else(|| out.clone())?)?;
        got["at"] = want["at"].clone();
        same(&got.to_string(), &want.to_string())
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

    /// `mark` refuses bad stdin with exit 2 and no write, appends a good mark to today's file,
    /// and exits 1 when the root cannot hold one.
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
        let after = clock()?;
        let lines = fs::read_to_string(format!(
            "{root}/marks/{}.jsonl",
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
                "{{\"at\":\"{}\",\"cwd\":\"/w\",\"interactive\":true,\"kind\":\"start\",\"machine\":\"studio\",\"reason\":\"startup\",\"resume\":\"r\",\"session\":\"{ID}a\",\"surface\":\"cli\"}}\n{{\"at\":\"{}\",\"cwd\":null,\"interactive\":null,\"kind\":\"end\",\"machine\":\"studio\",\"reason\":null,\"resume\":null,\"session\":\"{ID}a\",\"surface\":null}}\n",
                stamps.first().ok_or("start")?,
                stamps.get(1).ok_or("end")?
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

    /// A daemon killed with SIGKILL leaves its pid file and socket, which read as down; the next
    /// start replaces them, and a shutdown request answers, refuses the next connect and exits 0.
    #[test]
    fn recover() -> Outcome {
        let root = temp("recover")?;
        let envs = [("SANDMAN_ROOT", root.as_str())];
        let down = "1\n{\"code\":\"down\",\"ok\":false}\n";
        // the guard's drop is the SIGKILL
        drop(serve(&root, "")?);
        let stale = ["pid", "sock"].map(|kind| fs::exists(format!("{root}/run/serve.{kind}")));
        same(&format!("{stale:?}"), "[Ok(true), Ok(true)]")?;
        same(&sandman(&envs, &["status"], "")?, down)?;
        let (mut daemon, ready) = serve(&root, "")?;
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
        let (mut daemon, ready) = serve(&root, "1")?;
        let (pid, version) = (daemon.0.id(), env!("CARGO_PKG_VERSION"));
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
            &Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .status()?
                .success()
                .to_string(),
            "true",
        )?;
        same(&format!("{:?}", daemon.0.wait()?.code()), "Some(0)")?;
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

    /// `setup` prints the launchd plist for `dev.gigantic.sandman` and writes nothing.
    #[test]
    fn setup() -> Outcome {
        let root = temp("setup")?;
        same(
            &sandman(&[("SANDMAN_ROOT", &root)], &["setup"], "")?,
            &format!(
                r#"0
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>EnvironmentVariables</key>
  <dict><key>SANDMAN_ROOT</key><string>{root}</string></dict>
  <key>ExitTimeOut</key>
  <integer>2</integer>
  <key>KeepAlive</key>
  <true/>
  <key>Label</key>
  <string>dev.gigantic.sandman</string>
  <key>ProgramArguments</key>
  <array><string>{}</string><string>serve</string></array>
  <key>RunAtLoad</key>
  <true/>
  <key>StandardErrorPath</key>
  <string>{root}/run/launchd.err</string>
  <key>ThrottleInterval</key>
  <integer>10</integer>
</dict>
</plist>
"#,
                env!("CARGO_BIN_EXE_sandman")
            ),
        )?;
        same(&fs::read_dir(&root)?.count().to_string(), "0")
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
            conn.write_all(b"{\"ok\":true,\"version\":\"0.0.0\"}\n")
        }));
        same(
            &sandman(&[("SANDMAN_ROOT", &root)], &["status"], "")?,
            &format!(
                "0\n{{\"ok\":true,\"version\":\"0.0.0\"}}\nsandman status: the daemon is 0.0.0, this client {}; swap it: launchctl kickstart -k gui/$(id -u)/dev.gigantic.sandman\n",
                env!("CARGO_PKG_VERSION")
            ),
        )
    }

    /// No verb, or an unknown one, prints the two-line usage and exits 2; `$HOME` stands in
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
                "2\n 3",
            )?;
        }
        Ok(())
    }
}
