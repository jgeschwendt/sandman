//! The `shift` verbs: the clock, the brief, the landing and the handoff.

use std::env;
use std::path::PathBuf;

use crate::error::Error;
use crate::paths;
use crate::time::Timestamp;
use crate::verbs::shift;

use super::{Args, EMPTY, Failure, note, shift_context, usd_env};

/// `shift <subcommand> …`.
pub(super) fn shift_verb(args: &[String]) -> Result<(), Failure> {
    let Some(sub) = args.first() else {
        return Err(Failure::Usage("shift needs a subcommand".to_owned()));
    };
    let rest = &args[1..];
    if rest.iter().any(|arg| arg == "-h" || arg == "--help") {
        return Err(Failure::Help);
    }
    match sub.as_str() {
        "-h" | "--help" => Err(Failure::Help),
        "attach" => shift_attach(rest),
        "brief" => shift_brief(rest),
        "clock" => shift_clock(rest),
        "handoff" => shift_handoff(rest),
        "land" => shift_land(rest),
        "pick" => shift_pick(rest),
        "pr" => shift_pr(rest),
        "profile" => shift_profile(rest),
        "reaped" => shift_reaped(rest),
        "rotation" => shift_rotation(rest),
        "status" => shift_status(rest),
        other => Err(Failure::Usage(format!(
            "unknown shift subcommand `{other}`"
        ))),
    }
}

/// `shift land --shift ID --branch NAME`.
fn shift_land(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "shift land", &["--branch", "--shift"], &[], 0)?;
    let (id, branch) = (parsed.required("--shift")?, parsed.required("--branch")?);
    let landed = shift::land(&shift_context()?, &id, &branch)?;
    println!(
        "landed {branch} at {} on origin/main {}: {} commit(s); the runner pushes it and opens the pull request after the handoff",
        landed.head, landed.base, landed.commits
    );
    for old in &landed.superseded {
        println!("superseded: {old}, whose pull request the runner closes");
    }
    Ok(())
}

/// `shift pr --shift ID (--number N --url U | --auto BOOL | --red SHA |
/// --merged | --closed)`.
fn shift_pr(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(
        args,
        "shift pr",
        &["--auto", "--number", "--red", "--shift", "--url"],
        &["--closed", "--merged"],
        0,
    )?;
    let id = parsed.required("--shift")?;
    let wants = |flag: &str, value: &str, what: &str| {
        Failure::Usage(format!("{flag} wants {what}, got `{value}`"))
    };
    let mut marks = Vec::new();
    if let Some(auto) = parsed.flag("--auto") {
        let auto = auto
            .parse()
            .map_err(|_| wants("--auto", &auto, "true or false"))?;
        marks.push(shift::Mark::Auto(auto));
    }
    marks.extend(parsed.set("--closed").then_some(shift::Mark::Closed));
    marks.extend(parsed.set("--merged").then_some(shift::Mark::Merged));
    if let Some(number) = parsed.flag("--number") {
        marks.push(shift::Mark::Opened {
            number: number
                .parse()
                .map_err(|_| wants("--number", &number, "a count"))?,
            url: parsed.required("--url")?,
        });
    }
    marks.extend(parsed.flag("--red").map(shift::Mark::Red));
    let [mark] = &marks[..] else {
        return Err(Failure::Usage(
            "shift pr takes one of --number N --url U, --auto BOOL, --red SHA, --merged, --closed"
                .to_owned(),
        ));
    };
    Ok(shift::pr(&shift_context()?, &id, mark)?)
}

/// `shift reaped --shift ID`.
fn shift_reaped(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "shift reaped", &["--shift"], &[], 0)?;
    let id = parsed.required("--shift")?;
    Ok(shift::reaped(&shift_context()?, &id)?)
}

