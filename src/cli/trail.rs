//! `trail` and `reconcile`: the event log and the repos it is checked against.

use crate::error::Error;
use crate::grove;
use crate::paths;
use crate::verbs::{reconcile, trail};

use super::{Args, Failure, calendar_day, grove_roots, note};

/// `trail <subcommand> …`.
pub(super) fn trail_verb(args: &[String]) -> Result<(), Failure> {
    let Some(sub) = args.first() else {
        return Err(Failure::Usage("trail needs a subcommand".to_owned()));
    };
    let rest = &args[1..];
    match sub.as_str() {
        "-h" | "--help" => Err(Failure::Help),
        "append" => trail_append(rest),
        "log" => trail_log(rest),
        "revoke" => trail_revoke(rest),
        "show" => trail_show(rest),
        "today" => {
            Args::parse(rest, "trail today", &[], &[], 0)?;
            println!("{}", trail::today(&paths::data_root()?)?.display());
            Ok(())
        }
        "trailers" => trail_trailers(rest),
        other => Err(Failure::Usage(format!(
            "unknown trail subcommand `{other}`"
        ))),
    }
}

/// `trail append --kind K [flags] | --json '<object>'`.
fn trail_append(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(
        args,
        "trail append",
        &[
            "--branch",
            "--cost",
            "--json",
            "--kind",
            "--model",
            "--parent",
            "--rationale",
            "--ref",
            "--repo",
            "--sha",
            "--shift",
        ],
        &[],
        0,
    )?;
    let mut draft = match parsed.flag("--json") {
        Some(json) => trail::Draft::from_json(&json)
            .map_err(|error| Failure::Usage(format!("--json: {error}")))?,
        None => trail::Draft::default(),
    };
    // Flags land after `--json`, so a flag overrides the object's key.
    for (flag, value) in parsed.values {
        match flag.as_str() {
            "--branch" => draft.branch = Some(value),
            "--cost" => {
                draft.cost_usd = Some(
                    value
                        .parse::<f64>()
                        .ok()
                        .filter(|cost| cost.is_finite() && *cost >= 0.0)
                        .ok_or_else(|| {
                            Failure::Usage(format!("--cost wants USD, got `{value}`"))
                        })?,
                );
            }
            "--kind" => draft.kind = value,
            "--model" => draft.model = Some(value),
            "--parent" => draft.parent = Some(value),
            "--rationale" => draft.rationale = Some(value),
            "--ref" => draft.refs.push(value),
            "--repo" => draft.repo = Some(value),
            "--sha" => draft.sha = Some(value),
            "--shift" => draft.shift = Some(value),
            _ => {}
        }
    }
    if draft.kind.is_empty() {
        return Err(Failure::Usage("trail append needs --kind".to_owned()));
    }
    if !trail::is_kind(&draft.kind) {
        return Err(Failure::Usage(format!("unknown kind `{}`", draft.kind)));
    }
    if draft.kind == "decision" {
        return Err(Failure::Usage(
            "kind `decision` is the owner's answer to a proposal, written by `report serve` only"
                .to_owned(),
        ));
    }
    let data_root = paths::data_root()?;
    let event = trail::append(&data_root, draft)?;
    note(
        Some(&data_root),
        "trail",
        &format!("appended id={} kind={}", event.id, event.draft.kind),
    );
    println!("{}", event.id);
    Ok(())
}

/// `trail log [--since D] [--kind K] [--shift S] [--repo R] [--json]`.
fn trail_log(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(
        args,
        "trail log",
        &["--kind", "--repo", "--shift", "--since"],
        &["--json"],
        0,
    )?;
    let since = parsed.flag("--since");
    if let Some(value) = since
        .as_deref()
        .filter(|value| calendar_day(value).is_none())
    {
        return Err(Failure::Usage(format!(
            "--since wants yyyy-mm-dd, got `{value}`"
        )));
    }
    let entries = trail::read(&paths::data_root()?, since.as_deref())?;
    let matches = |wanted: &str, have: Option<&str>| {
        parsed
            .flag(wanted)
            .is_none_or(|wanted| have == Some(&wanted))
    };
    for entry in &entries {
        let draft = &entry.event.draft;
        if !matches("--kind", Some(&draft.kind))
            || !matches("--shift", draft.shift.as_deref())
            || !matches("--repo", draft.repo.as_deref())
        {
            continue;
        }
        if parsed.set("--json") {
            println!("{}", entry.raw);
        } else {
            println!("{}", entry.event.summary());
        }
    }
    Ok(())
}

