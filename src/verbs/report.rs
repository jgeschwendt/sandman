//! `report` — what the machine did, told to the owner: every shift's
//! `report.md` beside its handoff, and the open `proposal:` threads that wait
//! on his word. Pure over the data root and `~/.threads`: no model, no
//! network, no git.
//!
//! The contract is `docs/BANK-FORMAT.md` § report.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use crate::atomic;
use crate::error::Result;
use crate::hook::SessionStart;
use crate::paths;
use crate::time::Timestamp;
use crate::verbs::score::Week;
use crate::verbs::shift;

/// The headings every report carries, in this order.
pub const HEADINGS: [&str; 5] = [
    "## Worth your eye",
    "## Landed",
    "## Proposals · need your word",
    "## Decisions I made",
    "## Numbers",
];
/// A report's ceiling, and the hook digest's.
pub const REPORT_CHARS: usize = 6_000;

/// Why `text` is not a report, if it is not: over the cap, or a heading
/// missing or out of order.
#[must_use]
pub fn refusal(text: &str) -> Option<String> {
    let length = text.chars().count();
    if length > REPORT_CHARS {
        return Some(format!(
            "{length} characters, over the {REPORT_CHARS}-character cap"
        ));
    }
    // one pass over the lines, so a heading found out of order is missing
    let mut lines = text.lines().map(str::trim_end);
    HEADINGS
        .iter()
        .find(|heading| !lines.any(|line| line == **heading))
        .map(|heading| format!("no `{heading}` heading after the ones before it"))
}

/// An open `proposal:` thread.
pub(crate) struct Proposal {
    /// When the owner approved it, if he has.
    pub(crate) approved: Option<String>,
    /// Whether it still waits on him: `waiting: user` and undecided.
    pub(crate) awaiting: bool,
    /// `proposal:<owner/repo>:<slug>`.
    pub(crate) key: String,
    /// When it was opened.
    pub(crate) opened: String,
    /// Its `# ` title.
    pub(crate) title: String,
}

/// The open threads keyed `proposal:<repo>:…` — every repo's when `None`.
pub(crate) fn proposals(threads: &Path, repo: Option<&str>) -> Vec<Proposal> {
    let prefix = repo.map_or_else(
        || "proposal:".to_owned(),
        |repo| format!("proposal:{repo}:"),
    );
    shift::front_files(threads)
        .into_iter()
        .filter_map(|(_, fields, body)| {
            let field = |key: &str| fields.get(key).map(String::as_str);
            let key = field("key").filter(|key| key.starts_with(&prefix))?;
            (field("state") == Some("open")).then(|| Proposal {
                approved: field("decided")
                    .filter(|_| {
                        field("answer")
                            .is_some_and(|said| said.trim().eq_ignore_ascii_case("approve"))
                    })
                    .map(ToOwned::to_owned),
                awaiting: field("waiting") == Some("user") && field("decided").is_none(),
                key: key.to_owned(),
                opened: field("opened").unwrap_or("-").to_owned(),
                title: shift::title(&body).unwrap_or_else(|| "untitled".to_owned()),
            })
        })
        .collect()
}

/// `lines` as a section body, `none.` when empty.
pub(crate) fn listed(lines: &[String]) -> String {
    if lines.is_empty() {
        "none.".to_owned()
    } else {
        lines.join("\n")
    }
}

/// Which reports `report` reads, by their shift's start.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Select {
    /// Every one.
    All,
    /// Those started at or after the instant.
    Since(Timestamp),
    /// Those started after the read cursor.
    Unread,
    /// Those started in the week.
    Week(Week),
}

/// The read cursor: the start of the newest report the owner has been shown.
#[must_use]
pub fn cursor(data_root: &Path) -> Option<Timestamp> {
    Timestamp::parse_iso8601(
        fs::read_to_string(paths::reports_read(data_root))
            .ok()?
            .trim(),
    )
}