/// `shift attach --shift ID [--bg-id X] [--session-id S]`.
fn shift_attach(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(
        args,
        "shift attach",
        &["--bg-id", "--session-id", "--shift"],
        &[],
        0,
    )?;
    let id = parsed.required("--shift")?;
    let ctx = shift_context()?;
    let (bg_id, session_id) = (parsed.flag("--bg-id"), parsed.flag("--session-id"));
    shift::attach(&ctx, &id, bg_id.as_deref(), session_id.as_deref())?;
    note(
        Some(&ctx.data_root),
        "shift",
        &format!(
            "attached shift={id} bg={} session={}",
            bg_id.as_deref().unwrap_or(EMPTY),
            session_id.as_deref().unwrap_or(EMPTY)
        ),
    );
    Ok(())
}

/// What a brief is composed for: `--mind`, else `$SANDMAN_SHIFT_MIND`, else
/// opus, and `$SANDMAN_SHIFT_BUDGET_USD`, else 15.
fn brief_options(parsed: &Args, dry_run: bool) -> Result<shift::BriefOptions, Failure> {
    let budget_usd = usd_env("SANDMAN_SHIFT_BUDGET_USD", 15.0)?;
    let mind = parsed
        .flag("--mind")
        .or_else(|| {
            env::var("SANDMAN_SHIFT_MIND")
                .ok()
                .filter(|mind| !mind.is_empty())
        })
        .unwrap_or_else(|| "opus".to_owned());
    Ok(shift::BriefOptions {
        budget_usd,
        dry_run,
        mind,
    })
}

/// `shift brief [--mind M] [--dry-run]`.
fn shift_brief(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "shift brief", &["--mind"], &["--dry-run"], 0)?;
    let ctx = shift_context()?;
    let options = brief_options(&parsed, parsed.set("--dry-run"))?;
    let briefed = shift::brief(&ctx, &options);
    let briefed = match briefed {
        Ok(briefed) => briefed,
        Err(error) => {
            note(
                Some(&ctx.data_root),
                "shift",
                &format!("brief error: {error}"),
            );
            return Err(match error {
                Error::Untrusted(_) => Failure::Fault(error),
                error => error.into(),
            });
        }
    };
    if options.dry_run {
        println!("{}", briefed.text);
        return Ok(());
    }
    note(
        Some(&ctx.data_root),
        "shift",
        &format!(
            "briefed shift={} repo={} chars={} trimmed={} event={}",
            briefed.shift,
            briefed.target.label,
            briefed.text.chars().count(),
            if briefed.trimmed.is_empty() {
                EMPTY.to_owned()
            } else {
                briefed.trimmed.join(",")
            },
            briefed
                .event
                .as_ref()
                .map_or(EMPTY, |event| event.id.as_str()),
        ),
    );
    println!("{}", briefed.shift);
    Ok(())
}

/// `shift clock [--check]`.
fn shift_clock(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "shift clock", &[], &["--check"], 0)?;
    let ctx = shift_context()?;
    let clock = shift::read_clock(&ctx).map_err(Failure::Fault)?;
    println!("{}", clock.line());
    if parsed.set("--check")
        && let Some(refusal) = clock.refusal(ctx.min_window_seconds)
    {
        return Err(Error::refused(refusal).into());
    }
    Ok(())
}

