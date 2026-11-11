//! End to end: the built binary over temp data roots. The `chart` fixture's output is
//! `docs/data/chart.json`, the sample the docs page draws.

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use std::error::Error;
    use std::io::{self, Write as _};
    use std::path::PathBuf;
    use std::process::{Command, Stdio, id};
    use std::{env, fs};

    /// Session ids for the fixture: `5a000000-0000-4000-8000-00000000000<letter>`.
    const ID: &str = "5a000000-0000-4000-8000-00000000000";
    /// A test's fallible outcome.
    type Outcome = Result<(), Box<dyn Error>>;

    /// `Err` showing both sides unless `got == want`.
    fn same(got: &str, want: &str) -> Outcome {
        if got == want {
            Ok(())
        } else {
            Err(format!("got:\n{got}\nwant:\n{want}").into())
        }
    }

    /// `sandman args` with `envs` in place of `HOME` and the `SANDMAN_*` vars, `stdin` piped in: its exit code, a newline, its
    /// stdout, then its stderr.
    fn sandman(
        envs: &[(&str, &str)],
        args: &[&str],
        stdin: &str,
    ) -> Result<String, Box<dyn Error>> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_sandman"))
            .args(args)
            // not env_clear: llvm-cov reaches the child through LLVM_PROFILE_FILE
            .env_remove("HOME")
            .env_remove("SANDMAN_MACHINE")
            .env_remove("SANDMAN_ROOT")
            .envs(envs.iter().copied())
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