/// `trail show <id>`.
fn trail_show(args: &[String]) -> Result<(), Failure> {
    let id = Args::parse(args, "trail show", &[], &[], 1)?.positional("an id")?;
    let entries = trail::read(&paths::data_root()?, None)?;
    let event = trail::find(&entries, &id).ok_or_else(|| Error::not_found("event", id.clone()))?;
    println!("{}", event.pretty());
    let descendants = trail::descendants(&entries, &id);
    println!("\ndescendants:");
    if descendants.is_empty() {
        println!("  -");
    }
    for descendant in &descendants {
        println!("  {}", descendant.summary());
    }
    let revocations = trail::revocations(&entries, &id);
    if !revocations.is_empty() {
        println!("\nrevoked:");
        for revocation in revocations {
            println!("  {}", revocation.summary());
        }
    }
    let mut events: std::collections::BTreeSet<String> =
        descendants.iter().map(|event| event.id.clone()).collect();
    events.insert(id);
    println!("\ncommits:");
    let repos = grove::repos_with_hits(&grove_roots()?, &events, None);
    if repos.is_empty() {
        println!("  -");
    }
    for repo in repos {
        for hit in repo.hits {
            println!("  {}@{}  {}", repo.label, hit.sha, hit.event);
        }
    }
    Ok(())
}

/// `trail revoke <id> --reason TEXT`.
fn trail_revoke(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "trail revoke", &["--reason"], &[], 1)?;
    let id = parsed.positional("an id")?;
    let reason = parsed
        .flag("--reason")
        .filter(|reason| !reason.trim().is_empty())
        .ok_or_else(|| Failure::Usage("trail revoke needs --reason".to_owned()))?;
    let data_root = paths::data_root()?;
    match trail::revoke(&data_root, &id, &reason)? {
        trail::Revoked::Already(earlier) => {
            eprintln!("{id} is already revoked by {}", earlier.id);
            println!("{}", earlier.id);
        }
        trail::Revoked::Now(event) => {
            note(
                Some(&data_root),
                "trail",
                &format!("revoked id={id} by={}", event.id),
            );
            println!("{}", event.id);
        }
    }
    Ok(())
}

/// `trail trailers --shift S --event E`.
fn trail_trailers(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "trail trailers", &["--event", "--shift"], &[], 0)?;
    let (Some(shift), Some(event)) = (parsed.flag("--shift"), parsed.flag("--event")) else {
        return Err(Failure::Usage(
            "trail trailers needs --shift and --event".to_owned(),
        ));
    };
    println!("{}", trail::trailers(&shift, &event));
    Ok(())
}

/// `reconcile <event-id> [--dry-run]`.
pub(super) fn reconcile_verb(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "reconcile", &[], &["--dry-run"], 1)?;
    let (id, dry_run) = (
        parsed.positional("an event or shift id")?,
        parsed.set("--dry-run"),
    );
    let data_root = paths::data_root()?;
    let home = paths::home()?;
    let report = reconcile::reconcile(&data_root, &grove_roots()?, &id, dry_run)?;
    if dry_run {
        println!("dry run — nothing changed");
    }
    println!("revoked: {}", report.revoked.join(" "));
    for repo in &report.repos {
        println!(
            "\n{} ({}) on {} from {}",
            repo.label,
            paths::tildify(&repo.checkout, &home),
            repo.branch,
            if repo.base.is_empty() {
                "-"
            } else {
                &repo.base
            }
        );
        for line in &repo.notes {
            println!("  note: {line}");
        }
        for (label, shas) in [
            ("planned", &repo.planned),
            ("reverted", &repo.reverted),
            ("conflict", &repo.conflicts),
            ("already reverted", &repo.already),
            ("off trunk", &repo.off_trunk),
        ] {
            if dry_run && label == "reverted" {
                continue;
            }
            // a conflicting hit prints under `conflict:` alone
            for sha in shas
                .iter()
                .filter(|sha| label != "planned" || !repo.conflicts.contains(sha))
            {
                println!("  {label}: {sha}");
            }
        }
        if let Some(failed) = &repo.failed {
            println!("  failed: {failed}");
        }
        // the machine never merges its own reconcile: the owner opens the pull request
        if !repo.reverted.is_empty() {
            println!(
                "  pull request (yours to open and merge): cd {} && git push -u origin {branch} && gh pr create --base {} --head {branch} --fill",
                paths::tildify(&repo.checkout, &home),
                repo.base.trim_start_matches("refs/heads/"),
                branch = repo.branch,
            );
        }
    }
    if report.repos.is_empty() {
        println!("\nno commit carries a revoked event");
    }
    println!("\nfor reflect to retire:");
    if report.memories.is_empty() {
        println!("  -");
    }
    for memory in &report.memories {
        println!("  {}", paths::tildify(memory, &home));
    }
    if let Some(event) = &report.event {
        note(
            Some(&data_root),
            "reconcile",
            &format!(
                "reconciled target={id} event={} revoked={} repos={}",
                event.id,
                report.revoked.len(),
                report.repos.len()
            ),
        );
        println!("\nevent: {}", event.id);
    } else if !dry_run {
        println!("\nevent: none — nothing new to revert, the trail is unchanged");
    }
    for repo in &report.repos {
        for line in &repo.notes {
            note(
                Some(&data_root),
                "reconcile",
                &format!("repo={} {line}", repo.label),
            );
        }
    }
    let troubled: Vec<&str> = report
        .repos
        .iter()
        .filter(|repo| repo.troubled())
        .map(|repo| repo.label.as_str())
        .collect();
    if !troubled.is_empty() {
        return Err(Error::refused(format!(
            "reconcile left a conflict or failure in {}",
            troubled.join(", ")
        ))
        .into());
    }
    Ok(())
}
