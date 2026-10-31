//! `setup [--remove]` and `upgrade`, the two verbs of hotswap's tool contract
//! (<https://gigantic.dev/hotswap/>): mise installs and moves the version, setup does the install
//! duties, upgrade the swap.

use std::env;
use std::fs;
use std::io;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::Path;
use std::process::{Command, exit};

use crate::error::Error;
use crate::paths;

use super::{Args, Failure};

/// The upgrade agent's label; hotswap renders its plist.
const UPGRADE: &str = "dev.gigantic.sandman.upgrade";

/// The trunk-build install agent `upgrade` replaced (2026-10-30): setup boots it out and
/// deletes its plist and the script it ran.
const RETIRED: &str = "dev.gigantic.sandman.install";

/// The agents rendered from `launchd/`.
const AGENTS: [(&str, &str); 3] = [
    (
        "dev.gigantic.sandman.breathe",
        include_str!("../../launchd/dev.gigantic.sandman.breathe.plist"),
    ),
    (
        "dev.gigantic.sandman.reflect",
        include_str!("../../launchd/dev.gigantic.sandman.reflect.plist"),
    ),
    (
        "dev.gigantic.sandman.report",
        include_str!("../../launchd/dev.gigantic.sandman.report.plist"),
    ),
];

/// The sandbox files a shift's tests run under, by path under `$HOME`, with their modes.
const FILES: [(&str, &str, u32); 3] = [
    (
        ".local/bin/shift-test",
        include_str!("../../profiles/shift-test"),
        0o755,
    ),
    (
        ".local/share/sandman/shift-test",
        include_str!("../../profiles/shift-test"),
        0o644,
    ),
    (
        ".local/share/sandman/shift-test.sb",
        include_str!("../../profiles/shift-test.sb"),
        0o644,
    ),
];

/// The routine docs, installed into `~/.routines` unless the installed copy carries an edit.
const ROUTINES: [(&str, &str); 1] = [("shift.md", include_str!("../../docs/routines/shift.md"))];

