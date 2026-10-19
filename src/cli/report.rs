//! `report` and `score`: the week read back, on stdout or served.

use std::env;
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;

use crate::error::Error;
use crate::hook;
use crate::paths;
use crate::time::Timestamp;
use crate::verbs::{report, score};

use super::{Args, Failure, note, shift_context, stdin};

/// `report [--since T | --week W | --all] [--mark-read] | --hook | serve …`.
pub(super) fn report_verb(args: &[String]) -> Result<(), Failure> {
    if args.first().is_some_and(|arg| arg == "serve") {
        return report_serve(&args[1..]);
    }
    let parsed = Args::parse(
        args,
        "report",
        &["--since", "--week"],
        &["--all", "--hook", "--mark-read"],
        0,
    )?;
    let since = parsed
        .flag("--since")
        .map(|text| {
            Timestamp::parse_iso8601(&text)
                .ok_or_else(|| Failure::Usage(format!("--since wants RFC 3339, got `{text}`")))
        })
        .transpose()?;
    let week = parsed
        .flag("--week")
        .map(|text| {
            score::Week::parse(&text)
                .ok_or_else(|| Failure::Usage(format!("--week wants yyyy-Www, got `{text}`")))
        })
        .transpose()?;
    let select = match (since, week, parsed.set("--all")) {
        (None, None, false) => report::Select::Unread,
        (Some(at), None, false) => report::Select::Since(at),
        (None, Some(week), false) => report::Select::Week(week),
        (None, None, true) => report::Select::All,
        _ => {
            return Err(Failure::Usage(
                "report takes one of --since, --week, --all".to_owned(),
            ));
        }
    };
    let hook = parsed.set("--hook");
    if hook && (select != report::Select::Unread || parsed.set("--mark-read")) {
        return Err(Failure::Usage(
            "report --hook takes no other flag".to_owned(),
        ));
    }
    let data_root = paths::data_root()?;
    let threads = paths::docket_dir(&paths::home()?);
    if hook {
        let start = hook::session_start(&stdin()?)?;
        if report::own(&data_root, &start) {
            note(
                Some(&data_root),
                "report",
                "hook declined the machine's own session",
            );
            return Ok(());
        }
        // never the cursor: a shift's session can start before `attach` names
        // it, and only the page the owner opens marks reports read
        let digest = report::digest(&data_root, &threads, select, report::REPORT_CHARS);
        if digest.newest.is_some() {
            println!("{}", hook::session_start_reply(&digest.text));
        }
        return Ok(());
    }
    let digest = report::digest(&data_root, &threads, select, usize::MAX);
    print!("{}", digest.text);
    if parsed.set("--mark-read")
        && let Some(newest) = digest.newest
    {
        report::mark_read(&data_root, newest)?;
    }
    Ok(())
}

/// `report serve [--bind ADDR] [--requests N]`.
fn report_serve(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "report serve", &["--bind", "--requests"], &[], 0)?;
    let bind = parsed
        .flag("--bind")
        .unwrap_or_else(|| report::serve::BIND.to_owned());
    let address = bind
        .parse::<SocketAddr>()
        .ok()
        .filter(|address| address.ip().is_loopback())
        .ok_or_else(|| Failure::Usage(format!("--bind wants a loopback address, got `{bind}`")))?;
    let requests = parsed
        .flag("--requests")
        .map(|text| {
            text.parse()
                .map_err(|_| Failure::Usage(format!("--requests wants a count, got `{text}`")))
        })
        .transpose()?;
    let ctx = shift_context()?;
    let server = report::serve::Server {
        docket: env::var_os("SANDMAN_DOCKET_BIN")
            .filter(|bin| !bin.is_empty())
            .map_or(ctx.home.join(".local/bin/docket"), PathBuf::from),
        pinned: env::var_os("SANDMAN_NOW").is_some_and(|now| !now.is_empty()),
        requests,
        ctx,
    };
    let listener = TcpListener::bind(address).map_err(|source| Error::io(&bind, source))?;
    report::serve::serve(&listener, &server);
    Ok(())
}

/// `score [--week YYYY-Www] [--json]`.
pub(super) fn score_verb(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "score", &["--week"], &["--json"], 0)?;
    let ctx = shift_context()?;
    let week = match parsed.flag("--week") {
        None => score::Week::of(ctx.now),
        Some(text) => score::Week::parse(&text)
            .ok_or_else(|| Failure::Usage(format!("--week wants yyyy-Www, got `{text}`")))?,
    };
    let scored = score::score(&ctx, &paths::routines_state()?, week)?;
    let path = score::write(&ctx, &scored)?;
    note(
        Some(&ctx.data_root),
        "score",
        &format!("scored week={} path={}", week.name(), path.display()),
    );
    if parsed.set("--json") {
        println!("{}", scored.json());
    } else {
        println!("{}", path.display());
    }
    Ok(())
}
