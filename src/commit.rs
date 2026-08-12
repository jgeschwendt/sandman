//! `commit_memory` — the single format authority.
//!
//! Every bank write goes through here: the lock, `replaces` archiving into
//! `_archive/`, collision suffixes, the atomic file write and the `MEMORY.md`
//! regeneration that follows it. No other writer re-implements any of it.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::atomic;
use crate::bank::Bank;
use crate::error::{Error, Result};
use crate::lock::CommitLock;
use crate::memory::{Frontmatter, MemoryFile, MemoryType};
use crate::slug;
use crate::time::Timestamp;

/// How many collision suffixes are tried before giving up.
const MAX_COLLISIONS: u32 = 1_000;

/// One memory to commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitRequest {
    /// The markdown body. A trailing newline is added if it is missing.
    pub body: String,
    /// The one-line description; also the index entry, cut at 150 characters.
    pub description: String,
    /// The memory type — the filename prefix and the `type:` value.
    pub kind: MemoryType,
    /// The memory's name; the slug is derived from it.
    pub name: String,
    /// The filename of a memory in this bank that this one supersedes.
    pub replaces: Option<String>,
    /// Where the memory came from — session, pass, or operator.
    pub source: String,
}

/// What a commit did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitOutcome {
    /// Where the superseded file was moved, when `replaces` was set.
    pub archived: Option<PathBuf>,
    /// The committed filename, collision suffix included.
    pub filename: String,
    /// The committed file's full path.
    pub path: PathBuf,
    /// The instant stamped into `updated:` (and `created:` for a new memory).
    pub committed_at: Timestamp,
}

/// Commit one memory into `bank_key` under `data_root`.
///
/// Holds `.commit.lock` at the data root for the whole write: archive the
/// replaced file, place the new one under a free name, then regenerate the
/// index. Both file writes are atomic.
pub fn commit_memory(
    data_root: &Path,
    bank_key: &str,
    request: CommitRequest,
) -> Result<CommitOutcome> {
    check_bank_key(bank_key)?;
    check_single_line("name", &request.name)?;
    check_single_line("description", &request.description)?;
    check_single_line("source", &request.source)?;
    if request.name.trim().is_empty() {
        return Err(Error::invalid("name", &request.name));
    }
    if let Some(replaces) = &request.replaces {
        check_filename("replaces", replaces)?;
    }

    let _lock = CommitLock::acquire(data_root)?;
    let bank = Bank::in_data_root(data_root, bank_key);
    fs::create_dir_all(bank.dir()).map_err(|source| Error::io(bank.dir(), source))?;

    let now = Timestamp::now()?;
    let mut created = now.iso8601();
    let mut archived = None;
    if let Some(replaces) = &request.replaces {
        let superseded = supersede(&bank, replaces, now)?;
        if let Some(inherited) = superseded.created {
            created = inherited;
        }
        archived = Some(superseded.archived);
    }

    let (filename, path) = free_filename(&bank, request.kind, &request.name)?;

    let mut frontmatter = Frontmatter::new();
    frontmatter.set("name", &request.name);
    frontmatter.set("description", &request.description);
    frontmatter.set("type", request.kind.as_str());
    frontmatter.set("created", &created);
    frontmatter.set("source", &request.source);
    frontmatter.set("updated", now.iso8601());

    let file = MemoryFile::new(frontmatter, terminate(request.body));
    atomic::write(&path, &file.render())?;
    bank.write_index()?;

    Ok(CommitOutcome {
        archived,
        filename,
        path,
        committed_at: now,
    })
}

/// Retire one memory into `_archive/`, without replacing it.
///
/// The pruning half of reflect's bank upkeep. Memories are never destroyed:
/// this is the same move a supersession makes, minus the replacement, and it
/// deliberately leaves `MEMORY.md` alone — a caller that prunes several files
/// regenerates the index once, at the end.
pub fn archive_memory(data_root: &Path, bank_key: &str, filename: &str) -> Result<PathBuf> {
    check_bank_key(bank_key)?;
    check_filename("filename", filename)?;
    let _lock = CommitLock::acquire(data_root)?;
    let bank = Bank::in_data_root(data_root, bank_key);
    let path = bank.dir().join(filename);
    if !path.is_file() {
        return Err(Error::ReplacesMissing { path });
    }
    stow(&bank, filename, Timestamp::now()?)
}

/// What the superseded file handed forward.
struct Superseded {
    /// Where it now lives.
    archived: PathBuf,
    /// Its `created:`, carried into the replacement.
    created: Option<String>,
}

