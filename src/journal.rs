//! The journal — one line per decision, where the operator can find it later.
//!
//! Hook-driven verbs decide something and then exit, and until now the
//! deciding left no trace: a `take --hook` that declined said nothing, and a
//! take that fired said it only to a terminal nobody was watching. That is why
//! the 2026-08-26 mid-conversation archive had to be reconstructed out of
//! Claude Code's own `daemon.log`. A verb now writes what it decided to
//! `<root>/log/<verb>-<date>.log` — the convention dream and reflect already
//! keep — so the next incident is answerable from `~/.sandman/log` alone.
//!
//! Best-effort by construction. A journal that could fail would be a new way
//! for a session edge to break, so every error here is swallowed: there is no
//! result to ignore and nothing to unwrap, and a verb behaves identically
//! whether its line landed or not.
//!
//! `forget` is deliberately absent and must stay absent. It is sandman's
//! privacy ending, and a line naming the session it destroyed would be exactly
//! the trace the verb promises not to leave behind.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process;

use crate::paths;
use crate::time::Timestamp;

/// Append one line to the day's log for `verb`.
///
/// The entry is stamped and carries the writing process id: `pid=` is what
/// ties a decision here to the same process in Claude Code's `daemon.log`,
/// which is the correlation the 2026-08-26 forensics had to do by hand.
/// Newlines inside `line` are folded to spaces — one entry is one line, so
/// the log stays greppable however the payload that produced it was shaped.
pub fn note(data_root: &Path, verb: &str, line: &str) {
    try_note(data_root, verb, line);
}

/// The fallible half: `None` at the first thing that did not work.
fn try_note(data_root: &Path, verb: &str, line: &str) -> Option<()> {
    let now = Timestamp::now().ok()?;
    let path = paths::run_log(data_root, verb, now);
    fs::create_dir_all(path.parent()?).ok()?;
    let entry = format!(
        "{} pid={} {}\n",
        now.iso8601(),
        process::id(),
        line.replace(['\n', '\r'], " ")
    );
    // One append, one `write_all`: hook processes overlap freely — a take and
    // the recall of the session replacing it can be in flight together — and a
    // single small write to a file opened for appending is the most atomic
    // thing std offers without a lock the journal has no business holding.
    OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
        .ok()?
        .write_all(entry.as_bytes())
        .ok()
}

#[cfg(test)]
mod tests {
    use super::note;
    use crate::paths;
    use crate::testutil::TempDir;
    use crate::time::Timestamp;
    use std::fs;

    /// Every `.log` the journal wrote under `data_root`, by name.
    fn logs(data_root: &std::path::Path) -> Vec<String> {
        let Ok(entries) = fs::read_dir(paths::log_dir(data_root)) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter_map(|entry| entry.file_name().to_str().map(ToOwned::to_owned))
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_note_lands_in_the_days_log_for_its_verb() {
        let temp = TempDir::new("journal-note");
        let root = temp.path().join(".sandman");
        note(&root, "take", "declined resume session=abc");

        // The log directory is made on the way — nothing seeds it first.
        let names = logs(&root);
        assert_eq!(names.len(), 1, "{names:?}");
        let name = &names[0];
        assert!(name.starts_with("take-"), "{name}");
        assert_eq!(
            std::path::Path::new(name).extension(),
            Some(std::ffi::OsStr::new("log")),
            "{name}"
        );

        let text = fs::read_to_string(paths::log_dir(&root).join(name)).expect("the log");
        assert!(text.ends_with(" declined resume session=abc\n"), "{text}");
        assert!(
            text.contains(&format!(" pid={} ", std::process::id())),
            "{text}"
        );
        // The line opens with the ISO stamp, and it reads back as one.
        let stamp = text.split(' ').next().expect("a stamp");
        assert!(Timestamp::parse_iso8601(stamp).is_some(), "{stamp}");
    }

    #[test]
    fn notes_append_in_order_and_never_span_a_line() {
        let temp = TempDir::new("journal-append");
        let root = temp.path().join(".sandman");
        note(&root, "take", "hook session=one");
        note(&root, "take", "took session=one queue=1");
        // A payload pasted into a line cannot break the one-entry-one-line
        // contract: the newlines fold to spaces.
        note(
            &root,
            "take",
            "hook payload unparseable: x payload={\n  \"a\": 1\n}",
        );
        // A different verb is a different log.
        note(&root, "recall", "silent nothing-to-recall");

        let names = logs(&root);
        assert_eq!(names.len(), 2, "{names:?}");
        let text = fs::read_to_string(paths::log_dir(&root).join(&names[1])).expect("take log");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert!(lines[0].ends_with("hook session=one"), "{}", lines[0]);
        assert!(
            lines[1].ends_with("took session=one queue=1"),
            "{}",
            lines[1]
        );
        assert!(
            lines[2].ends_with(r#"payload={   "a": 1 }"#),
            "{}",
            lines[2]
        );
    }

    #[test]
    fn a_journal_that_cannot_be_written_is_silently_tolerated() {
        let temp = TempDir::new("journal-blocked");
        // A file where the data root's log directory would go: `create_dir_all`
        // cannot win, and the verb must not care.
        let blocked = temp.path().join("blocked");
        fs::write(&blocked, "not a directory").expect("the blocker");
        note(&blocked, "take", "declined pipeline");
        assert!(blocked.is_file(), "the blocker is untouched");

        // And a root nobody can create under at all.
        note(
            std::path::Path::new("/proc/nonexistent/sandman"),
            "take",
            "declined pipeline",
        );
    }
}
