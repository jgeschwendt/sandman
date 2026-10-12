//! `profiles/shift-test`, live: the wrapper and its Seatbelt profile against the
//! real home, from a probe directory under `~/.grove/code`. Skipped where
//! `sandbox-exec` is missing or already confined (Seatbelt does not nest — so
//! under shift-test itself) and where `$HOME` is a fabricated one.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A loopback server: bind an ephemeral port, listen, connect, accept.
const BIND: &str = "import socket; s = socket.socket(); s.bind(('127.0.0.1', 0)); s.listen(); \
                    c = socket.create_connection(s.getsockname()); s.accept(); c.close()";

#[test]
fn shift_test_confines_a_test_run_but_lets_it_test() {
    let home = PathBuf::from(env::var_os("HOME").expect("HOME"));
    let free = Command::new("/usr/bin/sandbox-exec")
        .args(["-p", "(version 1) (allow default)", "/usr/bin/true"])
        .status()
        .is_ok_and(|status| status.success());
    if free && home.join(".grove/code").is_dir() {
        confines(&home);
    }
}

/// Every probe from a directory under `~/.grove/code`, then the refusal outside it.
fn confines(home: &Path) {
    let wrapper = Path::new(env!("CARGO_MANIFEST_DIR")).join("profiles/shift-test");
    let probe = home.join(format!(".grove/code/.testbox-probe/{}", std::process::id()));
    fs::create_dir_all(&probe).expect("probe dir");
    let desktop = home.join("Desktop/.shift-test-probe");
    let wrong: Vec<_> = [
        (&["sh", "-c", "ls ~/.ssh"][..], false),
        (&["python3", "-c", BIND], true),
        (&["python3", "-c", "import pty; pty.openpty()"], true),
        (&["sh", "-c", "ps -p $$"], true),
        (&["bash", "-c", "cat <(echo hi)"], true),
        (
            &["curl", "-sS", "--max-time", "5", "https://github.com"],
            false,
        ),
        (&["sh", "-c", "echo x > ~/Desktop/.shift-test-probe"], false),
        (&["sh", "-c", "echo x > ./ok"], true),
        (&["ssh", "-V"], false),
        (&[], false),
    ]
    .into_iter()
    .filter(|(args, passes)| {
        let status = Command::new(&wrapper)
            .args(*args)
            .current_dir(&probe)
            .status();
        status.expect("shift-test").success() != *passes
    })
    .collect();
    let outside = Command::new(&wrapper)
        .arg("true")
        .current_dir(env::temp_dir())
        .output()
        .expect("shift-test");
    let ok = fs::read_to_string(probe.join("ok"));
    fs::remove_dir_all(&probe).expect("probe cleanup");
    assert!(wrong.is_empty(), "{wrong:?}");
    assert_eq!(ok.expect("./ok"), "x\n");
    assert!(!desktop.exists());
    assert_eq!(outside.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&outside.stderr).contains("is not under ~/.grove/code"));
}
