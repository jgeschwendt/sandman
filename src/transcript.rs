//! Claude Code transcripts: where a session's files are, and the little that
//! `take` reads out of them.
//!
//! A transcript is `~/.claude/projects/<project>/<session>.jsonl` — one JSON
//! object per line, appended live. Sandman never rewrites one: it moves the
//! file whole and derives a pointer from a read-only pass. The pass is
//! deliberately heuristic (the jsonl shape is Claude Code's, not sandman's) and
//! tolerant: an unparseable line costs that line, never the file.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::json::{self, Value};
use crate::paths::PROJECTS_DIR_NAME;

/// Titles are collapsed to one line and cut here.
pub const TITLE_MAX_CHARS: usize = 80;

/// The verb invocations whose quoted bodies become pointer highlights. The
/// legacy form is kept until orrery is decommissioned — old transcripts still
/// carry it, and a highlight is only worth having if it survives the rename.
const HIGHLIGHT_MARKERS: [&str; 2] = ["sandman remember ", "orrery attend "];

/// Every file Claude Code holds for one session.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SessionFiles {
    /// Sidecar directories — `<project>/<session>/`, subagent transcripts.
    pub directories: Vec<PathBuf>,
    /// Transcripts — `<project>/<session>.jsonl`.
    pub transcripts: Vec<PathBuf>,
}

impl SessionFiles {
    /// Whether the session left anything behind.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.directories.is_empty() && self.transcripts.is_empty()
    }
}

/// A session id is a path component and nothing else — it is joined into
/// paths under `~/.claude`, so `..` and separators are refused up front.
pub fn check_session_id(session_id: &str) -> Result<()> {
    let usable = !session_id.is_empty()
        && session_id != "."
        && session_id != ".."
        && session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if usable {
        Ok(())
    } else {
        Err(Error::invalid("session id", session_id))
    }
}

/// Find every file Claude Code holds for `session_id`, across project
/// directories. Missing project trees read as "nothing found".
pub fn find(claude_root: &Path, session_id: &str) -> Result<SessionFiles> {
    check_session_id(session_id)?;
    let projects = claude_root.join(PROJECTS_DIR_NAME);
    let mut found = SessionFiles::default();
    let entries = match fs::read_dir(&projects) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(found),
        Err(source) => return Err(Error::io(&projects, source)),
    };
    let mut project_dirs: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| Error::io(&projects, source))?;
        if entry.path().is_dir() {
            project_dirs.push(entry.path());
        }
    }
    project_dirs.sort();
    for project in project_dirs {
        let transcript = project.join(format!("{session_id}.jsonl"));
        if transcript.is_file() {
            found.transcripts.push(transcript);
        }
        let directory = project.join(session_id);
        if directory.is_dir() {
            found.directories.push(directory);
        }
    }
    Ok(found)
}

/// What a pointer needs from a transcript.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Digest {
    /// The session's working directory — the first line that carries one.
    pub cwd: Option<String>,
    /// Bodies the session asked to remember, in order, deduplicated.
    pub highlights: Vec<String>,
    /// The first user message, collapsed and cut.
    pub title: Option<String>,
}

/// Read a transcript's digest. Unparseable lines are skipped.
#[must_use]
pub fn digest(text: &str) -> Digest {
    let mut digest = Digest::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = json::parse(line) else {
            continue;
        };
        if digest.cwd.is_none() {
            digest.cwd = value
                .get("cwd")
                .and_then(Value::as_str)
                .filter(|cwd| !cwd.is_empty())
                .map(ToOwned::to_owned);
        }
        if digest.title.is_none() {
            digest.title = user_message_title(&value);
        }
        collect_highlights(&value, &mut digest.highlights);
    }
    digest
}

/// How much of a conversation a mind is handed.
pub const EXTRACT_MAX_CHARS: usize = 100_000;
/// Of that, how much comes from the opening — where the session's subject is
/// stated.
pub const EXTRACT_HEAD_CHARS: usize = 20_000;
/// And how much from the end — where the conclusions are.
pub const EXTRACT_TAIL_CHARS: usize = 80_000;