/// `shift handoff --shift ID (--from-session | --abandoned [--reason TEXT] | --file PATH)
/// [--cost USD] [--truncate]`.
fn shift_handoff(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(
        args,
        "shift handoff",
        &["--cost", "--file", "--reason", "--shift"],
        &["--abandoned", "--from-session", "--truncate"],
        0,
    )?;
    let cost = parsed
        .flag("--cost")
        .map(|value| {
            value
                .parse::<f64>()
                .ok()
                .filter(|cost| cost.is_finite() && *cost >= 0.0)
                .ok_or_else(|| Failure::Usage(format!("--cost wants USD, got `{value}`")))
        })
        .transpose()?;
    let reason = parsed.flag("--reason");
    let id = parsed.required("--shift")?;
    let file = parsed.flag("--file");
    let abandoned = parsed.set("--abandoned");
    if reason.is_some() && !abandoned {
        return Err(Failure::Usage(
            "shift handoff takes --reason only with --abandoned".to_owned(),
        ));
    }
    let source = match (abandoned, parsed.set("--from-session"), file) {
        (true, false, None) => {
            shift::Source::Abandoned(reason.filter(|reason| !reason.trim().is_empty()))
        }
        (false, true, None) => shift::Source::Session,
        (false, false, Some(file)) => shift::Source::File(PathBuf::from(file)),
        _ => {
            return Err(Failure::Usage(
                "shift handoff takes one of --from-session, --abandoned, --file PATH".to_owned(),
            ));
        }
    };
    let ctx = shift_context()?;
    let truncate = parsed.set("--truncate");
    match shift::handoff(&ctx, &id, &source, truncate, cost) {
        Ok(event) => {
            if cost.is_some_and(|cost| cost <= 0.0) {
                note(
                    Some(&ctx.data_root),
                    "shift",
                    &format!("cost zero shift={id}: the session could not read its cost"),
                );
            }
            note(
                Some(&ctx.data_root),
                "shift",
                &format!("handed off shift={id} event={}", event.id),
            );
            println!("{}", event.id);
            Ok(())
        }
        Err(error) => {
            note(
                Some(&ctx.data_root),
                "shift",
                &format!("handoff shift={id} error: {error}"),
            );
            Err(error.into())
        }
    }
}

/// `shift pick [--path] [--shift ID | --untrusted]`.
fn shift_pick(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(
        args,
        "shift pick",
        &["--shift"],
        &["--path", "--untrusted"],
        0,
    )?;
    let ctx = shift_context()?;
    let targets = match parsed.flag("--shift") {
        _ if parsed.set("--untrusted") => shift::untrusted(&ctx),
        Some(id) => {
            let target = shift::picked(&ctx, &id)?;
            vec![(target.label, target.path)]
        }
        None => {
            let target = shift::pick(&ctx)?
                .ok_or_else(|| Error::not_found("repo to shift", "the rotation"))?;
            vec![(target.label, target.path)]
        }
    };
    for (label, path) in targets {
        if parsed.set("--path") {
            println!("{}", path.display());
        } else {
            println!("{label}");
        }
    }
    Ok(())
}

/// `shift profile [--kind shift|breath]`.
fn shift_profile(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "shift profile", &["--kind"], &[], 0)?;
    let kind = parsed.flag("--kind").unwrap_or_else(|| "shift".to_owned());
    let profile = shift::profile(&kind).ok_or_else(|| {
        Failure::Usage(format!(
            "shift profile --kind wants one of {}, got `{kind}`",
            shift::PROFILE_KINDS.join(", ")
        ))
    })?;
    print!("{}", paths::render(profile)?);
    Ok(())
}

/// `shift rotation [--off REPO]… [--on REPO]…`.
fn shift_rotation(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "shift rotation", &["--off", "--on"], &[], 0)?;
    let ctx = shift_context()?;
    let mut rows = shift::rotation(&ctx)?;
    for (flag, on) in [("--off", false), ("--on", true)] {
        for repo in parsed.all(flag) {
            rows.iter_mut()
                .find(|row| row.label == repo)
                .ok_or_else(|| Failure::Usage(format!("{flag}: `{repo}` is not in the rotation")))?
                .on = on;
        }
    }
    let off = rows
        .iter()
        .filter(|row| !row.on)
        .map(|row| row.label.clone());
    if !parsed.values.is_empty() {
        shift::set_off(&ctx, off)?;
    }
    for row in rows {
        println!(
            "{} · {} · {}{}",
            row.label,
            if row.on { "on" } else { "off" },
            match row.last {
                _ if !row.found => "not found".to_owned(),
                last => last.map_or_else(|| "never shifted".to_owned(), Timestamp::iso8601),
            },
            if row.frozen { " · frozen" } else { "" }
        );
    }
    Ok(())
}

/// `shift status`.
fn shift_status(args: &[String]) -> Result<(), Failure> {
    Args::parse(args, "shift status", &[], &[], 0)?;
    let ctx = shift_context()?;
    match shift::status(&ctx)? {
        Some(current) => println!("{current}"),
        None => println!("no shift open"),
    }
    println!("clock: {}", shift::clock(&ctx).line());
    Ok(())
}
