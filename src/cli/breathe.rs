//! The `breathe` verbs: the ruach's store, its passes and their ledger.

use std::env;
use std::path::PathBuf;
use std::time::Duration;

use crate::atomic;
use crate::error::Error;
use crate::json::Value;
use crate::paths;
use crate::verbs::{breathe, shift};

use super::{Args, EMPTY, Failure, knob, note, pinned_now, usd_env};

/// `breathe <init|sources|file|list|spike|adopt|drop|pass|attach|close|reweight|tick> …`.
pub(super) fn breathe_verb(args: &[String]) -> Result<(), Failure> {
    let Some(sub) = args.first() else {
        return Err(Failure::Usage("breathe needs a subcommand".to_owned()));
    };
    let rest = &args[1..];
    if rest.iter().any(|arg| arg == "-h" || arg == "--help") {
        return Err(Failure::Help);
    }
    match sub.as_str() {
        "-h" | "--help" => Err(Failure::Help),
        "adopt" => breathe_transition(rest, breathe::Transition::Adopt),
        "attach" => breathe_attach(rest),
        "close" => breathe_close(rest),
        "drop" => breathe_transition(rest, breathe::Transition::Drop),
        "file" => breathe_file(rest),
        "init" => breathe_init(rest),
        "list" => breathe_list(rest),
        "pass" => breathe_pass(rest),
        "reaped" => breathe_reaped(rest),
        "reweight" => breathe_reweight(rest),
        "sources" => breathe_sources(rest),
        "spike" => breathe_transition(rest, breathe::Transition::Spike),
        "tick" => breathe_tick(rest),
        other => Err(Failure::Usage(format!(
            "unknown breathe subcommand `{other}`"
        ))),
    }
}

/// Everything the breathe verbs read from the environment, resolved once.
fn breathe_context() -> Result<breathe::Context, Failure> {
    Ok(breathe::Context {
        data_root: paths::data_root()?,
        now: pinned_now()?,
        prompts: paths::prompts_dir()?,
        shift: env::var("SANDMAN_SHIFT")
            .ok()
            .filter(|shift| !shift.is_empty()),
    })
}

/// Run a breathe verb, journalling a failure before passing it on.
fn breathe_journal<T>(
    ctx: &breathe::Context,
    what: &str,
    result: crate::error::Result<T>,
) -> Result<T, Failure> {
    result.map_err(|error| {
        note(
            Some(&ctx.data_root),
            "breathe",
            &format!("{what} error: {error}"),
        );
        error.into()
    })
}

/// `breathe init`.
fn breathe_init(args: &[String]) -> Result<(), Failure> {
    Args::parse(args, "breathe init", &[], &[], 0)?;
    let ctx = breathe_context()?;
    let made = breathe_journal(&ctx, "init", breathe::init(&ctx.data_root))?;
    note(
        Some(&ctx.data_root),
        "breathe",
        &format!("init made={}", made.len()),
    );
    for path in made {
        println!("{}", path.display());
    }
    Ok(())
}

/// `breathe sources [--due] [--kind skim|deep] [--json]`.
fn breathe_sources(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(
        args,
        "breathe sources",
        &["--kind"],
        &["--due", "--json"],
        0,
    )?;
    let kind = match parsed.flag("--kind") {
        Some(kind) => breathe::PassKind::parse(&kind)?,
        None => breathe::PassKind::Skim,
    };
    let due = (parsed.set("--due") || parsed.flag("--kind").is_some()).then_some(kind);
    let ctx = breathe_context()?;
    let sources = breathe::sources(&ctx, due)?;
    if parsed.set("--json") {
        println!("{}", breathe::sources_json(&sources));
    } else {
        for source in &sources {
            println!("{}", source.line());
        }
    }
    Ok(())
}

/// `breathe file --url U --repo R --title T --problem P --summary S [--source
/// NAME] [--bearing F] [--novelty F] [--spike]`.
fn breathe_file(args: &[String]) -> Result<(), Failure> {
    let what = "breathe file";
    let parsed = Args::parse(
        args,
        what,
        &[
            "--bearing",
            "--novelty",
            "--problem",
            "--repo",
            "--source",
            "--summary",
            "--title",
            "--url",
        ],
        &["--spike"],
        0,
    )?;
    let filing = breathe::Filing {
        bearing: parsed.number("--bearing")?,
        novelty: parsed.number("--novelty")?,
        problem: parsed.required("--problem")?,
        repo: parsed.required("--repo")?,
        source: parsed.flag("--source"),
        spike: parsed.set("--spike"),
        summary: parsed.required("--summary")?,
        title: parsed.required("--title")?,
        url: parsed.required("--url")?,
    };
    if !breathe::valid_repo(&filing.repo) {
        return Err(Failure::Usage(format!(
            "--repo wants `*` or owner/name, got `{}`",
            filing.repo
        )));
    }
    let ctx = breathe_context()?;
    match breathe_journal(&ctx, "file", breathe::file(&ctx, &filing))? {
        breathe::Filed::New(path, event) => {
            note(
                Some(&ctx.data_root),
                "breathe",
                &format!(
                    "filed {} repo={} event={}",
                    path.display(),
                    filing.repo,
                    event.id
                ),
            );
            println!("{}", path.display());
        }
        breathe::Filed::Seen(path) => {
            note(
                Some(&ctx.data_root),
                "breathe",
                &format!("seen_again {}", path.display()),
            );
            println!("seen_again {}", path.display());
        }
    }
    Ok(())
}

