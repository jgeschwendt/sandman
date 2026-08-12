//! A temp directory for the unit tests — the crate takes no dependencies, so
//! there is no `tempfile`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Disambiguates directories created within the same process.
static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A directory that removes itself on drop.
pub(crate) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Create a uniquely named directory under the system temp dir.
    pub(crate) fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "sandman-{label}-{}-{serial}-{nanos}",
            process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    /// The directory.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Write an executable `/bin/sh` script at `<dir>/<name>` and return its path.
///
/// This is how the tests stand in for the `claude` binary: no test in the
/// suite ever runs the real one, and the minds are pointed here through
/// `Ask::binary` (in-process) or `$SANDMAN_CLAUDE_BIN` (child process).
#[cfg(unix)]
pub(crate) fn stub_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;

    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}")).expect("write stub");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod stub");
    path
}
