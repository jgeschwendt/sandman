//! The harness every suite shares: a fabricated machine and the built binary
//! run inside it on a cleared environment. Each suite uses a subset.
#![allow(dead_code, reason = "each suite uses a subset")]

use std::env;
use std::fs;
use std::io::{ErrorKind, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Disambiguates directories created within the same process.
static COUNTER: AtomicU32 = AtomicU32::new(0);

/// The environment every git process gets on top of a cleared one.
pub const GIT_ENV: [(&str, &str); 6] = [
    ("GIT_AUTHOR_EMAIL", "machine@example.com"),
    ("GIT_AUTHOR_NAME", "machine"),
    ("GIT_COMMITTER_EMAIL", "machine@example.com"),
    ("GIT_COMMITTER_NAME", "machine"),
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
];

/// A fabricated machine: a temp `$HOME`, removed on drop.
pub struct Machine {
    pub home: PathBuf,
}

impl Machine {
    pub fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
        let home = env::temp_dir().join(format!(
            "sandman-{label}-{}-{serial}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&home).expect("create the fabricated home");
        Self { home }
    }

    pub fn root(&self) -> PathBuf {
        self.home.join(".sandman")
    }

    pub fn grove_code(&self) -> PathBuf {
        self.home.join(".grove").join("code")
    }

    /// `sandman <args>` on a cleared environment: `$HOME`, `$PATH`, a
    /// `$TMPDIR` the sandbox allows (scrubbed, it is /tmp) and — so `cargo
    /// llvm-cov` keeps the binary's coverage — `$LLVM_PROFILE_FILE`.
    pub fn sandman(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sandman"));
        command
            .args(args)
            .env_clear()
            .env("HOME", &self.home)
            .env("PATH", env::var("PATH").unwrap_or_default())
            .env("TMPDIR", env::temp_dir())
            .envs(env::var_os("LLVM_PROFILE_FILE").map(|file| ("LLVM_PROFILE_FILE", file)));
        command
    }

    pub fn write(&self, relative: impl AsRef<Path>, text: &str) -> PathBuf {
        let path = self.home.join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        fs::write(&path, text).expect("write");
        path
    }

    pub fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.home.join(relative)).unwrap_or_default()
    }

    /// An executable `/bin/sh` script. This is how the tests stand in for
    /// `claude`, `docket` and git hooks: no test ever runs the real one.
    pub fn script(&self, relative: impl AsRef<Path>, body: &str) -> PathBuf {
        let path = self.write(relative, &format!("#!/bin/sh\n{body}"));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    }

    /// Today's journal for `verb`, or empty when the verb wrote none.
    pub fn journal(&self, verb: &str) -> String {
        let Ok(entries) = fs::read_dir(self.root().join(".trace")) else {
            return String::new();
        };
        let mut logs: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&format!("{verb}-")))
                    && path.extension().is_some_and(|kind| kind == "log")
            })
            .collect();
        logs.sort();
        logs.into_iter()
            .filter_map(|path| fs::read_to_string(path).ok())
            .collect()
    }

    /// Every trail file's bytes, concatenated in path order.
    pub fn trail_bytes(&self) -> String {
        let mut files = Vec::new();
        let mut stack = vec![self.root().join("trail")];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    files.push(path);
                }
            }
        }
        files.sort();
        files
            .into_iter()
            .map(|path| fs::read_to_string(path).expect("trail file"))
            .collect()
    }

    /// Every trail line, in file order.
    pub fn trail(&self) -> Vec<String> {
        self.trail_bytes().lines().map(ToOwned::to_owned).collect()
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.home);
    }
}

/// Run `command` with `stdin` on its piped stdin. A process that exits before
/// it reads (a declining hook) closes the pipe under the write; that is its
/// behavior, not a harness failure.
pub fn feed(command: &mut Command, stdin: &str) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn sandman");
    if let Err(error) = child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        && error.kind() != ErrorKind::BrokenPipe
    {
        panic!("write stdin: {error}");
    }
    child.wait_with_output().expect("wait for sandman")
}

/// Run git in `dir` on a cleared environment: stdout, else stderr.
pub fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env_clear()
        .env("HOME", dir)
        .env("PATH", env::var("PATH").unwrap_or_default())
        .env("TMPDIR", env::temp_dir())
        .envs(GIT_ENV)
        .output()
        .expect("run git");
    if output.status.success() {
        Ok(stdout(&output))
    } else {
        Err(stderr(&output))
    }
}

/// The bank key rule, duplicated here on purpose: an integration test drives
/// the binary and must not borrow the crate's own implementation to check it.
pub fn bank_key(cwd: &Path) -> String {
    cwd.as_os_str()
        .as_encoded_bytes()
        .iter()
        .map(|&byte| {
            if byte.is_ascii_alphanumeric() {
                char::from(byte)
            } else {
                '-'
            }
        })
        .collect()
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

pub fn code(output: &Output) -> i32 {
    output.status.code().expect("an exit code")
}
