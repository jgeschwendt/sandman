//! `forget` — the privacy ending: destroy every copy of one session.
//!
//! It reaches the live transcript, its subagent directory, anything `take`
//! already archived, the dream minds' kept transcripts of it, and the pointer.
//! It never reaches a bank: a memory that was committed is a memory the
//! operator asked for, and unpicking it is `remember`'s and the dream pass's
//! business, not this verb's.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::json::{self, Value};
use crate::paths;
use crate::transcript;

/// Destroy every copy of `session_id`, returning what was destroyed.
///
/// Nothing found at all is an error: a silent success would read as "it is
/// gone" when it may mean the id was wrong.
///
/// The id must be a whole uuid: matching is by substring and destruction is
/// whole-tree, so a fragment like `2026` or `claude` would name an archive
/// year or lane and destroy it.
pub fn forget(data_root: &Path, claude_root: &Path, session_id: &str) -> Result<Vec<PathBuf>> {
    let uuid = session_id.len() == 36
        && session_id.bytes().enumerate().all(|(at, byte)| match at {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        });
    if !uuid {
        return Err(Error::invalid("session id", session_id));
    }
    let found = transcript::find(claude_root, session_id)?;

    let mut targets: Vec<PathBuf> = Vec::new();
    targets.extend(found.transcripts);
    targets.extend(found.directories);
    let archived = named(&data_root.join(paths::ARCHIVE_DIR_NAME), session_id, |_| {
        false
    })?;
    let read = reads(&archived);
    targets.extend(named(&paths::dream_dir(data_root), session_id, |path| {
        legacy(path, &read)
    })?);
    targets.extend(archived);
    let pointer = paths::recent_dir(data_root).join(format!("{session_id}.json"));
    if pointer.exists() {
        targets.push(pointer);
    }

    if targets.is_empty() {
        return Err(Error::not_found("trace", session_id));
    }
    targets.sort();
    for target in &targets {
        destroy(target)?;
    }
    Ok(targets)
}

/// Every path under `dir` whose name carries the session id, or that `also`
/// claims. A match is destroyed whole, so the walk does not descend into it.
fn named(dir: &Path, session_id: &str, also: impl Fn(&Path) -> bool) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    walk(dir, &mut |path| {
        let hit = path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|name| name.contains(session_id))
            || also(path);
        if hit {
            out.push(path.to_path_buf());
        }
        !hit
    })?;
    Ok(out)
}

/// What each archived copy reads as to a mind — its extract — and every cwd
/// its lines record. Read lossily, as dream reads it.
fn reads(archived: &[PathBuf]) -> Vec<(String, Vec<String>)> {
    archived
        .iter()
        .filter_map(|path| fs::read(path).ok())
        .map(|bytes| {
            let text = String::from_utf8_lossy(&bytes);
            let cwds = text
                .lines()
                .filter_map(|line| Some(json::parse(line).ok()?.get("cwd")?.as_str()?.to_owned()))
                .collect();
            (transcript::extract(&text).trim_end().to_owned(), cwds)
        })
        .collect()
}

/// Whether a kept mind transcript that predates `<lane>/<session id>/` — one
/// named by the mind's own id — read one of `reads`. The only tie back is the
/// prompt (`dream::prompt`): its `  cwd:` line must be a cwd the archive
/// records, and its conversation must be the extract or, since a later take
/// may have joined a fragment on, a whole-block prefix of it. Residual: a
/// session whose conversation and cwd both match another's still matches.
fn legacy(path: &Path, reads: &[(String, Vec<String>)]) -> bool {
    fs::read(path).is_ok_and(|bytes| {
        String::from_utf8_lossy(&bytes)
            .lines()
            .any(|line| json::parse(line).is_ok_and(|value| asked(&value, reads)))
    })
}

/// Whether any string inside `value` is a prompt made from one of `reads`.
fn asked(value: &Value, reads: &[(String, Vec<String>)]) -> bool {
    match value {
        Value::String(text) => {
            text.split_once("\n## The conversation\n")
                .is_some_and(|(head, said)| {
                    let said = said.trim_end();
                    let cwd = head.lines().find_map(|line| line.strip_prefix("  cwd: "));
                    !said.is_empty()
                        && reads.iter().any(|(whole, cwds)| {
                            (whole == said || whole.starts_with(&format!("{said}\n\n")))
                                && cwds.iter().any(|known| Some(known.as_str()) == cwd)
                        })
                })
        }
        Value::Array(items) => items.iter().any(|item| asked(item, reads)),
        Value::Object(entries) => entries.iter().any(|(_, item)| asked(item, reads)),
        _ => false,
    }
}

/// Walk `dir` depth-first. `visit` returns whether to descend into a
/// directory it was handed.
fn walk(dir: &Path, visit: &mut impl FnMut(&Path) -> bool) -> Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(Error::io(dir, error)),
    };
    for entry in entries {
        let entry = entry.map_err(|error| Error::io(dir, error))?;
        let path = entry.path();
        let descend = visit(&path);
        if descend && path.is_dir() {
            walk(&path, visit)?;
        }
    }
    Ok(())
}

/// Remove one path, file or tree.
fn destroy(path: &Path) -> Result<()> {
    let result = if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    match result {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::io(path, error)),
    }
}

#[cfg(test)]
mod tests {
    use super::forget;
    use crate::error::Error;
    use crate::json::Value;
    use crate::testutil::TempDir;
    use crate::verbs::dream;
    use std::fs;
    use std::path::PathBuf;

    const SID: &str = "aaaabbbb-cccc-dddd-eeee-ffff00001111";
    const PROJECT: &str = "-Users-you--code-project";

