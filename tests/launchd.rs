//! `sandman plist <job>` and `launchd/install`, the install agent's script, on
//! a fabricated home: a trunk checkout, a stub `mise` that "builds" it, a stub
//! installed `sandman` and a stub `launchctl` first on `$PATH`.

mod common;

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use common::{Machine, feed, git, stderr, stdout};

/// `sandman plist <args>`: exit `code`, stdout.
fn plist(home: &Machine, args: &[&str], night: &str, code: i32) -> String {
    let output = feed(
        home.sandman(&[&["plist"], args].concat())
            .env("SANDMAN_SHIFT_NIGHT", night),
        "",
    );
    assert_eq!(common::code(&output), code, "{args:?}: {}", stderr(&output));
    stdout(&output)
}

/// Whether `text` parses as a property list.
fn parses(text: &str) -> bool {
    let check = "import plistlib, sys; plistlib.loads(sys.stdin.buffer.read())";
    common::code(&feed(Command::new("python3").args(["-c", check]), text)) == 0
}

#[test]
fn plist_renders_each_agent_under_this_home() {
    let home = Machine::new("plist");
    let (install, reflect, report) = (
        plist(&home, &["install"], "", 0),
        plist(&home, &["reflect"], "", 0),
        plist(&home, &["report"], "", 0),
    );
    let sandman = format!(
        "<string>{}/.local/bin/sandman</string>",
        home.home.display()
    );
    let script = format!(
        "<string>{}/.local/share/sandman/install</string>",
        home.home.display()
    );
    assert!(
        parses(&install)
            && parses(&reflect)
            && parses(&report)
            && !format!("{install}{reflect}{report}").contains("{{")
            && install.contains("<string>dev.gigantic.sandman.install</string>")
            && install.contains(&script)
            && reflect.contains(&format!("{sandman}\n    <string>reflect</string>"))
            && reflect.contains("<key>Hour</key><integer>23</integer>")
            && !reflect.contains("RunAtLoad")
            && report.contains("<string>dev.gigantic.sandman.report</string>")
            && !report.contains("<key>SANDMAN_SHIFT_NIGHT")
            && report.contains(&sandman),
        "{install}{reflect}{report}"
    );
    assert_eq!(plist(&home, &["install"], "07-06", 0), install);
    assert_eq!(
        plist(&home, &["report"], "07-06", 0),
        report.replace(
            "</string>\n  </dict>",
            "</string>\n    <key>SANDMAN_SHIFT_NIGHT</key><string>07-06</string>\n  </dict>"
        )
    );
    let homeless = feed(home.sandman(&["plist", "report"]).env_remove("HOME"), "");
    assert_eq!(common::code(&homeless), 1, "{}", stderr(&homeless));
    let unknown = feed(&mut home.sandman(&["plist", "dream"]), "");
    assert!(
        common::code(&unknown) == 2 && stderr(&unknown).contains("install, reflect or report"),
        "{}",
        stderr(&unknown)
    );
    plist(&home, &[], "", 2);
}

/// `launchd/install` under `home`, the stubs' directory first on `$PATH`.
fn install(home: &Machine) -> Output {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("launchd/install");
    Command::new("/bin/sh")
        .arg(script)
        .env_clear()
        .env("HOME", &home.home)
        .env(
            "PATH",
            format!("{}/stub:/usr/bin:/bin", home.home.display()),
        )
        .envs(common::GIT_ENV)
        .output()
        .expect("run launchd/install")
}

#[test]
fn install_follows_a_clean_trunk_and_spares_edited_routines() {
    let home = Machine::new("install");
    let trunk = home.grove_code().join("jgeschwendt/sandman/main");
    for (file, text) in [
        ("docs/routines/edited.md", "canonical\n"),
        ("docs/routines/fresh.md", "canonical\n"),
        ("launchd/install", "script\n"),
        ("profiles/shift-test", "wrapper\n"),
        ("profiles/shift-test.sb", "profile\n"),
    ] {
        home.write(trunk.join(file), text);
    }
    let routines = home.home.join(".routines");
    home.write(routines.join("edited.md"), "mine\n");
    for (dir, args) in [
        (&trunk, &["init", "-q"][..]),
        (&trunk, &["add", "."]),
        (&trunk, &["commit", "-qm", "trunk"]),
        (&routines, &["init", "-q"]),
        (&routines, &["add", "."]),
        (&routines, &["commit", "-qm", "routines"]),
    ] {
        git(dir, args).expect("fixture git");
    }
    home.write(routines.join("edited.md"), "edited\n");
    let sha = git(&trunk, &["rev-parse", "--short", "HEAD"]).expect("sha");
    let sha = sha.trim();
    home.script("stub/launchctl", "echo \"$*\" >> \"$HOME/launchctl.log\"");
    home.script(".local/bin/mise", "exit 1");
    let run = |code: i32, line: &str| {
        let output = install(&home);
        assert!(
            common::code(&output) == code && stdout(&output).contains(line),
            "{line}: {}{}",
            stdout(&output),
            stderr(&output)
        );
    };
    home.script(".local/bin/sandman", &format!("echo 0.1.0-{sha}"));
    home.write(trunk.join("stray"), "");
    run(
        0,
        &format!("dirty: trunk at {sha}, installed {sha} — skipped"),
    );
    fs::remove_file(trunk.join("stray")).expect("clean");
    run(0, &format!("current: {sha}"));
    home.script(".local/bin/sandman", "echo 0.1.0-old");
    run(1, "");
    home.script(
        ".local/bin/mise",
        "mkdir -p \"$3/target/release\" && printf '#!/bin/sh\\necho 0.1.0-new\\n' > \"$3/target/release/sandman\"",
    );
    run(
        0,
        "edited: ~/.routines/edited.md — left alone\ninstalled: old → 0.1.0-new",
    );
    for (path, text) in [
        (".local/bin/shift-test", "wrapper\n"),
        (".local/share/sandman/install", "script\n"),
        (".local/share/sandman/shift-test", "wrapper\n"),
        (".local/share/sandman/shift-test.sb", "profile\n"),
        (".routines/edited.md", "edited\n"),
        (".routines/fresh.md", "canonical\n"),
        ("launchctl.log", "kickstart -k gui/"),
    ] {
        assert!(
            home.read(path).starts_with(text),
            "{path}: {}",
            home.read(path)
        );
    }
}