/// Move `filename` out of the bank into `_archive/<stamp>_<filename>`,
/// verbatim, and read the `created:` it hands forward.
fn supersede(bank: &Bank, filename: &str, now: Timestamp) -> Result<Superseded> {
    let path = bank.dir().join(filename);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(source) if source.kind() == ErrorKind::NotFound => {
            return Err(Error::ReplacesMissing { path });
        }
        Err(source) => return Err(Error::io(&path, source)),
    };
    let old = MemoryFile::parse(&text).map_err(|source| Error::Parse {
        path: path.clone(),
        source,
    })?;
    let created = old.frontmatter.get("created").map(ToOwned::to_owned);
    let archived = stow(bank, filename, now)?;
    Ok(Superseded { archived, created })
}

/// The move itself: `<bank>/<filename>` → `<bank>/_archive/<stamp>_<filename>`.
fn stow(bank: &Bank, filename: &str, now: Timestamp) -> Result<PathBuf> {
    let archive_dir = bank.archive_dir();
    fs::create_dir_all(&archive_dir).map_err(|source| Error::io(&archive_dir, source))?;
    let archived = archive_dir.join(format!("{}_{filename}", now.stamp()));
    fs::rename(bank.dir().join(filename), &archived)
        .map_err(|source| Error::io(&archived, source))?;
    Ok(archived)
}

/// The first filename in the bank that is not taken, `_2`, `_3`, … suffixed.
///
/// A replaced file has already moved to `_archive/`, so it never counts as a
/// collision with its own replacement.
fn free_filename(bank: &Bank, kind: MemoryType, name: &str) -> Result<(String, PathBuf)> {
    for nth in 1..=MAX_COLLISIONS {
        let filename = slug::filename_nth(kind.as_str(), name, nth);
        let path = bank.dir().join(&filename);
        if !path.exists() {
            return Ok((filename, path));
        }
    }
    Err(Error::TooManyCollisions {
        filename: slug::filename(kind.as_str(), name),
    })
}

/// Frontmatter values are single-line — a newline would break the block.
fn check_single_line(what: &'static str, value: &str) -> Result<()> {
    if value.contains(['\n', '\r']) {
        return Err(Error::invalid(what, value));
    }
    Ok(())
}

/// A bank key is one path component.
fn check_bank_key(key: &str) -> Result<()> {
    if key.is_empty() || key == "." || key == ".." || key.contains(['/', '\\']) {
        return Err(Error::invalid("bank key", key));
    }
    Ok(())
}

/// A caller-named memory file is inside the bank, never a path.
fn check_filename(what: &'static str, name: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\'])
        || !crate::bank::is_memory_filename(name)
    {
        return Err(Error::invalid(what, name));
    }
    Ok(())
}

/// Bodies end with exactly one newline so the file is well-formed; an empty
/// body stays empty.
fn terminate(body: String) -> String {
    if body.is_empty() || body.ends_with('\n') {
        body
    } else {
        let mut body = body;
        body.push('\n');
        body
    }
}

#[cfg(test)]
mod tests {
    use super::{CommitRequest, archive_memory, commit_memory};
    use crate::bank::Bank;
    use crate::error::Error;
    use crate::lock::CommitLock;
    use crate::memory::{MemoryFile, MemoryType};
    use crate::testutil::TempDir;
    use std::fs;
    use std::path::Path;

    const BANK: &str = "-Users-you--code-project";

