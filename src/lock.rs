//! `.commit.lock` — the data root's single writer gate.
//!
//! Create-exclusive, so acquisition is the OS's atomic `O_EXCL`; a second
//! writer waits out the holder — every one is a short write — and gets
//! [`Error::LockHeld`] rather than a corrupted bank only when it outlasts the
//! wait. The lockfile carries the holder's pid for post-mortems and is removed
//! on drop.

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::thread;
use std::time::Duration;

use crate::error::{Error, Result};

/// The lockfile's name at the data root.
pub const LOCK_FILE_NAME: &str = ".commit.lock";
/// How many times an acquire tries a held lock before reporting it.
const LOCK_ATTEMPTS: u32 = 50;
/// The pause between lock attempts.
const LOCK_PAUSE: Duration = Duration::from_millis(100);

/// A held commit lock. Dropping it releases the lock.
#[derive(Debug)]
pub struct CommitLock {
    path: PathBuf,
}

impl CommitLock {
    /// Take the lock at `data_root`, creating the root if it is not there yet,
    /// waiting out a short-lived holder. Every writer waits alike: one that
    /// failed fast would lose each race to a trail append and leave a
    /// multi-step caller (reflect's merge) half-applied.
    ///
    /// A stale lockfile — one left by a killed process — is never stolen: it
    /// is reported, and clearing it is the operator's call.
    pub fn acquire(data_root: &Path) -> Result<Self> {
        let mut attempt = 1;
        loop {
            match Self::try_acquire(data_root) {
                Err(Error::LockHeld { .. }) if attempt < LOCK_ATTEMPTS => {
                    attempt += 1;
                    thread::sleep(LOCK_PAUSE);
                }
                other => return other,
            }
        }
    }

    /// One attempt at the lock.
    fn try_acquire(data_root: &Path) -> Result<Self> {
        fs::create_dir_all(data_root).map_err(|source| Error::io(data_root, source))?;
        let path = data_root.join(LOCK_FILE_NAME);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => {
                let lock = Self { path };
                lock.stamp(file)?;
                Ok(lock)
            }
            Err(source) if source.kind() == ErrorKind::AlreadyExists => {
                Err(Error::LockHeld { path })
            }
            Err(source) => Err(Error::io(&path, source)),
        }
    }

    /// Where the lockfile lives.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Record the holder's pid. Failure releases the lock via `self`'s drop.
    fn stamp(&self, mut file: File) -> Result<()> {
        writeln!(file, "{}", process::id()).map_err(|source| Error::io(&self.path, source))
    }
}

impl Drop for CommitLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::{CommitLock, LOCK_FILE_NAME};
    use crate::error::Error;
    use crate::testutil::TempDir;

    #[test]
    fn a_second_acquire_reports_the_lock_as_held() {
        let temp = TempDir::new("lock-contention");
        let first = CommitLock::acquire(temp.path()).expect("first acquire");
        assert!(first.path().is_file());

        assert!(matches!(
            CommitLock::try_acquire(temp.path()),
            Err(Error::LockHeld { path }) if path == temp.path().join(LOCK_FILE_NAME)
        ));

        drop(first);
        let again = CommitLock::acquire(temp.path()).expect("acquire after release");
        assert!(again.path().is_file());
    }

    #[test]
    fn an_unwritable_root_is_an_io_error_not_held() {
        let temp = TempDir::new("lock-readonly");
        let mode = |mode| {
            std::fs::set_permissions(temp.path(), PermissionsExt::from_mode(mode)).expect("chmod");
        };
        mode(0o555);
        let taken = CommitLock::acquire(temp.path());
        mode(0o755);
        assert!(
            matches!(taken, Err(Error::Io { path, .. }) if path == temp.path().join(LOCK_FILE_NAME))
        );
    }

    #[test]
    fn the_lockfile_carries_the_holder_pid() {
        let temp = TempDir::new("lock-pid");
        let lock = CommitLock::acquire(temp.path()).expect("acquire");
        let contents = std::fs::read_to_string(lock.path()).expect("read lockfile");
        assert_eq!(contents, format!("{}\n", std::process::id()));
    }

    #[test]
    fn releasing_removes_the_lockfile() {
        let temp = TempDir::new("lock-release");
        let path = {
            let lock = CommitLock::acquire(temp.path()).expect("acquire");
            lock.path().to_path_buf()
        };
        assert!(!path.exists());
    }

    #[test]
    fn acquire_creates_a_missing_data_root() {
        let temp = TempDir::new("lock-mkdir");
        let root = temp.path().join("nested").join("root");
        let lock = CommitLock::acquire(&root).expect("acquire");
        assert!(lock.path().is_file());
    }
}
