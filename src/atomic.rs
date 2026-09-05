//! Atomic file writes: a temp file in the destination directory, then a
//! rename. A reader never sees a half-written memory or index.
//!
//! The run logs are the one exception — they are appended, never replaced, so
//! two passes writing the same day's log interleave lines instead of losing
//! each other's.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::process;

use crate::error::{Error, Result};

/// Write `contents` to `path`, replacing it in one step.
pub(crate) fn write(path: &Path, contents: &str) -> Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| Error::io(path, invalid("path has no parent directory")))?;
    let name = path
        .file_name()
        .ok_or_else(|| Error::io(path, invalid("path has no file name")))?;

    let mut temp_name = OsString::from(".");
    temp_name.push(name);
    temp_name.push(format!(".tmp.{}", process::id()));
    let temp = directory.join(temp_name);

    let result = fill(&temp, contents);
    if result.is_err() {
        let _ = fs::remove_file(&temp);
        return result;
    }
    fs::rename(&temp, path).map_err(|source| {
        let _ = fs::remove_file(&temp);
        Error::io(path, source)
    })
}

/// Append one line to `path`, creating it and its directory if need be.
pub(crate) fn append_line(path: &Path, line: &str) -> Result<()> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory).map_err(|source| Error::io(directory, source))?;
    }
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .map_err(|source| Error::io(path, source))?;
    writeln!(file, "{line}").map_err(|source| Error::io(path, source))
}

/// Write the temp file and flush it to the device.
fn fill(temp: &Path, contents: &str) -> Result<()> {
    let mut file = File::create(temp).map_err(|source| Error::io(temp, source))?;
    file.write_all(contents.as_bytes())
        .map_err(|source| Error::io(temp, source))?;
    file.sync_all().map_err(|source| Error::io(temp, source))
}

/// An `InvalidInput` io error carrying `message`.
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::{append_line, write};
    use crate::testutil::TempDir;

    #[test]
    fn appending_creates_the_log_and_keeps_every_line() {
        let temp = TempDir::new("atomic-append");
        let path = temp.path().join(".trace").join("dream-2026-08-12.log");
        append_line(&path, "first").expect("append");
        append_line(&path, "second").expect("append again");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "first\nsecond\n"
        );
    }

    #[test]
    fn writes_and_replaces_leaving_no_temp_files() {
        let temp = TempDir::new("atomic");
        let path = temp.path().join("MEMORY.md");
        write(&path, "first\n").expect("write");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "first\n");
        write(&path, "second\n").expect("overwrite");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "second\n");

        let leftovers: Vec<_> = std::fs::read_dir(temp.path())
            .expect("read dir")
            .filter_map(|entry| entry.ok().map(|entry| entry.file_name()))
            .filter(|name| name != "MEMORY.md")
            .collect();
        assert!(leftovers.is_empty(), "left {leftovers:?} behind");
    }

    #[test]
    fn a_missing_directory_is_an_error_not_a_panic() {
        let temp = TempDir::new("atomic-missing");
        let path = temp.path().join("nope").join("MEMORY.md");
        assert!(write(&path, "x").is_err());
    }
}