    #[test]
    fn forget_destroys_every_copy_and_leaves_the_banks_alone() {
        let temp = TempDir::new("forget");
        let claude = temp.path().join(".claude");
        let root = temp.path().join(".sandman");
        let project = claude.join("projects").join(PROJECT);
        fs::create_dir_all(&project).expect("project");
        fs::create_dir_all(project.join(SID)).expect("sidecar");
        fs::write(project.join(format!("{SID}.jsonl")), "{}\n").expect("transcript");
        fs::write(project.join(SID).join("agent.jsonl"), "{}\n").expect("sidecar transcript");
        fs::write(project.join("other.jsonl"), "{}\n").expect("other transcript");

        // Nested by day: the walk has to descend `<yyyy>/<mm>/<dd>` to reach
        // the copies, and stop at the `-dir` it destroys whole.
        let archive = crate::paths::archive_day_dir(&root, 2026, 8, 6);
        fs::create_dir_all(&archive).expect("archive");
        let archived = archive.join(format!("121137-projects-{PROJECT}-{SID}.jsonl"));
        let archived_dir = archive.join(format!("121137-projects-{PROJECT}-{SID}-dir"));
        // What an earlier mind read, then a stray byte and a fragment a later
        // take joined on.
        let said = r#"{"type":"user","cwd":"/code","message":{"content":"a secret diary"}}"#;
        let joined = r#"{"type":"assistant","cwd":"/code","message":{"content":"joined"}}"#;
        fs::write(
            &archived,
            [said.as_bytes(), b"\n\xff\n", joined.as_bytes()].concat(),
        )
        .expect("archived");
        fs::create_dir_all(&archived_dir).expect("archived dir");
        fs::write(archived_dir.join("agent.jsonl"), "{}\n").expect("archived sidecar");
        let unrelated = archive.join("121137-projects-other.jsonl");
        fs::write(&unrelated, "{}\n").expect("unrelated archive");

        let recent = root.join("memories").join(".recent");
        fs::create_dir_all(&recent).expect("recent");
        let pointer = recent.join(format!("{SID}.json"));
        let body = format!(
            r#"{{"archived":{},"cwd":"/code"}}"#,
            Value::string(archived.display().to_string()).render()
        );
        fs::write(&pointer, body).expect("pointer");
        fs::write(recent.join("other.json"), "{}\n").expect("other pointer");

        // The minds' kept transcripts: today's filed under the session id;
        // legacy ones named by the minds' own ids, one of this session and
        // one of another with the same conversation in another cwd.
        let lane = crate::paths::dream_dir(&root).join(PROJECT);
        let named = lane.join(SID);
        fs::create_dir_all(&named).expect("kept");
        fs::write(named.join("m0.jsonl"), "{}\n").expect("named mind");
        let asked = dream::prompt(
            &dream::pending(&root).expect("pending")[0],
            &[],
            &crate::transcript::extract(said),
        );
        let mind = |name: &str, asked: &str| {
            let path = lane.join(name);
            let said = format!(
                "{{\"message\":{{\"content\":{}}}}}\n",
                Value::string(asked).render()
            );
            fs::write(&path, [said.as_bytes(), b"\xff"].concat()).expect("mind");
            path
        };
        let dreamt = mind("m1.jsonl", &asked);
        let twin = mind(
            "m2.jsonl",
            &asked.replace("  cwd: /code", "  cwd: /elsewhere"),
        );

        // A bank whose files carry the session id in their name and source.
        let bank = root.join("memories").join(PROJECT);
        fs::create_dir_all(&bank).expect("bank");
        let memory = bank.join(format!("feedback_{SID}.md"));
        fs::write(&memory, format!("---\nsource: {SID}\n---\n\nbody\n")).expect("memory");

        let destroyed = forget(&root, &claude, SID).expect("forget");
        let expected: Vec<PathBuf> = {
            let mut paths = vec![
                project.join(format!("{SID}.jsonl")),
                project.join(SID),
                archived.clone(),
                archived_dir.clone(),
                dreamt,
                named,
                pointer.clone(),
            ];
            paths.sort();
            paths
        };
        assert_eq!(destroyed, expected);
        for path in &expected {
            assert!(!path.exists(), "{} survived", path.display());
        }

        // Untouched: other sessions, and every bank memory.
        assert!(project.join("other.jsonl").is_file());
        assert!(unrelated.is_file() && twin.is_file());
        assert!(recent.join("other.json").is_file());
        assert!(memory.is_file());
    }

    #[test]
    fn forgetting_nothing_is_an_error() {
        let temp = TempDir::new("forget-missing");
        let claude = temp.path().join(".claude");
        let root = temp.path().join(".sandman");
        assert!(matches!(
            forget(&root, &claude, SID),
            Err(Error::NotFound { what: "trace", .. })
        ));
    }

    #[test]
    fn forget_refuses_anything_but_a_whole_uuid() {
        let temp = TempDir::new("forget-fragment");
        let claude = temp.path().join(".claude");
        let root = temp.path().join(".sandman");
        let memory = claude.join("projects").join(PROJECT).join("memory");
        let archive = crate::paths::archive_day_dir(&root, 2026, 8, 6);
        for dir in [&memory, &archive] {
            fs::create_dir_all(dir).expect("dir");
        }
        let short = &SID[..35];
        for fragment in [
            "2026",
            "claude",
            "memory",
            "../escape",
            short,
            &SID.replace('a', "g"),
        ] {
            assert!(
                matches!(
                    forget(&root, &claude, fragment),
                    Err(Error::InvalidInput { .. })
                ),
                "{fragment} was not refused"
            );
        }
        assert!(memory.is_dir() && archive.is_dir());
    }
}
