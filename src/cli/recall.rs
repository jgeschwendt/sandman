//! `recall`: the banks a session starts with, by hand or from the hook.

use std::env;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::error::Error;
use crate::hook;
use crate::paths;
use crate::slug::truncate_chars;
use crate::time::Timestamp;
use crate::verbs::recall;

use super::{Args, Failure, NONE, PAYLOAD_CHARS, PIPELINE_ENV, note, stdin};

/// How much of a bank's memory list one journal line keeps, before the rest
/// becomes a count. A bank of a hundred files must not be a hundred-file line.
const BANK_CHARS: usize = 2_000;

/// `recall [--cwd PATH] | --hook`.
pub(super) fn recall_verb(args: &[String]) -> Result<(), Failure> {
    let parsed = Args::parse(args, "recall", &["--cwd"], &["--hook"], 0)?;
    let from_hook = parsed.set("--hook");
    let journal = paths::data_root().ok();
    let result = recall_run(
        journal.as_deref(),
        from_hook,
        parsed.flag("--cwd").map(PathBuf::from),
    );
    if from_hook && let Err(Failure::Error(error)) = &result {
        note(journal.as_deref(), "recall", &format!("error: {error}"));
    }
    result
}

/// The whole of `recall` once the command line is understood.
fn recall_run(
    journal: Option<&Path>,
    from_hook: bool,
    mut cwd: Option<PathBuf>,
) -> Result<(), Failure> {
    if env::var(PIPELINE_ENV).as_deref() == Ok("1") {
        note(journal, "recall", "declined pipeline");
        return Ok(());
    }
    // The session the payload names is the only handle that ties this priming
    // to the conversation it went into — and to the `take` line the same id
    // writes when that conversation ends. A recall asked for by hand primes
    // nothing and names nobody, so it carries none.
    let mut session: Option<String> = None;
    if from_hook && cwd.is_none() {
        let payload = stdin()?;
        cwd = match hook::session_start(&payload) {
            Ok(start) => {
                session = start.session_id;
                start.cwd
            }
            Err(error) => {
                note(
                    journal,
                    "recall",
                    &format!(
                        "hook payload unparseable: {error} payload={}",
                        truncate_chars(payload.trim(), PAYLOAD_CHARS)
                    ),
                );
                return Err(Failure::Error(error));
            }
        };
    }
    let session = session.as_deref().unwrap_or(NONE);
    if from_hook {
        note(
            journal,
            "recall",
            &format!(
                "hook session={session} cwd={}",
                cwd.as_deref()
                    .map_or_else(|| NONE.to_owned(), |cwd| cwd.display().to_string())
            ),
        );
    }
    let cwd = match cwd {
        Some(cwd) => cwd,
        None => env::current_dir().map_err(|source| Error::io(".", source))?,
    };

    // Recall runs in front of every session start, so what it costs is a cost
    // the operator pays per session and can otherwise only guess at.
    let started = Instant::now();
    let composed = recall::compose(
        &paths::data_root()?,
        &paths::home()?,
        &cwd,
        Timestamp::now()?,
    );
    let elapsed = started.elapsed().as_millis();
    if composed.text.is_empty() {
        note(journal, "recall", "silent nothing-to-recall");
        return Ok(());
    }
    // The shape of the priming, never its content: a log that quoted the
    // memories back would be a second copy of the banks. `chars=` against
    // `budget=` is the one comparison worth making, so both are in the unit
    // the budget is actually spent in — `trimmed=`, `banks_degraded=` and
    // `cut_lines=` say what it cost to get there, which a size alone cannot.
    let trimmed = composed.trimmed.listed();
    note(
        journal,
        "recall",
        &format!(
            "recalled cwd={} session={session} banks={} memories={} pointers={} \
chars={} budget={} trimmed={trimmed} banks_degraded={} cut_lines={} ms={elapsed}",
            cwd.display(),
            composed.banks.len(),
            composed.memories,
            composed.pointers,
            composed.text.chars().count(),
            recall::BUDGET_CHARS,
            composed.trimmed.banks_degraded,
            composed.trimmed.ceiling_lines,
        ),
    );
    // One line per bank, naming the files that went in. Identities, not
    // bodies: "was that rule in front of the session that broke this?" is a
    // question the shape line cannot answer and this one can.
    for bank in &composed.banks {
        note(
            journal,
            "recall",
            &format!(
                "recalled-bank session={session} bank={} memories={} rendering={}",
                bank.key,
                bank_memories(&bank.memories),
                if bank.degraded { "index" } else { "full" }
            ),
        );
    }
    if from_hook {
        println!("{}", hook::session_start_reply(&composed.text));
    } else {
        println!("{}", composed.text);
    }
    Ok(())
}

/// A bank's memory filenames, cut to one line's worth.
///
/// Whole or not at all would make a large bank's line unreadable and a small
/// bank's line perfect; a count for the tail keeps both honest, and the cut is
/// visible rather than silent.
fn bank_memories(files: &[String]) -> String {
    let mut listed = String::new();
    for (index, file) in files.iter().enumerate() {
        let separator = if index == 0 { "" } else { "," };
        if listed.chars().count() + separator.len() + file.chars().count() > BANK_CHARS {
            let _ = write!(listed, "…+{}", files.len() - index);
            break;
        }
        listed.push_str(separator);
        listed.push_str(file);
    }
    listed
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_long_bank_line_is_cut_with_a_count_for_the_tail() {
        let files = ["a".repeat(1_500), "b".repeat(400), "c".repeat(400)];
        let listed = super::bank_memories(&files);
        assert_eq!(listed, format!("{},{}…+1", files[0], files[1]));
    }
}
