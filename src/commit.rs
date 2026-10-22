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
    /// The filenames of memories in this bank that this one supersedes; the
    /// first hands its `created:` forward.
    pub replaces: Vec<String>,
    /// Where the memory came from — session, pass, or operator.
    pub source: String,
}

/// What a commit did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitOutcome {
    /// Where each superseded file was moved, in `replaces` order.
    pub archived: Vec<PathBuf>,
    /// Whether the filename took a collision suffix because the slug's own
    /// name was already taken in the bank.
    pub collided: bool,
    /// The committed filename, collision suffix included.
    pub filename: String,
    /// The committed file's full path.
    pub path: PathBuf,
    /// The instant stamped into `updated:` (and `created:` for a new memory).
    pub committed_at: Timestamp,
}

/// Commit one memory into `bank_key` under `data_root`.
///
/// Holds `.commit.lock` at the data root for the whole write: check
/// everything that can refuse the commit, then archive the replaced files,
/// place the new one under a free name and regenerate the index. Both file
/// writes are atomic.
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
    for replaces in &request.replaces {
        check_filename("replaces", replaces)?;
    }

    let _lock = CommitLock::acquire(data_root)?;
    let bank = Bank::in_data_root(data_root, bank_key);
    fs::create_dir_all(bank.dir()).map_err(|source| Error::io(bank.dir(), source))?;
    // A sibling the index cannot read fails the commit before anything lands,
    // not after: a write that then reports failure invites a duplicating retry.
    // Every other refusal — a replaced file missing or unparseable, a name with
    // no free suffix — runs before anything moves too, so a refused commit
    // leaves the bank as it stood.
    bank.render_index_without(&request.replaces)?;
    let created = request
        .replaces
        .iter()
        .map(|file| created_of(&bank, file))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .next()
        .flatten();
    let (filename, path, collided) =
        free_filename(&bank, request.kind, &request.name, &request.replaces)?;

    let now = Timestamp::now()?;
    let archived = request
        .replaces
        .iter()
        .map(|file| stow(&bank, file, now))
        .collect::<Result<_>>()?;

    let mut frontmatter = Frontmatter::new();
    frontmatter.set("name", &request.name);
    frontmatter.set("description", &request.description);
    frontmatter.set("type", request.kind.as_str());
    frontmatter.set("created", created.unwrap_or_else(|| now.iso8601()));
    frontmatter.set("source", &request.source);
    frontmatter.set("updated", now.iso8601());

    let file = MemoryFile::new(frontmatter, terminate(request.body));
    atomic::write(&path, &file.render())?;
    bank.write_index()?;

    Ok(CommitOutcome {
        archived,
        collided,
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

/// The `created:` a replaced file hands forward, read before anything moves.
fn created_of(bank: &Bank, filename: &str) -> Result<Option<String>> {
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
    Ok(old.frontmatter.get("created").map(ToOwned::to_owned))
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

/// The first filename in the bank that is not taken, `_2`, `_3`, … suffixed,
/// and whether it needed a suffix at all.
///
/// A replaced file is about to move to `_archive/`, so it never counts as a
/// collision with its own replacement.
fn free_filename(
    bank: &Bank,
    kind: MemoryType,
    name: &str,
    replaced: &[String],
) -> Result<(String, PathBuf, bool)> {
    for nth in 1..=MAX_COLLISIONS {
        let filename = slug::filename_nth(kind.as_str(), name, nth);
        let path = bank.dir().join(&filename);
        if !path.exists() || replaced.contains(&filename) {
            return Ok((filename, path, nth > 1));
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
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::Path;

    const BANK: &str = "-Users-you--code-project";

    fn request(name: &str) -> CommitRequest {
        CommitRequest {
            body: format!("Body of {name}.\n"),
            description: format!("description of {name}"),
            kind: MemoryType::Project,
            name: name.to_owned(),
            replaces: Vec::new(),
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
        assert!(outcome.archived.is_empty());

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
    fn a_commit_waits_out_a_short_holder() {
        let temp = TempDir::new("commit-blocked");
        let held = CommitLock::acquire(temp.path()).expect("acquire");
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            drop(held);
        });
        commit_memory(temp.path(), BANK, request("blocked")).expect("commit after release");
        release.join().expect("release");
    }

    #[test]
    fn an_unreadable_sibling_fails_the_commit_before_it_lands() {
        let temp = TempDir::new("commit-unreadable");
        let bank = Bank::in_data_root(temp.path(), BANK);
        fs::create_dir_all(bank.dir()).expect("bank");
        fs::write(bank.dir().join("README.md"), "# readme\n").expect("readme");
        assert!(commit_memory(temp.path(), BANK, request("kept")).is_err());
        assert_eq!(bank.memory_filenames().expect("list"), ["README.md"]);
        // …but replacing that very file repairs the bank.
        fs::write(
            bank.dir().join("README.md"),
            "---\nname: readme\n---\n\nbody\n",
        )
        .expect("readme");
        let mut repair = request("kept");
        repair.replaces = vec!["README.md".to_owned()];
        commit_memory(temp.path(), BANK, repair).expect("repair");
        assert!(bank.index_path().is_file());
        // A bank that is a file cannot be committed into.
        let blocked = Bank::in_data_root(temp.path(), "-blocked");
        fs::write(blocked.dir(), "").expect("a file");
        assert!(matches!(
            commit_memory(temp.path(), "-blocked", request("x")),
            Err(Error::Io { path, .. }) if path == blocked.dir()
        ));
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
        second.replaces = vec![first.filename.clone()];
        let outcome = commit_memory(temp.path(), BANK, second).expect("second commit");

        // Same name → same filename; the archive freed it.
        assert_eq!(outcome.filename, first.filename);
        assert_eq!(outcome.archived.len(), 1);
        let archived = &outcome.archived[0];
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
        assert_eq!(read(archived), original);

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
        second.replaces = vec![first.filename.clone()];
        let outcome = commit_memory(temp.path(), BANK, second).expect("second");

        assert_eq!(outcome.filename, "project_new_name.md");
        assert!(!bank.dir().join(&first.filename).exists());
        assert_eq!(bank.memory_filenames().expect("list"), [outcome.filename]);
    }

    #[test]
    fn a_replaces_target_missing_unreadable_or_unparseable_is_a_typed_error() {
        let temp = TempDir::new("commit-replaces-missing");
        let bank = Bank::in_data_root(temp.path(), BANK);
        fs::create_dir_all(bank.dir().join("project_a_dir.md")).expect("a directory");
        let replacing = |filename: &str| {
            let mut req = request("ghost");
            req.replaces = vec![filename.to_owned()];
            commit_memory(temp.path(), BANK, req)
        };
        assert!(matches!(
            replacing("project_not_here.md"),
            Err(Error::ReplacesMissing { .. })
        ));
        assert!(matches!(
            replacing("project_a_dir.md"),
            Err(Error::Io { .. })
        ));
        fs::write(bank.dir().join("project_broken.md"), "# no frontmatter\n").expect("broken");
        assert!(matches!(
            replacing("project_broken.md"),
            Err(Error::Parse { .. })
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
        // Every suffix taken: the commit refuses rather than counting on…
        for nth in 4..=super::MAX_COLLISIONS {
            let taken = bank.dir().join(format!("project_same_name_{nth}.md"));
            fs::copy(&first.path, taken).expect("taken");
        }
        assert!(matches!(
            commit_memory(temp.path(), BANK, request("same name")),
            Err(Error::TooManyCollisions { .. })
        ));
        // …and before the file it would replace has moved.
        let other = commit_memory(temp.path(), BANK, request("other")).expect("other");
        let mut retitle = request("same name");
        retitle.replaces = vec![other.filename];
        assert!(commit_memory(temp.path(), BANK, retitle).is_err());
        assert!(other.path.is_file() && !bank.archive_dir().exists());
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
                req.replaces = vec!["../../etc/passwd".to_owned()];
                req
            }),
        ];
        for (index, (what, req)) in cases.into_iter().enumerate() {
            let key = if index == 0 { "../escape" } else { BANK };
            let got = commit_memory(temp.path(), key, req);
            assert!(
                matches!(got, Err(Error::InvalidInput { what: refused, .. }) if refused == what),
                "{what}: {got:?}"
            );
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

        // An `_archive/` that will not take the file, or that is a file, fails
        // the move and leaves the memory in the bank.
        let kept = commit_memory(temp.path(), BANK, request("kept")).expect("commit");
        let archive = bank.archive_dir();
        let chmod = |mode| fs::set_permissions(&archive, fs::Permissions::from_mode(mode));
        chmod(0o555).expect("chmod");
        let refused = archive_memory(temp.path(), BANK, &kept.filename);
        chmod(0o755).expect("chmod back");
        assert!(
            matches!(&refused, Err(Error::Io { path, .. }) if path.parent() == Some(archive.as_path())),
            "{refused:?}"
        );
        fs::remove_dir_all(&archive).expect("rm");
        fs::write(&archive, "").expect("a file");
        assert!(matches!(
            archive_memory(temp.path(), BANK, &kept.filename),
            Err(Error::Io { path, .. }) if path == archive
        ));
        assert!(kept.path.is_file());
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