/// The conversation as a mind reads it: the user and assistant text of the
/// transcript, in order, one labelled block each.
///
/// Tool calls, tool results and thinking are left out — dream proposes
/// memories from what was said, and the machinery is noise at this budget.
/// An over-long conversation keeps its head and its tail with the elision
/// marked, because the middle of a long session is the least load-bearing
/// part of it.
#[must_use]
pub fn extract(text: &str) -> String {
    let mut blocks: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = json::parse(line) else {
            continue;
        };
        let Some(role @ ("assistant" | "user")) = value.get("type").and_then(Value::as_str) else {
            continue;
        };
        let Some(said) = message_text(&value) else {
            continue;
        };
        let said = said.trim();
        if !said.is_empty() {
            blocks.push(format!("[{role}] {said}"));
        }
    }
    elide(&blocks.join("\n\n"))
}

/// The spoken text of one transcript line — a plain string content, or every
/// `text` block of a list content, in order.
fn message_text(value: &Value) -> Option<String> {
    let content = value.get("message")?.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(text.to_owned());
    }
    let blocks = content.as_array()?;
    let said: Vec<&str> = blocks
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect();
    if said.is_empty() {
        None
    } else {
        Some(said.join("\n"))
    }
}

/// Cut the middle out of an over-long extract, marking what went.
fn elide(text: &str) -> String {
    let total = text.chars().count();
    if total <= EXTRACT_MAX_CHARS {
        return text.to_owned();
    }
    let head_end = crate::slug::truncate_index(text, EXTRACT_HEAD_CHARS);
    let tail_start = crate::slug::truncate_index(text, total - EXTRACT_TAIL_CHARS);
    let elided = total - EXTRACT_HEAD_CHARS - EXTRACT_TAIL_CHARS;
    format!(
        "{}\n\n… [{elided} characters of the middle elided] …\n\n{}",
        &text[..head_end],
        &text[tail_start..]
    )
}

/// The title carried by a `type: "user"` line, when it has one.
fn user_message_title(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) != Some("user") {
        return None;
    }
    let content = value.get("message")?.get("content")?;
    let text = if let Some(text) = content.as_str() {
        text.to_owned()
    } else if let Some(blocks) = content.as_array() {
        blocks
            .iter()
            .find(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .and_then(|block| block.get("text"))
            .and_then(Value::as_str)?
            .to_owned()
    } else {
        return None;
    };
    let title = one_line(&text, TITLE_MAX_CHARS);
    if title.is_empty() { None } else { Some(title) }
}

/// Collapse whitespace and cut at `max` characters.
#[must_use]
pub fn one_line(text: &str, max: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    crate::slug::truncate_chars(&collapsed, max).to_owned()
}

/// Walk every string in the line and pull the quoted bodies out of any
/// remember invocation it contains.
fn collect_highlights(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            for body in quoted_bodies(text) {
                if !out.contains(&body) {
                    out.push(body);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_highlights(item, out);
            }
        }
        Value::Object(entries) => {
            for (_, item) in entries {
                collect_highlights(item, out);
            }
        }
        _ => {}
    }
}

/// The bodies of `sandman remember "…"` / `orrery attend "…"` inside one
/// string. Both the plain and the shell-escaped (`\"`) quoting survive,
/// because a transcript carries commands as the model typed them.
fn quoted_bodies(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for marker in HIGHLIGHT_MARKERS {
        let mut cursor = 0;
        while let Some(offset) = text[cursor..].find(marker) {
            let after = cursor + offset + marker.len();
            cursor = after;
            // Either quoting can open the body; whichever did also closes it.
            let (rest, shell_escaped) = match text[after..].strip_prefix("\\\"") {
                Some(rest) => (rest, true),
                None => match text[after..].strip_prefix('"') {
                    Some(rest) => (rest, false),
                    None => continue,
                },
            };
            let Some((body, consumed)) = read_body(rest, shell_escaped) else {
                continue;
            };
            cursor = text.len() - rest.len() + consumed;
            let body = body.trim().to_owned();
            if !body.is_empty() {
                out.push(body);
            }
        }
    }
    out
}