    fn request(name: &str) -> CommitRequest {
        CommitRequest {
            body: format!("Body of {name}.\n"),
            description: format!("description of {name}"),
            kind: MemoryType::Project,
            name: name.to_owned(),
            replaces: None,
            source: "unit test".to_owned(),
        }
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).expect("read committed file")
    }

    #[test]
    fn a_new_commit_writes_the_documented_frontmatter_order() {
        let temp = TempDir::new("commit-new");
        let outcome =
            commit_memory(temp.path(), BANK, request("orrery extraction")).expect("commit");
        assert_eq!(outcome.filename, "project_orrery_extraction.md");
        assert_eq!(outcome.archived, None);

        let text = read(&outcome.path);
        let parsed = MemoryFile::parse(&text).expect("well-formed");
        let keys: Vec<&str> = parsed
            .frontmatter
            .fields()
            .iter()
            .map(crate::memory::Field::key)
            .collect();
        assert_eq!(
            keys,
            [
                "name",
                "description",
                "type",
                "created",
                "source",
                "updated"
            ]
        );
        let stamp = outcome.committed_at.iso8601();
        assert_eq!(parsed.frontmatter.get("created"), Some(stamp.as_str()));
        assert_eq!(parsed.frontmatter.get("updated"), Some(stamp.as_str()));
        assert_eq!(parsed.frontmatter.get("type"), Some("project"));
        assert_eq!(parsed.body, "Body of orrery extraction.\n");
        assert_eq!(parsed.render(), text, "commit output must round-trip");

        let index = read(&Bank::in_data_root(temp.path(), BANK).index_path());
        assert!(index.ends_with(
            "- [orrery extraction](project_orrery_extraction.md) — description of orrery extraction\n"
        ));
    }

    #[test]
    fn the_lock_is_released_after_a_commit() {
        let temp = TempDir::new("commit-lock");
        commit_memory(temp.path(), BANK, request("first")).expect("commit");
        assert!(!temp.path().join(crate::lock::LOCK_FILE_NAME).exists());
        commit_memory(temp.path(), BANK, request("second")).expect("second commit");
    }

    #[test]
    fn a_held_lock_blocks_the_commit() {
        let temp = TempDir::new("commit-blocked");
        let held = CommitLock::acquire(temp.path()).expect("acquire");
        assert!(matches!(
            commit_memory(temp.path(), BANK, request("blocked")),
            Err(Error::LockHeld { .. })
        ));
        drop(held);
        commit_memory(temp.path(), BANK, request("blocked")).expect("commit after release");
    }

    #[test]
    fn replaces_archives_the_old_file_and_carries_created_forward() {
        let temp = TempDir::new("commit-replaces");
        let bank = Bank::in_data_root(temp.path(), BANK);
        let first = commit_memory(temp.path(), BANK, request("stele v1")).expect("first commit");
        let original = read(&first.path);
        let created = MemoryFile::parse(&original)
            .expect("well-formed")
            .frontmatter
            .get("created")
            .expect("created")
            .to_owned();

        let mut second = request("stele v1");
        second.body = "Superseding body.\n".to_owned();
        second.replaces = Some(first.filename.clone());
        let outcome = commit_memory(temp.path(), BANK, second).expect("second commit");

        // Same name → same filename; the archive freed it.
        assert_eq!(outcome.filename, first.filename);
        let archived = outcome.archived.expect("archived path");
        assert_eq!(archived.parent(), Some(bank.archive_dir().as_path()));
        let archived_name = archived
            .file_name()
            .and_then(|name| name.to_str())
            .expect("archive name");
        assert_eq!(
            archived_name,
            format!("{}_{}", outcome.committed_at.stamp(), first.filename)
        );
        assert_eq!(archived_name.len(), 15 + 1 + first.filename.len());
        // Archived verbatim — byte for byte as it stood.
        assert_eq!(read(&archived), original);

        let replacement = MemoryFile::parse(&read(&outcome.path)).expect("well-formed");
        assert_eq!(
            replacement.frontmatter.get("created"),
            Some(created.as_str())
        );
        assert_eq!(
            replacement.frontmatter.get("updated"),
            Some(outcome.committed_at.iso8601().as_str())
        );
        assert_eq!(replacement.body, "Superseding body.\n");

        // The index lists the replacement once and never the archive.
        let index = read(&bank.index_path());
        assert_eq!(index.matches("project_stele_v1.md").count(), 1);
        assert!(!index.contains(archived_name));
    }

    #[test]
    fn replaces_a_file_under_a_different_name_leaves_only_the_new_one() {
        let temp = TempDir::new("commit-rename");
        let bank = Bank::in_data_root(temp.path(), BANK);
        let first = commit_memory(temp.path(), BANK, request("old name")).expect("first");
        let mut second = request("new name");
        second.replaces = Some(first.filename.clone());
        let outcome = commit_memory(temp.path(), BANK, second).expect("second");

        assert_eq!(outcome.filename, "project_new_name.md");
        assert!(!bank.dir().join(&first.filename).exists());
        assert_eq!(bank.memory_filenames().expect("list"), [outcome.filename]);
    }

    #[test]
    fn a_missing_replaces_target_is_a_typed_error() {
        let temp = TempDir::new("commit-replaces-missing");
        let mut req = request("ghost");
        req.replaces = Some("project_not_here.md".to_owned());
        assert!(matches!(
            commit_memory(temp.path(), BANK, req),
            Err(Error::ReplacesMissing { .. })
        ));
        // The lock did not survive the failure.
        assert!(!temp.path().join(crate::lock::LOCK_FILE_NAME).exists());
    }

    #[test]
    fn colliding_names_take_numbered_suffixes() {
        let temp = TempDir::new("commit-collision");
        let bank = Bank::in_data_root(temp.path(), BANK);
        let first = commit_memory(temp.path(), BANK, request("same name")).expect("first");
        let second = commit_memory(temp.path(), BANK, request("same name")).expect("second");
        let third = commit_memory(temp.path(), BANK, request("same-name!")).expect("third");

        assert_eq!(first.filename, "project_same_name.md");
        assert_eq!(second.filename, "project_same_name_2.md");
        assert_eq!(third.filename, "project_same_name_3.md");
        assert_eq!(
            bank.memory_filenames().expect("list"),
            [
                "project_same_name.md",
                "project_same_name_2.md",
                "project_same_name_3.md"
            ]
        );
        let index = read(&bank.index_path());
        assert_eq!(
            index.lines().filter(|line| line.starts_with("- [")).count(),
            3
        );
    }

    #[test]
    fn hostile_inputs_are_rejected_before_anything_is_written() {
        let temp = TempDir::new("commit-hostile");
        let cases: Vec<(&str, CommitRequest)> = vec![
            ("bank key", request("traversal")),
            ("name", {
                let mut req = request("multi");
                req.name = "two\nlines".to_owned();
                req
            }),
            ("description", {
                let mut req = request("multi");
                req.description = "two\nlines".to_owned();
                req
            }),
            ("source", {
                let mut req = request("multi");
                req.source = "two\nlines".to_owned();
                req
            }),
            ("name", {
                let mut req = request("blank");
                req.name = "   ".to_owned();
                req
            }),
            ("replaces", {
                let mut req = request("escape");
                req.replaces = Some("../../etc/passwd".to_owned());
                req
            }),
        ];
        for (index, (what, req)) in cases.into_iter().enumerate() {
            let key = if index == 0 { "../escape" } else { BANK };
            match commit_memory(temp.path(), key, req) {
                Err(Error::InvalidInput { what: got, .. }) => assert_eq!(got, what),
                other => panic!("expected InvalidInput({what}), got {other:?}"),
            }
        }
        assert!(!temp.path().join("memories").exists());
    }

    #[test]
    fn archiving_retires_a_memory_without_replacing_it() {
        let temp = TempDir::new("commit-archive");
        let bank = Bank::in_data_root(temp.path(), BANK);
        let first = commit_memory(temp.path(), BANK, request("stale fact")).expect("commit");
        let original = read(&first.path);

        let archived = archive_memory(temp.path(), BANK, &first.filename).expect("archive");
        assert!(!first.path.exists());
        assert_eq!(archived.parent(), Some(bank.archive_dir().as_path()));
        assert!(
            archived
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(&first.filename)),
            "{archived:?}"
        );
        // Verbatim, and the lock was released.
        assert_eq!(read(&archived), original);
        assert!(!temp.path().join(crate::lock::LOCK_FILE_NAME).exists());
        // The index is the caller's to regenerate — one pass, not one per file.
        assert!(read(&bank.index_path()).contains(&first.filename));
        bank.write_index().expect("regen");
        assert!(!read(&bank.index_path()).contains(&first.filename));

        // Archiving what is not there, or what is not a filename, is refused.
        assert!(matches!(
            archive_memory(temp.path(), BANK, &first.filename),
            Err(Error::ReplacesMissing { .. })
        ));
        assert!(matches!(
            archive_memory(temp.path(), BANK, "../escape.md"),
            Err(Error::InvalidInput {
                what: "filename",
                ..
            })
        ));
    }

    #[test]
    fn a_body_without_a_trailing_newline_is_terminated() {
        let temp = TempDir::new("commit-body");
        let mut req = request("unterminated");
        req.body = "no newline".to_owned();
        let outcome = commit_memory(temp.path(), BANK, req).expect("commit");
        assert!(read(&outcome.path).ends_with("\n\nno newline\n"));
    }

    #[test]
    fn unicode_survives_the_commit_and_the_index() {
        let temp = TempDir::new("commit-unicode");
        let mut req = request("placeholder");
        req.name = "game_01: Zelda/LA-remake, ported web→Rust+Bevy".to_owned();
        req.description = "em—dash · “quotes” · 🌙".to_owned();
        req.body = "Body — with an em dash.\n".to_owned();
        let outcome = commit_memory(temp.path(), BANK, req).expect("commit");
        assert_eq!(
            outcome.filename,
            "project_game_01_zelda_la_remake_ported_web_rust_bevy.md"
        );
        let text = read(&outcome.path);
        assert!(text.contains("name: game_01: Zelda/LA-remake, ported web→Rust+Bevy\n"));
        let index = read(&Bank::in_data_root(temp.path(), BANK).index_path());
        assert!(index.contains("— em—dash · “quotes” · 🌙\n"));
    }
}
