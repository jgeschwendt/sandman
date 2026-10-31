//! `sandman setup [--remove]` and `sandman upgrade` on a fabricated home: stub `launchctl` and
//! `mise` first on `$PATH`, mise's shim a link to the built binary, so hotswap's setup and smoke
//! steps run the real `sandman`.

mod common;

use std::fs;
use std::os::unix::fs::symlink;
use std::process::Command;

use common::{Machine, feed, git, stderr, stdout};

#[test]
fn setup_installs_every_agent_upgrade_swaps_and_remove_undoes_it() {
    let home = Machine::new("setup");
    let h = home.home.display().to_string();
    let agents = format!("{h}/Library/LaunchAgents");
    // `print` answers loaded while `loaded` exists; `print-disabled` prints `disabled`; `fail`
    // fails the bootstrap of the agent it names
    home.script(
        "stub/launchctl",
        "echo \"$*\" >> \"$HOME/launchctl.log\"\n[ \"$1\" = print ] && { [ -e \"$HOME/loaded\" ]; exit; }\n[ \"$1\" = print-disabled ] && { cat \"$HOME/disabled\"; exit 0; }\n[ \"$1\" = bootstrap ] && [ -e \"$HOME/fail\" ] && case $3 in *$(cat \"$HOME/fail\")*) exit 5 ;; esac\nexit 0",
    );
    home.script(
        "stub/mise",
        "case $1 in ls) cat \"$HOME/ls.json\" ;; latest) echo 2026.10.1 ;; esac",
    );
    home.write(
        "ls.json",
        r#"{"packslip:jgeschwendt/sandman":[{"active":true,"installed":true,"version":"2026.10.1"}]}"#,
    );
    let run = |args: &[&str], night: &str, code: i32| {
        let output = feed(
            home.sandman(args)
                .env("PATH", format!("{h}/stub:/usr/bin:/bin"))
                .env("SANDMAN_SHIFT_NIGHT", night),
            "",
        );
        assert_eq!(
            common::code(&output),
            code,
            "{args:?}: {}{}",
            stdout(&output),
            stderr(&output)
        );
        let log = home.read("launchctl.log");
        fs::remove_file(home.home.join("launchctl.log")).ok();
        format!("{}{}{log}", stdout(&output), stderr(&output))
    };
    let uid =
        String::from_utf8(Command::new("id").arg("-u").output().expect("id").stdout).expect("uid");
    let gui = format!("gui/{}", uid.trim());
    let shim = format!("{h}/.local/share/mise/shims/sandman");
    // one line set per agent, in setup's order, each given its label and its state
    let each = |states: [&str; 4], line: &dyn Fn(&str, &str) -> String| {
        (["breathe", "reflect", "report", "upgrade"]
            .iter()
            .zip(states))
        .map(|(job, state)| line(&format!("dev.gigantic.sandman.{job}"), state))
        .collect::<String>()
    };
    let bootstrap = |label: &str| format!("bootstrap {gui} {agents}/{label}.plist\n");
    let loads = |states: [&str; 4]| {
        let routine = if states[0] == "loaded" {
            "wrote ~/.routines/shift.md"
        } else {
            "edited: ~/.routines/shift.md — left alone"
        };
        format!(
            "linked {h}/.local/bin/sandman → {shim}\nwrote ~/.local/bin/shift-test\nwrote ~/.local/share/sandman/shift-test\nwrote ~/.local/share/sandman/shift-test.sb\n{routine}\n{}",
            each(states, &|label, state| format!("{state} {label}\n"))
        )
    };
    let retire = format!("print-disabled {gui}\nbootout {gui}/dev.gigantic.sandman.install\n");

    let xdg = feed(
        home.sandman(&["setup"])
            .env("PATH", format!("{h}/stub:/usr/bin:/bin"))
            .env("XDG_DATA_HOME", format!("{h}/.local/share")),
        "",
    );
    assert!(stderr(&xdg).contains(&format!("no mise shim at {shim}")));
    fs::remove_file(home.home.join("launchctl.log")).ok();
    for retired in [
        "Library/LaunchAgents/dev.gigantic.sandman.install.plist",
        ".local/share/sandman/install",
    ] {
        home.write(retired, "");
    }
    assert_eq!(
        run(&["setup"], "", 1),
        format!(
            "removed {agents}/dev.gigantic.sandman.install.plist\nremoved {h}/.local/share/sandman/install\nsandman: no mise shim at {shim}: install with `mise use -g packslip:jgeschwendt/sandman` first\n{retire}"
        )
    );
    fs::create_dir_all(home.home.join(".local/share/mise/shims")).expect("shims");
    symlink(env!("CARGO_BIN_EXE_sandman"), &shim).expect("shim");
    fs::create_dir_all(home.home.join(".local/bin/shift-test")).expect("in the way");
    assert!(run(&["setup"], "", 1).contains(&format!("sandman: {h}/.local/bin/shift-test: ")));
    fs::remove_dir(home.home.join(".local/bin/shift-test")).expect("out of the way");
    let boot = each(["loaded"; 4], &|label, _| {
        format!("print {gui}/{label}\n{}", bootstrap(label))
    });
    assert_eq!(
        run(&["setup"], "07-06", 0),
        format!("{}{retire}{boot}", loads(["loaded"; 4]))
    );
    let check = "import plistlib, sys; [plistlib.load(open(p, 'rb')) for p in sys.argv[1:]]";
    let plists = ["breathe", "reflect", "report", "upgrade"]
        .map(|job| format!("{agents}/dev.gigantic.sandman.{job}.plist"));
    let python = Command::new("python3")
        .args(["-c", check])
        .args(&plists)
        .status();
    assert!(python.expect("python3").success());
    let report = home.read("Library/LaunchAgents/dev.gigantic.sandman.report.plist");
    assert!(
        report.contains(&format!("<string>{h}/.local/bin/sandman</string>"))
            && report.contains("<key>SANDMAN_SHIFT_NIGHT</key><string>07-06</string>")
            && !plists
                .iter()
                .any(|p| fs::read_to_string(p).expect("plist").contains("{{"))
            && fs::read_link(home.home.join(".local/bin/sandman"))
                .expect("link")
                .display()
                .to_string()
                == shim
            && home.read(".routines/shift.md") == include_str!("../docs/routines/shift.md"),
        "{report}"
    );

    // loaded: a changed plist reloads, the upgrade agent's never; an edited routine stays
    let routines = home.home.join(".routines");
    for args in [
        &["init", "-q"][..],
        &["add", "."],
        &["commit", "-qm", "routines"],
    ] {
        git(&routines, args).expect("fixture git");
    }
    home.write(".routines/shift.md", "mine\n");
    home.write("loaded", "");
    home.write(&plists[3][h.len() + 1..], "old\n");
    let reloads = ["reloaded", "current", "reloaded", "current"];
    home.write(&plists[0][h.len() + 1..], "old\n");
    let log = each(reloads, &|label, state| {
        let reload = format!("bootout {gui}/{label}\n{}", bootstrap(label));
        format!(
            "print {gui}/{label}\n{}",
            if state == "reloaded" { &reload } else { "" }
        )
    });
    assert_eq!(
        run(&["setup"], "", 0),
        format!("{}{retire}{log}", loads(reloads))
    );
    // a failed bootstrap fails the run, every other agent still loaded
    home.write("fail", "reflect");
    fs::remove_file(home.home.join("loaded")).expect("unload");
    let failed = run(&["setup"], "", 1);
    assert!(
        failed.contains(&format!("loaded dev.gigantic.sandman.upgrade\nsandman: launchctl bootstrap dev.gigantic.sandman.reflect failed\n{retire}{boot}")),
        "{failed}"
    );
    fs::remove_file(home.home.join("fail")).expect("unfail");

    // a disabled agent is held: its plist and a hint, no bootstrap, and setup is green
    home.write(
        "disabled",
        "\t\t\"dev.gigantic.sandman.breathe\" => disabled\n\t\t\"dev.gigantic.sandman.reflect\" => disabled\n\t\t\"dev.gigantic.sandman.report\" => enabled\n",
    );
    let hint = |job: &str| {
        format!(
            "dev.gigantic.sandman.{job}: disabled by launchctl; launchctl enable {gui}/dev.gigantic.sandman.{job} && sandman setup loads it\n"
        )
    };
    let held = run(&["setup"], "", 0);
    assert!(
        held.contains(&format!(
            "loaded dev.gigantic.sandman.report\nloaded dev.gigantic.sandman.upgrade\n{}{}{retire}{}",
            hint("breathe"),
            hint("reflect"),
            &boot[boot.find(&format!("print {gui}/dev.gigantic.sandman.report")).expect("report")..]
        )),
        "{held}"
    );

    // the swap: setup, the report agent's kickstart and both probes run the shim's binary
    let swapped = run(&["upgrade"], "", 0);
    assert!(
        swapped.starts_with("sandman 2026.10.1: green, 2 smoke probes\n")
            && swapped.ends_with(&format!("kickstart -k {gui}/dev.gigantic.sandman.report\n")),
        "{swapped}"
    );
    assert!(home.home.join(".local/state/hotswap/sandman.json").exists());

    home.write("loaded", "");
    // a held agent is never booted out
    let out = each(["held", "held", "", ""], &|label, state| {
        let booted = if state.is_empty() {
            format!("booted out {label}\n")
        } else {
            String::new()
        };
        format!("{booted}removed {agents}/{label}.plist\n")
    });
    let files = format!(
        "removed {h}/.local/bin/shift-test\nremoved {h}/.local/share/sandman/shift-test\nremoved {h}/.local/share/sandman/shift-test.sb\nremoved {h}/.local/bin/sandman\n"
    );
    assert!(run(&["setup", "--remove"], "", 0).starts_with(&format!(
        "{out}edited: ~/.routines/shift.md — left alone\n{files}"
    )));
    git(&routines, &["commit", "-qam", "mine"]).expect("fixture git");
    assert!(
        run(&["setup", "--remove"], "", 0).contains(&format!("removed {h}/.routines/shift.md\n"))
    );
    assert!(!home.home.join(".local/state/hotswap/sandman.json").exists());
}