/// `breathe list [--repo R] [--state S] [--json]`.
fn breathe_list(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "breathe list", &["--repo", "--state"], &["--json"], 0)?;
    let ctx = breathe_context()?;
    let breaths = breathe::list(
        &ctx.data_root,
        parsed.flag("--repo").as_deref(),
        parsed.flag("--state").as_deref(),
    )?;
    if parsed.set("--json") {
        println!("{}", breathe::list_json(&breaths));
    } else {
        for breath in &breaths {
            println!("{}", breath.line());
        }
    }
    Ok(())
}

/// `breathe spike|adopt|drop <file> [--reason TEXT]`.
fn breathe_transition(args: &[String], to: breathe::Transition) -> Result<(), Failure> {
    let what = format!("breathe {}", to.verb());
    let parsed = Args::parse(args, &what, &["--reason"], &[], 1)?;
    let file = parsed.positional("a breath file")?;
    let reason = parsed.flag("--reason");
    if to == breathe::Transition::Drop
        && reason.as_deref().is_none_or(|text| text.trim().is_empty())
    {
        return Err(Failure::Usage("breathe drop needs --reason".to_owned()));
    }
    let ctx = breathe_context()?;
    let (breath, event) = breathe_journal(
        &ctx,
        to.verb(),
        breathe::transition(&ctx, &file, to, reason.as_deref()),
    )?;
    note(
        Some(&ctx.data_root),
        "breathe",
        &format!(
            "{} {} event={}",
            breath.state,
            breath.file,
            event.as_ref().map_or(EMPTY, |event| event.id.as_str())
        ),
    );
    println!("{}  {}", breath.state, breath.file);
    Ok(())
}

/// `breathe pass --kind skim|deep [--dry-run]`.
fn breathe_pass(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "breathe pass", &["--kind"], &["--dry-run"], 0)?;
    let kind = breathe::PassKind::parse(&parsed.required("--kind")?)?;
    let budget_usd = usd_env("SANDMAN_BREATH_BUDGET_USD", breathe::DEFAULT_BUDGET_USD)?;
    let dead_usd = usd_env(
        "SANDMAN_BREATH_DEAD_COST_USD",
        breathe::DEFAULT_DEAD_COST_USD,
    )?;
    let dry_run = parsed.set("--dry-run");
    let ctx = breathe_context()?;
    // nothing due, in flight and budget spent are waits (1); anything else is a fault (3)
    let composed = breathe_journal(
        &ctx,
        "pass",
        breathe::pass(&ctx, kind, budget_usd, dead_usd, dry_run),
    )
    .map_err(|failure| match failure {
        Failure::Error(error)
            if !matches!(error, Error::Refused { .. } | Error::LockHeld { .. }) =>
        {
            Failure::Fault(error)
        }
        other => other,
    })?;
    if dry_run {
        println!("{}", composed.text);
        return Ok(());
    }
    note(
        Some(&ctx.data_root),
        "breathe",
        &format!(
            "pass {} sources={} chars={}",
            composed.id,
            composed.sources.len(),
            composed.text.chars().count()
        ),
    );
    println!("{}", composed.id);
    Ok(())
}

/// `breathe reaped --id ID`.
fn breathe_reaped(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "breathe reaped", &["--id"], &[], 0)?;
    let id = parsed.required("--id")?;
    let ctx = breathe_context()?;
    breathe_journal(&ctx, "reaped", breathe::reaped(&ctx, &id))
}

/// `breathe attach --id ID [--bg-id X] [--session-id S]`.
fn breathe_attach(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(
        args,
        "breathe attach",
        &["--bg-id", "--id", "--session-id"],
        &[],
        0,
    )?;
    let id = parsed.required("--id")?;
    let (bg_id, session_id) = (parsed.flag("--bg-id"), parsed.flag("--session-id"));
    let ctx = breathe_context()?;
    breathe_journal(
        &ctx,
        "attach",
        breathe::attach(&ctx, &id, bg_id.as_deref(), session_id.as_deref()),
    )?;
    note(
        Some(&ctx.data_root),
        "breathe",
        &format!(
            "attached pass={id} bg={} session={}",
            bg_id.as_deref().unwrap_or(EMPTY),
            session_id.as_deref().unwrap_or(EMPTY)
        ),
    );
    Ok(())
}