/// `setup [--remove]`: link `~/.local/bin/sandman` to mise's shim, install the sandbox files and
/// routines, write and bootstrap every agent, retire the install agent; `--remove` undoes it all
/// and forgets hotswap's record, leaving the data root alone.
pub(super) fn setup_verb(args: &[String]) -> Result<(), Failure> {
    let remove = Args::parse(args, "setup", &[], &["--remove"], 0)?.set("--remove");
    let home = paths::home()?;
    let uid = Command::new("id").arg("-u").output();
    let gui = format!(
        "gui/{}",
        uid.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default()
    );
    let launchctl = |args: &[&str]| {
        Command::new("launchctl")
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    let agents = home.join("Library/LaunchAgents");
    let gone = |path: &Path| {
        if fs::remove_file(path).is_ok() {
            println!("removed {}", path.display());
        }
    };
    launchctl(&["bootout", &format!("{gui}/{RETIRED}")]);
    gone(&agents.join(format!("{RETIRED}.plist")));
    gone(&home.join(".local/share/sandman/install"));
    let shim = hotswap::shims().join("sandman");
    let link = home.join(".local/bin/sandman");
    let routines = home.join(".routines");
    let edited = |name: &str| {
        Command::new("git")
            .arg("-C")
            .arg(&routines)
            .args(["status", "--porcelain", "--", name])
            .output()
            .is_ok_and(|o| !o.stdout.is_empty())
    };
    let make = |path: &Path, make: &dyn Fn() -> io::Result<()>| {
        fs::create_dir_all(path.parent().unwrap_or(&home))
            .and_then(|()| make())
            .map_err(|source| Error::io(path, source))
    };
    let put = |path: &Path, text: &str, mode: u32| {
        make(path, &|| {
            fs::write(path, text)
                .and_then(|()| fs::set_permissions(path, fs::Permissions::from_mode(mode)))
        })
    };
    let labels = AGENTS.iter().map(|(label, _)| *label).chain([UPGRADE]);
    if remove {
        for label in labels {
            if launchctl(&["bootout", &format!("{gui}/{label}")]) {
                println!("booted out {label}");
            }
            gone(&agents.join(format!("{label}.plist")));
        }
        for (name, _) in ROUTINES {
            if edited(name) {
                println!("edited: ~/.routines/{name} — left alone");
            } else {
                gone(&routines.join(name));
            }
        }
        for path in FILES
            .map(|(path, ..)| home.join(path))
            .iter()
            .chain([&link])
        {
            gone(path);
        }
        hotswap::forget("sandman");
        return Ok(());
    }
    if !shim.exists() {
        return Err(Error::refused(format!(
            "no mise shim at {}: install with `mise use -g packslip:jgeschwendt/sandman` first",
            shim.display()
        ))
        .into());
    }
    fs::remove_file(&link).ok();
    make(&link, &|| symlink(&shim, &link))?;
    println!("linked {} → {}", link.display(), shim.display());
    for (path, text, mode) in FILES {
        put(&home.join(path), text, mode)?;
        println!("wrote ~/{path}");
    }
    for (name, text) in ROUTINES {
        if edited(name) {
            println!("edited: ~/.routines/{name} — left alone");
        } else {
            put(&routines.join(name), text, 0o644)?;
            println!("wrote ~/.routines/{name}");
        }
    }
    let night = env::var("SANDMAN_SHIFT_NIGHT")
        .ok()
        .filter(|night| !night.is_empty())
        .map_or_else(String::new, |night| {
            format!("\n    <key>SANDMAN_SHIFT_NIGHT</key><string>{night}</string>")
        });
    for (label, template) in AGENTS.into_iter().chain([(UPGRADE, "")]) {
        let text = if label == UPGRADE {
            hotswap::agent_plist("sandman", "sandman")
        } else {
            paths::render(template)?.replace("{{EXTRA_ENV}}", &night)
        };
        let path = agents.join(format!("{label}.plist"));
        let fresh = fs::read_to_string(&path).ok() != Some(text.clone());
        put(&path, &text, 0o644)?;
        let target = format!("{gui}/{label}");
        let loaded = launchctl(&["print", &target]);
        // a changed plist reaches launchd only through a reload — never the upgrade agent's,
        // whose bootout would kill the very swap that runs this setup
        let reload = loaded && fresh && label != UPGRADE;
        if reload {
            launchctl(&["bootout", &target]);
        }
        if (!loaded || reload) && !launchctl(&["bootstrap", &gui, &path.to_string_lossy()]) {
            return Err(Error::refused(format!("launchctl bootstrap {label} failed")).into());
        }
        let state = match (loaded, reload) {
            (false, _) => "loaded",
            (true, true) => "reloaded",
            (true, false) => "current",
        };
        println!("{state} {label}");
    }
    Ok(())
}

/// `upgrade [--check | --back | --retry]`: hotswap's swap under sandman's manifest. No drain:
/// a shift, a dream or a take keeps the binary it started on (mise keeps the old version
/// installed beside the new), and the next run of each takes the new one. Only the report
/// server is kickstarted: breathe and reflect are calendar jobs a kickstart would fire now, and
/// each next fire runs the new version through the shim anyway.
pub(super) fn upgrade_verb(args: &[String]) -> Result<(), Failure> {
    let owned = |words: &[&str]| words.iter().map(|&w| w.to_owned()).collect::<Vec<_>>();
    let manifest = hotswap::Manifest {
        agents: owned(&["dev.gigantic.sandman.report"]),
        setup: owned(&["sandman", "setup"]),
        smoke: vec![
            owned(&["sandman", "version"]),
            owned(&["sandman", "shift", "status"]),
        ],
        timeout: Some(120),
        ..hotswap::Manifest::default()
    };
    exit(hotswap::upgrade("sandman", &manifest, args))
}
