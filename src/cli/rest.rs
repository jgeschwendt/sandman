//! The single verbs: `remember`, `dream`, `reflect`, `mcp`, `version`, `forget`.

use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Instant;

use crate::error::Error;
use crate::memory::MemoryType;
use crate::paths;
use crate::time::Timestamp;
use crate::verbs::{dream, forget, mcp, reflect, remember};
use crate::version::VERSION;

use super::{Args, Failure, NONE, calendar_day, knobs, note};

/// Claude Code names the running session here.
const SESSION_ENV: &str = "CLAUDE_SESSION_ID";

/// `remember "<body>" [flags]`.
pub(super) fn remember_verb(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(
        args,
        "remember",
        &["--bank", "--cwd", "--description", "--name", "--type"],
        &[],
        1,
    )?;
    let request = remember::Remember {
        bank: parsed.flag("--bank"),
        body: parsed.positional("a body")?,
        cwd: parsed.flag("--cwd").map(PathBuf::from),
        description: parsed.flag("--description"),
        kind: parsed
            .flag("--type")
            .map(|value| {
                MemoryType::from_str(&value)
                    .map_err(|_| Failure::Usage(format!("unknown --type `{value}`")))
            })
            .transpose()?,
        name: parsed.flag("--name"),
        session_id: env::var(SESSION_ENV).ok(),
    };

    let data_root = paths::data_root()?;
    let outcome = remember::remember(&data_root, request)?;
    // Read the bank and the type back off what the commit path wrote, rather
    // than restating its defaults here. Never the body: a memory belongs in
    // its bank, not doubled into the day's log.
    let bank = outcome
        .path
        .parent()
        .and_then(Path::file_name)
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or(NONE);
    let kind = outcome.filename.split('_').next().unwrap_or(NONE);
    note(
        Some(&data_root),
        "remember",
        &format!("wrote {} bank={bank} type={kind}", outcome.path.display()),
    );
    println!("{}", outcome.path.display());
    Ok(())
}

/// `dream [--now]`.
pub(super) fn dream_verb(args: &[String]) -> Result<(), Failure> {
    // The flag is documentation: dream always routes what is queued.
    Args::parse(args, "dream", &[], &["--now"], 0)?;
    let data_root = paths::data_root()?;
    let mut options = dream::Options::from_env(&data_root, &paths::claude_root()?);
    let models = knobs(&data_root)?.dream;
    for mind in &mut options.minds {
        if let Some(model) = models.get(mind.tier.as_str()) {
            mind.model.clone_from(model);
        }
    }
    let outcome = dream::dream(&data_root, &paths::home()?, &options)?;
    for path in &outcome.committed {
        println!("{}", path.display());
    }
    eprintln!(
        "dreamt {} pointer(s), {} left for want of a quorum, {} memories committed, \
{} already held",
        outcome.dreamed,
        outcome.skipped,
        outcome.committed.len(),
        outcome.deduped
    );
    Ok(())
}

/// `reflect [--day <yyyy-mm-dd>]`.
pub(super) fn reflect_verb(args: &[String]) -> Result<(), Failure> {
    match Args::parse(args, "reflect", &["--day"], &[], 0)?.flag("--day") {
        Some(value) => reflect_one_day(
            calendar_day(&value)
                .ok_or_else(|| Failure::Usage(format!("--day wants yyyy-mm-dd, got `{value}`")))?,
        ),
        None => reflect_pass(),
    }
}