/// `breathe close --id ID --read N --filed M --dropped K [--cost F]
/// [--cursor NAME=VALUE]…`.
fn breathe_close(args: &[String]) -> Result<(), Failure> {
    let what = "breathe close";
    let parsed = Args::parse(
        args,
        what,
        &[
            "--cost",
            "--cursor",
            "--dropped",
            "--filed",
            "--id",
            "--read",
        ],
        &[],
        0,
    )?;
    let id = parsed.required("--id")?;
    let cursors = parsed
        .all("--cursor")
        .map(|value| {
            value
                .split_once('=')
                .filter(|(name, _)| !name.is_empty())
                .map(|(name, cursor)| (name.to_owned(), cursor.to_owned()))
                .ok_or_else(|| Failure::Usage(format!("--cursor wants NAME=VALUE, got `{value}`")))
        })
        .collect::<Result<Vec<_>, Failure>>()?;
    let closing = breathe::Closing {
        cost_usd: parsed.number("--cost")?.ok_or_else(|| {
            Failure::Usage(
                "breathe close needs --cost (the session's own cost, or 0 when it cannot read it)"
                    .to_owned(),
            )
        })?,
        cursors,
        dropped: parsed.count("--dropped")?,
        filed: parsed.count("--filed")?,
        read: parsed.count("--read")?,
    };
    let ctx = breathe_context()?;
    let event = breathe_journal(&ctx, "close", breathe::close(&ctx, &id, &closing))?;
    if closing.cost_usd <= 0.0 {
        note(
            Some(&ctx.data_root),
            "breathe",
            &format!("cost zero pass={id}: the session could not read its cost"),
        );
    }
    note(
        Some(&ctx.data_root),
        "breathe",
        &format!(
            "closed pass={id} read={} filed={} dropped={} event={}",
            closing.read, closing.filed, closing.dropped, event.id
        ),
    );
    println!("{}", event.id);
    Ok(())
}

/// `breathe reweight [--dry-run]`.
fn breathe_reweight(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "breathe reweight", &[], &["--dry-run"], 0)?;
    let ctx = breathe_context()?;
    let (changes, event) = breathe_journal(
        &ctx,
        "reweight",
        breathe::reweight(&ctx, parsed.set("--dry-run")),
    )?;
    if changes.is_empty() {
        println!("no change");
    }
    for change in &changes {
        println!("{}", change.line());
    }
    if let Some(event) = event {
        note(
            Some(&ctx.data_root),
            "breathe",
            &format!("reweighted changes={} event={}", changes.len(), event.id),
        );
    }
    Ok(())
}

/// `breathe tick`, its whole run — knobs included — recorded in
/// `breath/ticks.jsonl` with the exit it takes; a line it cannot append is
/// journalled and the exit stands.
fn breathe_tick(args: &[String]) -> Result<(), Failure> {
    let result = breathe_tick_run(args);
    if let (Ok(root), Ok(started)) = (paths::data_root(), pinned_now()) {
        let exit = result.as_ref().err().map_or(0, Failure::code);
        let line = Value::object([
            ("started", Value::string(started.iso8601())),
            ("exit", Value::count(exit.into())),
        ]);
        if let Err(error) = atomic::append_line(&paths::breath_ticks(&root), &line.render()) {
            note(Some(&root), "breathe", &format!("tick unrecorded: {error}"));
        }
    }
    result
}

/// The ruach's agent: close the ended passes' sessions, then open what is
/// due through `$SANDMAN_DISPATCH_BIN`, else `~/.local/bin/dispatch`.
fn breathe_tick_run(args: &[String]) -> Result<(), Failure> {
    Args::parse(args, "breathe tick", &[], &[], 0)?;
    let tick = breathe::Tick {
        budget_usd: usd_env("SANDMAN_BREATH_BUDGET_USD", breathe::DEFAULT_BUDGET_USD)?,
        dead_cost_usd: usd_env(
            "SANDMAN_BREATH_DEAD_COST_USD",
            breathe::DEFAULT_DEAD_COST_USD,
        )?,
        dispatch: env::var_os("SANDMAN_DISPATCH_BIN")
            .filter(|bin| !bin.is_empty())
            .map_or(paths::home()?.join(".local/bin/dispatch"), PathBuf::from),
        mind: env::var("SANDMAN_BREATH_MIND")
            .ok()
            .filter(|mind| !mind.is_empty())
            .unwrap_or_else(|| "opus".to_owned()),
        settings: paths::render(shift::profile("breath").unwrap_or_default())?,
        settle: Duration::from_millis(knob(
            "SANDMAN_BREATH_SETTLE_MS",
            "milliseconds",
            3_000,
            |text| text.parse().ok(),
        )?),
    };
    let ctx = breathe_context()?;
    let result = breathe::tick(&ctx, &tick, &mut |line, failed| {
        note(Some(&ctx.data_root), "breathe", line);
        if failed {
            eprintln!("sandman: {line}");
        }
    });
    breathe_journal(&ctx, "tick", result)
}
