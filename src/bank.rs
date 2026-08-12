//! A bank — one directory of memory files, its `MEMORY.md` index and its
//! `_archive/` lineage.
//!
//! The index is regenerated, never hand-edited: fixed frontmatter, then one
//! line per memory file sorted by filename (`docs/BANK-FORMAT.md`).

use std::fs;
use std::path::{Path, PathBuf};

use crate::atomic;
use crate::error::{Error, Result};
use crate::memory::MemoryFile;
use crate::slug::truncate_chars;

/// Where banks live under the data root.
pub const MEMORIES_DIR_NAME: &str = "memories";
/// The per-bank index.
pub const INDEX_FILE_NAME: &str = "MEMORY.md";
/// Superseded files, never deleted.
pub const ARCHIVE_DIR_NAME: &str = "_archive";
/// Index descriptions are hard-cut here — no ellipsis. The longest live entry
/// sits exactly at this length.
pub const INDEX_DESCRIPTION_MAX_CHARS: usize = 150;

/// The index's fixed frontmatter, blank line included.
const INDEX_HEADER: &str = concat!(
    "---\n",
    "name: MEMORY index\n",
    "description: One-line map of all durable memories in this knowledge bank\n",
    "type: reference\n",
    "---\n",
    "\n",
);

/// Whether `name` is a `.md` file. The extension is case-sensitive: the
/// commit path only ever writes lowercase.
pub(crate) fn is_memory_filename(name: &str) -> bool {
    Path::new(name).extension().is_some_and(|ext| ext == "md")
}

/// One directory of memories.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bank {
    dir: PathBuf,
}

impl Bank {
    /// The bank key for a working directory: every non-alphanumeric **byte**
    /// replaced by `-` (`docs/BANK-FORMAT.md`).
    ///
    /// The same encoding Claude Code uses for `~/.claude/projects/`, so a
    /// transcript directory and its bank carry the same name.
    #[must_use]
    pub fn key_for(cwd: &Path) -> String {
        cwd.as_os_str()
            .as_encoded_bytes()
            .iter()
            .map(|byte| {
                if byte.is_ascii_alphanumeric() {
                    char::from(*byte)
                } else {
                    '-'
                }
            })
            .collect()
    }

    /// A bank at an explicit directory.
    #[must_use]
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The bank `key` under `data_root` — `<data_root>/memories/<key>`.
    #[must_use]
    pub fn in_data_root(data_root: &Path, key: &str) -> Self {
        Self::at(data_root.join(MEMORIES_DIR_NAME).join(key))
    }

    /// The bank's directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Where superseded files go.
    #[must_use]
    pub fn archive_dir(&self) -> PathBuf {
        self.dir.join(ARCHIVE_DIR_NAME)
    }

    /// The index file's path.
    #[must_use]
    pub fn index_path(&self) -> PathBuf {
        self.dir.join(INDEX_FILE_NAME)
    }

    /// Every memory file in the bank, sorted by filename. Directories,
    /// non-`.md` files and the index itself are not memories.
    pub fn memory_filenames(&self) -> Result<Vec<String>> {
        let entries = fs::read_dir(&self.dir).map_err(|source| Error::io(&self.dir, source))?;
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| Error::io(&self.dir, source))?;
            let is_file = entry
                .file_type()
                .map_err(|source| Error::io(entry.path(), source))?
                .is_file();
            if !is_file {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
                continue;
            };
            if name == INDEX_FILE_NAME || !is_memory_filename(&name) {
                continue;
            }
            names.push(name);
        }
        names.sort();
        Ok(names)
    }

    /// The index as it should be on disk.
    pub fn render_index(&self) -> Result<String> {
        let mut out = String::from(INDEX_HEADER);
        for name in self.memory_filenames()? {
            let path = self.dir.join(&name);
            let memory = MemoryFile::read(&path)?;
            let entry_name = memory.name().ok_or_else(|| Error::MissingField {
                key: "name",
                path: path.clone(),
            })?;
            let description = memory.description().ok_or_else(|| Error::MissingField {
                key: "description",
                path: path.clone(),
            })?;
            out.push_str("- [");
            out.push_str(entry_name);
            out.push_str("](");
            out.push_str(&name);
            out.push_str(") — ");
            out.push_str(truncate_chars(description, INDEX_DESCRIPTION_MAX_CHARS));
            out.push('\n');
        }
        Ok(out)
    }

    /// Regenerate the index on disk, atomically.
    pub fn write_index(&self) -> Result<()> {
        atomic::write(&self.index_path(), &self.render_index()?)
    }
}

#[cfg(test)]
mod tests {
    use super::{Bank, INDEX_DESCRIPTION_MAX_CHARS, INDEX_FILE_NAME};
    use crate::error::Error;
    use crate::testutil::TempDir;
    use std::fs;
    use std::path::Path;

    fn seed(bank: &Bank, filename: &str, name: &str, description: &str) {
        fs::create_dir_all(bank.dir()).expect("bank dir");
        fs::write(
            bank.dir().join(filename),
            format!("---\nname: {name}\ndescription: {description}\ntype: user\n---\n\nbody\n"),
        )
        .expect("seed file");
    }

