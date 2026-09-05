//! `forget` — the privacy ending: destroy every copy of one session.
//!
//! It reaches the live transcript, its subagent directory, anything `take`
//! already archived, and the pointer. It never reaches a bank: a memory that
//! was committed is a memory the operator asked for, and unpicking it is
//! `remember`'s and the dream pass's business, not this verb's.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::paths;
use crate::transcript;

/// Destroy every copy of `session_id`, returning what was destroyed.
///
/// Nothing found at all is an error: a silent success would read as "it is
/// gone" when it may mean the id was wrong.
pub fn forget(data_root: &Path, claude_root: &Path, session_id: &str) -> Result<Vec<PathBuf>> {
    transcript::check_session_id(session_id)?;
    let found = transcript::find(claude_root, session_id)?;

    let mut targets: Vec<PathBuf> = Vec::new();
    targets.extend(found.transcripts);
    targets.extend(found.directories);
    targets.extend(archived_copies(data_root, session_id)?);
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

/// Every archived path under the data root whose name carries the session id.
fn archived_copies(data_root: &Path, session_id: &str) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    walk(&data_root.join(paths::ARCHIVE_DIR_NAME), &mut |path| {
        if path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|name| name.contains(session_id))
        {
            out.push(path.to_path_buf());
            // The match is destroyed whole; descending into it is pointless.
            return false;
        }
        true
    })?;
    Ok(out)
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
    use crate::testutil::TempDir;
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
        fs::write(&archived, "{}\n").expect("archived");
        fs::create_dir_all(&archived_dir).expect("archived dir");
        fs::write(archived_dir.join("agent.jsonl"), "{}\n").expect("archived sidecar");
        let unrelated = archive.join("121137-projects-other.jsonl");
        fs::write(&unrelated, "{}\n").expect("unrelated archive");

        let recent = root.join("memories").join(".recent");
        fs::create_dir_all(&recent).expect("recent");
        let pointer = recent.join(format!("{SID}.json"));
        fs::write(&pointer, "{}\n").expect("pointer");
        fs::write(recent.join("other.json"), "{}\n").expect("other pointer");

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
        assert!(unrelated.is_file());
        assert!(recent.join("other.json").is_file());
        assert!(memory.is_file());
    }

    #[test]
    fn forgetting_nothing_is_an_error() {
        let temp = TempDir::new("forget-missing");
        let claude = temp.path().join(".claude");
        let root = temp.path().join(".sandman");
        assert!(matches!(
            forget(&root, &claude, "no-such-session"),
            Err(Error::NotFound { what: "trace", .. })
        ));
        assert!(matches!(
            forget(&root, &claude, "../escape"),
            Err(Error::InvalidInput { .. })
        ));
    }
}