/// Move the read cursor forward to `at`; never back.
pub fn mark_read(data_root: &Path, at: Timestamp) -> Result<()> {
    if cursor(data_root).is_some_and(|read| read >= at) {
        return Ok(());
    }
    atomic::write(
        &paths::reports_read(data_root),
        &format!("{}\n", at.iso8601()),
    )
}

/// What `report` prints, and the start of the newest report in it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Digest {
    /// The newest report's shift start; `None` when no report was selected.
    pub newest: Option<Timestamp>,
    /// The digest.
    pub text: String,
}

/// The digest of the selected reports, newest first, under `cap` characters:
/// whole reports go oldest first, never the Worth-your-eye or Proposals
/// sections.
#[must_use]
pub fn digest(data_root: &Path, threads: &Path, select: Select, cap: usize) -> Digest {
    let read = cursor(data_root);
    let header = match select {
        Select::Week(week) => format!("# Reports in {}", week.name()),
        Select::Since(at) => format!("# Reports since {}", at.iso8601()),
        _ => format!(
            "# Reports since {}",
            read.filter(|_| select == Select::Unread)
                .map_or_else(|| "the beginning".to_owned(), Timestamp::iso8601)
        ),
    };
    let wanted = |at: Timestamp| match select {
        Select::All => true,
        Select::Since(since) => at >= since,
        Select::Unread => read.is_none_or(|read| at > read),
        Select::Week(week) => week.contains(at),
    };
    let reports: Vec<(Timestamp, String, String)> = shift::metas(data_root)
        .into_iter()
        .rev()
        .filter_map(|meta| {
            let at = meta.started.filter(|at| wanted(*at))?;
            let path = paths::shift_dir(data_root, &meta.shift).join("report.md");
            Some((at, meta.shift, fs::read_to_string(path).ok()?))
        })
        .collect();
    let awaiting: Vec<String> = proposals(threads, None)
        .into_iter()
        .filter(|proposal| proposal.awaiting)
        .map(|p| format!("- {} · {} · opened {}", p.key, p.title, p.opened))
        .collect();
    if reports.is_empty() && awaiting.is_empty() {
        return Digest {
            newest: None,
            text: format!("{header}\n\nnothing new.\n"),
        };
    }
    let eye: Vec<String> = reports
        .iter()
        .flat_map(|(_, shift, text)| {
            text.lines()
                .skip_while(|line| line.trim_end() != HEADINGS[0])
                .skip(1)
                .take_while(|line| !line.starts_with("## "))
                .filter(|line| line.starts_with("- "))
                .map(move |line| format!("{} — {shift}", line.trim_end()))
        })
        .collect();
    let render = |kept: usize| {
        let mut text = format!(
            "{header}\n\n## Worth your eye\n{}\n\n## Proposals awaiting you\n{}\n",
            listed(&eye),
            listed(&awaiting)
        );
        for (.., report) in &reports[..kept] {
            let _ = write!(text, "\n---\n\n{}\n", report.trim_end());
        }
        if kept < reports.len() {
            let _ = write!(
                text,
                "\n… {} older report(s) cut at {cap} characters — `sandman report --all` reads them\n",
                reports.len() - kept
            );
        }
        text
    };
    let mut kept = reports.len();
    let mut text = render(kept);
    while kept > 0 && text.chars().count() > cap {
        kept -= 1;
        text = render(kept);
    }
    Digest {
        newest: reports.first().map(|(at, ..)| *at),
        text,
    }
}

/// Whether a starting session is the machine's own — the open shift's, or
/// one working under the data root (breath passes, dream minds) — which the
/// hook leaves unprimed: the report is the owner's.
#[must_use]
pub fn own(data_root: &Path, start: &SessionStart) -> bool {
    start
        .cwd
        .as_deref()
        .is_some_and(|cwd| cwd.starts_with(data_root))
        || start
            .session_id
            .as_deref()
            .is_some_and(|id| shift::session(data_root).as_deref() == Some(id))
}