    #[test]
    fn index_is_header_plus_one_sorted_line_per_memory() {
        let temp = TempDir::new("index");
        let bank = Bank::at(temp.path().join("bank"));
        seed(&bank, "user_zulu.md", "zulu", "last by filename");
        seed(&bank, "feedback_alpha.md", "alpha", "first by filename");
        seed(&bank, "project_mike.md", "mike — with a dash", "middle");
        // Not memories: the index, a non-markdown file, a directory.
        fs::write(bank.dir().join("_reflect.json"), "{}").expect("json");
        fs::create_dir_all(bank.archive_dir()).expect("archive dir");
        fs::write(bank.archive_dir().join("user_old.md"), "---\n").expect("archived");
        fs::write(bank.index_path(), "stale\n").expect("stale index");

        bank.write_index().expect("write index");
        let index = fs::read_to_string(bank.index_path()).expect("read index");
        assert_eq!(
            index,
            concat!(
                "---\n",
                "name: MEMORY index\n",
                "description: One-line map of all durable memories in this knowledge bank\n",
                "type: reference\n",
                "---\n",
                "\n",
                "- [alpha](feedback_alpha.md) — first by filename\n",
                "- [mike — with a dash](project_mike.md) — middle\n",
                "- [zulu](user_zulu.md) — last by filename\n",
            )
        );
        assert_eq!(bank.memory_filenames().expect("list").len(), 3);
    }

    #[test]
    fn an_empty_bank_indexes_to_the_header_alone() {
        let temp = TempDir::new("index-empty");
        let bank = Bank::at(temp.path().join("bank"));
        fs::create_dir_all(bank.dir()).expect("bank dir");
        let index = bank.render_index().expect("render");
        assert!(index.ends_with("type: reference\n---\n\n"));
        assert!(!index.contains("\n- "));
    }

    #[test]
    fn descriptions_are_hard_cut_at_one_hundred_fifty_characters() {
        let temp = TempDir::new("index-truncate");
        let bank = Bank::at(temp.path().join("bank"));
        let exact = "e".repeat(INDEX_DESCRIPTION_MAX_CHARS);
        let over = format!("{}X", "o".repeat(INDEX_DESCRIPTION_MAX_CHARS));
        // Unicode before the cut: characters count, bytes do not.
        let unicode = format!("{}tail", "—".repeat(INDEX_DESCRIPTION_MAX_CHARS));
        seed(&bank, "user_exact.md", "exact", &exact);
        seed(&bank, "user_over.md", "over", &over);
        seed(&bank, "user_unicode.md", "unicode", &unicode);

        let index = bank.render_index().expect("render");
        assert!(index.contains(&format!("- [exact](user_exact.md) — {exact}\n")));
        assert!(index.contains(&format!(
            "- [over](user_over.md) — {}\n",
            "o".repeat(INDEX_DESCRIPTION_MAX_CHARS)
        )));
        assert!(index.contains(&format!(
            "- [unicode](user_unicode.md) — {}\n",
            "—".repeat(INDEX_DESCRIPTION_MAX_CHARS)
        )));
        assert!(!index.contains('X'));
        assert!(!index.contains("tail"));
    }

    #[test]
    fn a_memory_without_a_description_is_a_typed_error() {
        let temp = TempDir::new("index-missing");
        let bank = Bank::at(temp.path().join("bank"));
        fs::create_dir_all(bank.dir()).expect("bank dir");
        fs::write(
            bank.dir().join("user_bare.md"),
            "---\nname: bare\ntype: user\n---\n\nbody\n",
        )
        .expect("seed");
        match bank.render_index() {
            Err(Error::MissingField { key, .. }) => assert_eq!(key, "description"),
            other => panic!("expected MissingField, got {other:?}"),
        }
    }

    #[test]
    fn an_unparseable_memory_is_a_typed_error() {
        let temp = TempDir::new("index-broken");
        let bank = Bank::at(temp.path().join("bank"));
        fs::create_dir_all(bank.dir()).expect("bank dir");
        fs::write(bank.dir().join("user_broken.md"), "no frontmatter\n").expect("seed");
        assert!(matches!(bank.render_index(), Err(Error::Parse { .. })));
    }

    #[test]
    fn the_bank_key_is_the_cwd_with_every_non_alphanumeric_byte_dashed() {
        assert_eq!(
            Bank::key_for(Path::new("/Users/you/.code/project")),
            "-Users-you--code-project"
        );
        assert_eq!(Bank::key_for(Path::new("/")), "-");
        assert_eq!(Bank::key_for(Path::new("plain42")), "plain42");
        // Bytes, not characters: one dash per UTF-8 byte.
        assert_eq!(Bank::key_for(Path::new("/tmp/é")), "-tmp---");
    }

    #[test]
    fn the_bank_key_resolves_under_the_data_root() {
        let bank = Bank::in_data_root(Path::new("/data"), "-Users-you");
        assert_eq!(bank.dir(), Path::new("/data/memories/-Users-you"));
        assert_eq!(
            bank.index_path(),
            Path::new("/data/memories/-Users-you").join(INDEX_FILE_NAME)
        );
    }
}
