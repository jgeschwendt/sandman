//! `remember` — commit one memory now.
//!
//! The thinnest caller of the commit path: it fills the fields the operator
//! left out and hands the request to [`commit_memory`], which owns everything
//! about the format.

use std::env;
use std::path::{Path, PathBuf};

use crate::bank::Bank;
use crate::commit::{CommitOutcome, CommitRequest, commit_memory};
use crate::error::{Error, Result};
use crate::memory::MemoryType;
use crate::time::Timestamp;

/// How many words of the body become the default name.
pub const NAME_WORDS: usize = 8;

/// A memory to remember. Every `None` is filled in from the body or the
/// process environment.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Remember {
    /// The bank to commit into; default is the bank for `cwd`.
    pub bank: Option<String>,
    /// The memory itself.
    pub body: String,
    /// The working directory the memory belongs to; default is the process's.
    pub cwd: Option<PathBuf>,
    /// The one-line description; default is the body's first line.
    pub description: Option<String>,
    /// The memory type; default is `feedback`.
    pub kind: Option<MemoryType>,
    /// The memory's name; default is the body's first [`NAME_WORDS`] words.
    pub name: Option<String>,
    /// The session that asked, when there is one.
    pub session_id: Option<String>,
}

/// Commit one memory into `data_root`.
pub fn remember(data_root: &Path, request: Remember) -> Result<CommitOutcome> {
    let body = request.body.trim().to_owned();
    if body.is_empty() {
        return Err(Error::invalid("body", request.body));
    }
    let bank = if let Some(bank) = request.bank {
        bank
    } else {
        let cwd = match request.cwd {
            Some(cwd) => cwd,
            None => env::current_dir().map_err(|source| Error::io(".", source))?,
        };
        Bank::key_for(&cwd)
    };
    let now = Timestamp::now()?;
    let commit = CommitRequest {
        description: request
            .description
            .unwrap_or_else(|| first_line(&body).to_owned()),
        kind: request.kind.unwrap_or(MemoryType::Feedback),
        name: request.name.unwrap_or_else(|| default_name(&body)),
        replaces: None,
        source: source_line(now, request.session_id.as_deref()),
        body,
    };
    commit_memory(data_root, &bank, commit)
}

/// The body's first line, whitespace-trimmed. The 150-character index cut
/// happens downstream, in the bank index.
#[must_use]
pub fn first_line(body: &str) -> &str {
    body.lines().next().unwrap_or("").trim()
}

/// The body's first [`NAME_WORDS`] words, whitespace-collapsed.
#[must_use]
pub fn default_name(body: &str) -> String {
    body.split_whitespace()
        .take(NAME_WORDS)
        .collect::<Vec<_>>()
        .join(" ")
}

/// `remember <ISO now>`, plus the session when `$CLAUDE_SESSION_ID` named one.
#[must_use]
pub fn source_line(now: Timestamp, session_id: Option<&str>) -> String {
    match session_id.filter(|session| !session.is_empty()) {
        Some(session) => format!("remember {} · {session}", now.iso8601()),
        None => format!("remember {}", now.iso8601()),
    }
}

#[cfg(test)]
mod tests {
    use super::{Remember, default_name, first_line, remember, source_line};
    use crate::error::Error;
    use crate::memory::{MemoryFile, MemoryType};
    use crate::testutil::TempDir;
    use crate::time::Timestamp;
    use std::fs;
    use std::path::PathBuf;

    fn body_only(body: &str) -> Remember {
        Remember {
            body: body.to_owned(),
            cwd: Some(PathBuf::from("/Users/you/.code/project")),
            ..Remember::default()
        }
    }

    #[test]
    fn the_defaults_are_type_feedback_eight_words_and_the_first_line() {
        let temp = TempDir::new("remember-defaults");
        let outcome = remember(
            temp.path(),
            body_only("one two three four five six seven eight nine ten\n\nA second paragraph.\n"),
        )
        .expect("remember");

        assert_eq!(
            outcome.path.parent().and_then(std::path::Path::file_name),
            Some(std::ffi::OsStr::new("-Users-you--code-project"))
        );
        assert_eq!(
            outcome.filename,
            "feedback_one_two_three_four_five_six_seven_eight.md"
        );
        let file = MemoryFile::parse(&fs::read_to_string(&outcome.path).expect("read"))
            .expect("well-formed");
        assert_eq!(file.name(), Some("one two three four five six seven eight"));
        assert_eq!(
            file.description(),
            Some("one two three four five six seven eight nine ten")
        );
        assert_eq!(file.frontmatter.get("type"), Some("feedback"));
        assert!(
            file.frontmatter
                .get("source")
                .expect("source")
                .starts_with("remember 20")
        );
        assert!(file.body.starts_with("one two three"));
    }

    #[test]
    fn explicit_fields_win_over_every_default() {
        let temp = TempDir::new("remember-explicit");
        let outcome = remember(
            temp.path(),
            Remember {
                bank: Some("-explicit-bank".to_owned()),
                body: "the body".to_owned(),
                cwd: Some(PathBuf::from("/ignored")),
                description: Some("an explicit description".to_owned()),
                kind: Some(MemoryType::Reference),
                name: Some("An Explicit Name".to_owned()),
                session_id: Some("sid-42".to_owned()),
            },
        )
        .expect("remember");

        assert!(
            outcome
                .path
                .ends_with("-explicit-bank/reference_an_explicit_name.md")
        );
        let file = MemoryFile::parse(&fs::read_to_string(&outcome.path).expect("read"))
            .expect("well-formed");
        assert_eq!(file.description(), Some("an explicit description"));
        assert!(
            file.frontmatter
                .get("source")
                .expect("source")
                .ends_with(" · sid-42")
        );
    }

    #[test]
    fn an_empty_body_is_refused_before_anything_is_written() {
        let temp = TempDir::new("remember-empty");
        assert!(matches!(
            remember(temp.path(), body_only("   \n  ")),
            Err(Error::InvalidInput { what: "body", .. })
        ));
        assert!(!temp.path().join("memories").exists());
    }

    #[test]
    fn the_name_and_description_helpers_stand_alone() {
        assert_eq!(default_name(""), "");
        assert_eq!(default_name("only three words"), "only three words");
        assert_eq!(default_name("  a  b\nc\td e f g h i j "), "a b c d e f g h");
        assert_eq!(first_line("  first  \nsecond\n"), "first");
        assert_eq!(first_line(""), "");

        let now = Timestamp::from_unix_seconds(1_786_018_297);
        assert_eq!(source_line(now, None), "remember 2026-08-06T12:11:37Z");
        assert_eq!(
            source_line(now, Some("sid")),
            "remember 2026-08-06T12:11:37Z · sid"
        );
        assert_eq!(source_line(now, Some("")), "remember 2026-08-06T12:11:37Z");
    }
}