/// Read the body up to its closing delimiter. Returns it and how far it read.
fn read_body(text: &str, shell_escaped: bool) -> Option<(String, usize)> {
    if !shell_escaped {
        return read_until_unescaped_quote(text);
    }
    let end = text.find("\\\"")?;
    Some((text[..end].replace("\\\\", "\\"), end + 2))
}

/// Read up to the first `"` that is not backslash-escaped, unescaping `\"`
/// and `\\` on the way. Returns the body and how far it read.
fn read_until_unescaped_quote(text: &str) -> Option<(String, usize)> {
    let mut body = String::new();
    let mut chars = text.char_indices();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '"' => return Some((body, index + 1)),
            '\\' => match chars.next() {
                Some((_, escaped @ ('"' | '\\'))) => body.push(escaped),
                Some((_, other)) => {
                    body.push('\\');
                    body.push(other);
                }
                None => return None,
            },
            other => body.push(other),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        Digest, EXTRACT_HEAD_CHARS, EXTRACT_MAX_CHARS, EXTRACT_TAIL_CHARS, check_session_id,
        digest, extract, find, one_line, quoted_bodies,
    };
    use crate::error::Error;
    use crate::testutil::TempDir;
    use std::fs;

    fn transcript(lines: &[&str]) -> String {
        format!("{}\n", lines.join("\n"))
    }

    #[test]
    fn a_hostile_session_id_never_reaches_the_filesystem() {
        for bad in ["", ".", "..", "../escape", "a/b", "a\\b", "sid;rm"] {
            assert!(
                matches!(check_session_id(bad), Err(Error::InvalidInput { .. })),
                "for {bad:?}"
            );
        }
        check_session_id("aaaabbbb-cccc-dddd-eeee-ffff00001111").expect("a real session id");
    }

    #[test]
    fn find_locates_the_transcript_and_its_sidecar_directory() {
        let temp = TempDir::new("transcript-find");
        let claude = temp.path().join(".claude");
        let project = claude.join("projects").join("-Users-you-code");
        let other = claude.join("projects").join("-Users-you-other");
        fs::create_dir_all(&project).expect("project");
        fs::create_dir_all(&other).expect("other project");
        fs::write(project.join("sid-1.jsonl"), "{}\n").expect("transcript");
        fs::create_dir_all(project.join("sid-1")).expect("sidecar");
        fs::write(other.join("sid-2.jsonl"), "{}\n").expect("other transcript");

        let found = find(&claude, "sid-1").expect("find");
        assert_eq!(found.transcripts, [project.join("sid-1.jsonl")]);
        assert_eq!(found.directories, [project.join("sid-1")]);

        let missing = find(&claude, "sid-3").expect("find");
        assert!(missing.is_empty());
        // A machine with no projects tree is not an error.
        assert!(
            find(temp.path(), "sid-1")
                .expect("find without projects")
                .is_empty()
        );
    }

    #[test]
    fn the_digest_takes_cwd_title_and_highlights_from_the_jsonl() {
        let text = transcript(&[
            r#"{"type":"last-prompt","sessionId":"sid"}"#,
            "not json at all",
            r#"{"type":"attachment","cwd":"/Users/you/code","sessionId":"sid"}"#,
            r#"{"type":"user","cwd":"/elsewhere","message":{"content":"  first   words\nspill over  "}}"#,
            r#"{"type":"user","message":{"content":"a later message"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","input":{"command":"sandman remember \"the first body\""}}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"ran orrery attend \"a legacy body\" earlier"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","input":{"command":"sandman remember \"the first body\""}}]}}"#,
        ]);
        let Digest {
            cwd,
            highlights,
            title,
        } = digest(&text);
        assert_eq!(cwd.as_deref(), Some("/Users/you/code"));
        assert_eq!(title.as_deref(), Some("first words spill over"));
        assert_eq!(highlights, ["the first body", "a legacy body"]);
    }

    #[test]
    fn the_title_comes_from_a_text_block_when_the_content_is_a_list() {
        let text = transcript(&[
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"noise"}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"text","text":"the real prompt"}]}}"#,
        ]);
        assert_eq!(digest(&text).title.as_deref(), Some("the real prompt"));
    }

    #[test]
    fn a_long_title_is_collapsed_and_cut_at_eighty_characters() {
        let long = "word ".repeat(40);
        let text = transcript(&[&format!(
            r#"{{"type":"user","message":{{"content":"{}"}}}}"#,
            long.trim()
        )]);
        let title = digest(&text).title.expect("title");
        assert_eq!(title.chars().count(), 80);
        assert!(title.starts_with("word word"));
        assert_eq!(one_line("  a \n b \t c  ", 80), "a b c");
        assert_eq!(one_line("aéb", 2), "aé");
    }

    #[test]
    fn a_transcript_with_nothing_to_say_yields_an_empty_digest() {
        assert_eq!(digest(""), Digest::default());
        assert_eq!(digest("\n\n  \n"), Digest::default());
    }

    #[test]
    fn the_extract_keeps_only_what_was_said_in_order() {
        let text = transcript(&[
            r#"{"type":"attachment","cwd":"/Users/you"}"#,
            "not json at all",
            r#"{"type":"user","message":{"content":"  port the memory verbs  "}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"on it"},{"type":"tool_use","input":{"command":"ls"}}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","input":{"command":"ls"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"noise"}]}}"#,
            r#"{"type":"user","message":{"content":"   "}}"#,
            r#"{"type":"user","message":{"content":[{"type":"text","text":"ship it"}]}}"#,
        ]);
        assert_eq!(
            extract(&text),
            "[user] port the memory verbs\n\n[assistant] on it\n\n[user] ship it"
        );
        assert_eq!(extract(""), "");
    }

    #[test]
    fn an_over_long_extract_keeps_its_head_and_its_tail() {
        // One block, well past the cap: 300,000 characters.
        let body = "x".repeat(300_000);
        let text = transcript(&[&format!(
            r#"{{"type":"user","message":{{"content":"{body}"}}}}"#
        )]);
        let extracted = extract(&text);
        let head = format!("[user] {}", "x".repeat(EXTRACT_HEAD_CHARS - 7));
        assert!(extracted.starts_with(&head), "head not kept");
        assert!(extracted.ends_with(&"x".repeat(EXTRACT_TAIL_CHARS)), "tail");
        assert!(
            extracted.contains("characters of the middle elided"),
            "the elision is unmarked"
        );
        // The marker is the only thing over the cap.
        assert!(extracted.chars().count() < EXTRACT_MAX_CHARS + 100);

        // Exactly at the cap: untouched.
        let exact = "y".repeat(EXTRACT_MAX_CHARS - 7);
        let text = transcript(&[&format!(
            r#"{{"type":"user","message":{{"content":"{exact}"}}}}"#
        )]);
        let extracted = extract(&text);
        assert_eq!(extracted.chars().count(), EXTRACT_MAX_CHARS);
        assert!(!extracted.contains("elided"));
    }

    #[test]
    fn highlight_scanning_handles_both_quotings_and_ignores_the_unquoted() {
        assert_eq!(
            quoted_bodies(r#"sandman remember "plain body""#),
            ["plain body"]
        );
        assert_eq!(
            quoted_bodies(r#"sandman remember \"escaped body\" --type user"#),
            ["escaped body"]
        );
        assert_eq!(
            quoted_bodies(r#"orrery attend "one" && sandman remember "two""#),
            ["two", "one"]
        );
        assert_eq!(
            quoted_bodies(r#"sandman remember "a \"quoted\" word""#),
            [r#"a "quoted" word"#]
        );
        assert!(quoted_bodies("sandman remember --help").is_empty());
        assert!(quoted_bodies(r#"sandman remember "unterminated"#).is_empty());
    }
}