/// The whole 24 h pass, then the backfill behind it.
fn reflect_pass() -> Result<(), Failure> {
    // The pass already writes one line per bank. What it had no line for is
    // the run: a nightly tick that died halfway through left a log ending in
    // an ordinary bank line, indistinguishable from one that finished.
    let started = Instant::now();
    let (data_root, now, options) = (
        paths::data_root()?,
        Timestamp::now()?,
        reflect::Options::from_env(),
    );
    let outcome = match reflect::reflect(&data_root, &paths::claude_root()?, now, &options) {
        Ok(outcome) => outcome,
        Err(error) => {
            note(
                Some(&data_root),
                "reflect",
                &format!(
                    "reflect error ms={}: {error}",
                    started.elapsed().as_millis()
                ),
            );
            return Err(Failure::Error(error));
        }
    };
    note(
        Some(&data_root),
        "reflect",
        &format!(
            "reflect done banks={} due={} failed={} swept={} entry={} ms={}",
            outcome.banks.len(),
            outcome.due,
            outcome.failed,
            outcome.swept,
            outcome.entry_status,
            started.elapsed().as_millis()
        ),
    );
    println!(
        "{}",
        entry_line(outcome.entry.as_deref(), outcome.entry_status)
    );
    println!("{}", outcome.index.display());
    eprintln!(
        "swept {} pointer(s); {} bank(s) considered",
        outcome.swept,
        outcome.banks.len()
    );
    // A day that fails is passed like a bank that does: the rest still run,
    // and the exit counts it.
    let mut lost = 0;
    reflect::backfill(&data_root, now, |day| {
        reflect_day(&data_root, day, &options)
            .inspect_err(|_| lost += 1)
            .ok()
    });
    match (outcome.failed, lost) {
        (0, 0) => Ok(()),
        (banks, days) => Err(Error::refused(format!(
            "{banks} bank(s) failed upkeep, {days} backfill day(s) failed: see their lines in the reflect run log"
        ))
        .into()),
    }
}

/// One past day's entry and the index behind it, and nothing else: no drain,
/// no sweep, no upkeep, no backfill.
fn reflect_one_day(day: reflect::Day) -> Result<(), Failure> {
    reflect_day(&paths::data_root()?, day, &reflect::Options::from_env())?;
    Ok(())
}

/// Render `day`'s entry and the index behind it, journal the run line and
/// print the entry line — `--day`'s whole work, and the backfill's per day.
fn reflect_day(
    data_root: &Path,
    day: reflect::Day,
    options: &reflect::Options,
) -> Result<&'static str, Error> {
    let started = Instant::now();
    let rendered = reflect::entry_step(data_root, day, options)
        .and_then(|rendered| reflect::write_index(data_root, day).map(|_| rendered));
    let (entry, status) = match rendered {
        Ok(rendered) => rendered,
        Err(error) => {
            note(
                Some(data_root),
                "reflect",
                &format!(
                    "reflect error day={} ms={}: {error}",
                    day.key(),
                    started.elapsed().as_millis()
                ),
            );
            return Err(error);
        }
    };
    note(
        Some(data_root),
        "reflect",
        &format!(
            "reflect done day={} entry={status} ms={}",
            day.key(),
            started.elapsed().as_millis()
        ),
    );
    println!("{}", entry_line(entry.as_deref(), status));
    Ok(status)
}

/// What the entry step has to show for itself: the path when it wrote one,
/// else the word that says why it did not.
fn entry_line(entry: Option<&Path>, status: &str) -> String {
    entry.map_or_else(|| status.to_owned(), |path| path.display().to_string())
}

/// `mcp` — the banks as a tool server, JSON-RPC on stdio.
///
/// The one verb whose whole conversation is stdout, so nothing else may print
/// there for as long as it runs. It takes no arguments: a client launches it
/// with none, and an argument here is a registration to fix rather than
/// something to guess at.
pub(super) fn mcp_verb(args: &[String]) -> Result<(), Failure> {
    Args::parse(args, "mcp", &[], &[], 0)?;
    // The roots resolve here, as they do for every verb: the handlers below
    // take them as arguments and read the environment for nothing.
    let ctx = mcp::Context {
        data_root: paths::data_root()?,
        home: paths::home()?,
        session_id: env::var(SESSION_ENV).ok(),
    };
    // Locked for the whole conversation: one client, one message at a time, and
    // no other writer to interleave with on either stream.
    mcp::serve(&ctx, io::stdin().lock(), io::stdout().lock())
        .map_err(|source| Failure::Error(Error::io("<stdout>", source)))
}

/// `version`.
pub(super) fn version_verb(args: &[String]) -> Result<(), Failure> {
    Args::parse(args, "version", &[], &[], 0)?;
    println!("{VERSION}");
    Ok(())
}

/// `forget <session-id>`.
pub(super) fn forget_verb(args: &[String]) -> Result<(), Failure> {
    let session_id = Args::parse(args, "forget", &[], &[], 1)?.positional("a session id")?;

    // Deliberately unjournaled, and it must stay that way. Every other verb
    // writes what it decided to <root>/.trace; this one destroys every copy of a
    // session, and a line naming the session it destroyed would be precisely
    // the pointer the verb exists to remove.
    for path in forget::forget(&paths::data_root()?, &paths::claude_root()?, &session_id)? {
        println!("{}", path.display());
    }
    Ok(())
}
