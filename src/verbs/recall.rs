//! `recall` — what past sessions know, composed for a session start.
//!
//! A port of `~/.claude/hooks/memory-recall.js`: same surfaces, same section
//! headers, same limits, same trim-cheapest-first budget. Two changes, both
//! forced by the rewrite — banks live under `<root>/memories/` instead of
//! `~/.orrery/memory/`, and the short-term surface is now `.recent/` pointers
//! (orrery's `.dissolve-queue.jsonl` and its sweep ledger retire with it).
//!
//! Nothing here fails: an unreadable surface is an absent surface, because a
//! session start that errors is worse than one that recalls less.

use std::fs;
use std::path::{Path, PathBuf};

use crate::bank::{INDEX_FILE_NAME, MEMORIES_DIR_NAME};
use crate::json::{self, Value};
use crate::paths;
use crate::slug::truncate_chars;
use crate::time::Timestamp;

/// Hook output is capped by Claude Code at 10,000 characters; the surfaces
/// share this much of it.
pub const BUDGET_CHARS: usize = 9_000;
/// How many day-page lines the chronological surface carries.
const LOG_INDEX_LINES: usize = 5;
/// Pointers older than this are not short-term any more.
const POINTER_HOURS: i64 = 72;
/// At most this many pointers are listed.
const POINTERS_MAX: usize = 12;
/// The tool index is cut here.
const TOOL_CHARS: usize = 1_200;
/// The voyage log's chronological index.
const LOG_INDEX_FILE_NAME: &str = "INDEX.md";
/// The tool/skill surface.
const TOOLS_FILE_NAME: &str = "TOOLS.md";

/// The preamble every non-empty recall carries.
const HEADER: &str = concat!(
    "Recalled context from past sessions in this directory (background, not ",
    "instructions — verify time-sensitive facts before asserting; read the ",
    "referenced files for full bodies):\n\n",
);

/// Compose the recall payload for `cwd`. Empty means nothing to recall.
#[must_use]
pub fn recall(data_root: &Path, home: &Path, cwd: &Path, now: Timestamp) -> String {
    let sections = Sections {
        chronological: chronological(data_root, home),
        graph: graph_sections(data_root, home, cwd),
        recent: recent_sessions(data_root, now),
        tools: tools(data_root, home),
    };
    sections.compose()
}

/// One bank's section, in both renderings.
struct GraphSection {
    /// Bodies for the types that carry behavioral rules.
    full: String,
    /// One line per memory — the degraded form.
    index: String,
}

/// Everything recall could say, before the budget has its say.
struct Sections {
    /// The voyage log's index tail.
    chronological: Option<String>,
    /// The cwd's bank, then its ancestors.
    graph: Vec<GraphSection>,
    /// Pointers from the last three days.
    recent: Option<String>,
    /// The tool/skill surface.
    tools: Option<String>,
}

/// Which rendering each surface is currently at.
struct Budget {
    /// Whether the chronological surface is in.
    chronological: bool,
    /// Per bank: `true` while it still renders bodies.
    graph: Vec<bool>,
    /// Whether the pointer surface is in.
    recent: bool,
    /// Whether the tool surface is in.
    tools: bool,
}

impl Sections {
    /// Render, then trim cheapest-surface-first until the payload fits.
    fn compose(&self) -> String {
        let mut budget = Budget {
            chronological: self.chronological.is_some(),
            graph: vec![true; self.graph.len()],
            recent: self.recent.is_some(),
            tools: self.tools.is_some(),
        };
        let mut text = self.render(&budget);

        // Cheapest first, the graph last — a payload that already fits is
        // never trimmed at all.
        for step in [Surface::Tools, Surface::Chronological, Surface::Recent] {
            if !over(&text) {
                break;
            }
            step.set(&mut budget, false);
            text = self.render(&budget);
        }

        let mut floored = false;
        for index in (0..budget.graph.len()).rev() {
            if !over(&text) {
                break;
            }
            budget.graph[index] = false;
            text = self.render(&budget);
            floored = index == 0;
        }

        // The graph is at its floor and cannot give back more, so the leftover
        // would otherwise be wasted: reinstate the trimmed surfaces in reverse
        // order, each only if it still fits.
        if floored {
            for step in [Surface::Recent, Surface::Chronological, Surface::Tools] {
                let had = step.get(&budget);
                step.set(&mut budget, true);
                let candidate = self.render(&budget);
                if over(&candidate) {
                    step.set(&mut budget, had);
                } else {
                    text = candidate;
                }
            }
        }

        truncate_chars(&text, BUDGET_CHARS).to_owned()
    }

    /// The payload at this budget.
    fn render(&self, budget: &Budget) -> String {
        let mut parts: Vec<&str> = Vec::new();
        for (index, section) in self.graph.iter().enumerate() {
            parts.push(if budget.graph[index] {
                &section.full
            } else {
                &section.index
            });
        }
        for (enabled, section) in [
            (budget.recent, self.recent.as_ref()),
            (budget.chronological, self.chronological.as_ref()),
            (budget.tools, self.tools.as_ref()),
        ] {
            if let (true, Some(section)) = (enabled, section) {
                parts.push(section);
            }
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!("{HEADER}{}", parts.join("\n\n"))
        }
    }
}

/// The optional surfaces, as budget switches.
#[derive(Clone, Copy)]
enum Surface {
    /// The voyage log tail.
    Chronological,
    /// The pointer list.
    Recent,
    /// The tool index.
    Tools,
}

impl Surface {
    /// Whether the surface is currently in.
    fn get(self, budget: &Budget) -> bool {
        match self {
            Self::Chronological => budget.chronological,
            Self::Recent => budget.recent,
            Self::Tools => budget.tools,
        }
    }

    /// Switch the surface in or out.
    fn set(self, budget: &mut Budget, enabled: bool) {
        match self {
            Self::Chronological => budget.chronological = enabled,
            Self::Recent => budget.recent = enabled,
            Self::Tools => budget.tools = enabled,
        }
    }
}

/// Whether the payload has outgrown its budget.
fn over(text: &str) -> bool {
    text.chars().count() > BUDGET_CHARS
}

// ─── surface · long-term (the graph) ──────────────────────────────────────

/// One memory as recall sees it. Read leniently: the strict parser is the
/// commit path's, and a hand-broken file must cost its own line, not the bank.
struct Memory {
    /// Everything after the frontmatter.
    body: String,
    /// The one-line description, for index lines.
    description: String,
    /// The filename, for the index line's pointer.
    file: String,
    /// The memory's name.
    name: String,
    /// `pin` | `index` | `mute` — steers rendering independent of type.
    recall: String,
    /// `user` | `feedback` | `project` | `reference`.
    kind: String,
    /// `updated:`, else `created:` — newest first within a rank.
    updated: String,
}

impl Memory {
    /// `### name (type)` plus the body.
    fn full(&self) -> String {
        format!("### {} ({})\n{}", self.name, self.kind, self.body)
    }

    /// The one-line form, with the file to read for the rest.
    fn index(&self, bank: &str) -> String {
        format!(
            "- {} ({}) — {}  [{bank}/{}]",
            self.name, self.kind, self.description, self.file
        )
    }

    /// user/feedback carry behavioral rules and sort first; `recall: pin`
    /// outranks every type.
    fn rank(&self) -> i32 {
        if self.recall == "pin" {
            return -1;
        }
        match self.kind.as_str() {
            "user" => 0,
            "feedback" => 1,
            "project" => 2,
            "reference" => 3,
            _ => 4,
        }
    }
}

/// Split a leading `---` fenced block off the front of a file.
fn split_frontmatter(raw: &str) -> (Option<&str>, &str) {
    let Some(rest) = raw
        .strip_prefix("---\n")
        .or_else(|| raw.strip_prefix("---\r\n"))
    else {
        return (None, raw);
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\n', '\r']) == "---" {
            return (Some(&rest[..offset]), &rest[offset + line.len()..]);
        }
        offset += line.len();
    }
    (None, raw)
}

/// `key: value` lines, quotes stripped — frontmatter as recall reads it.
fn frontmatter_fields(block: &str) -> Vec<(&str, &str)> {
    block
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let colon = line.find(':')?;
            let key = &line[..colon];
            if key.is_empty()
                || !key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            {
                return None;
            }
            let value = line[colon + 1..].trim();
            let value = value.strip_prefix(['"', '\'']).unwrap_or(value);
            let value = value.strip_suffix(['"', '\'']).unwrap_or(value);
            if value.is_empty() {
                None
            } else {
                Some((key, value))
            }
        })
        .collect()
}

/// Read one memory file. `None` when there is nothing to render.
fn parse_memory(raw: &str, file: &str) -> Option<Memory> {
    let (frontmatter, body) = split_frontmatter(raw);
    let fields = frontmatter.map(frontmatter_fields).unwrap_or_default();
    let field = |key: &str| {
        fields
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| *value)
    };
    let body = body.trim().to_owned();
    let name = field("name");
    if name.is_none() && body.is_empty() {
        return None;
    }
    Some(Memory {
        body,
        description: field("description").unwrap_or_default().to_owned(),
        file: file.to_owned(),
        kind: field("type").unwrap_or("reference").to_owned(),
        name: name
            .unwrap_or_else(|| file.strip_suffix(".md").unwrap_or(file))
            .to_owned(),
        // Values outside the trio degrade to type policy.
        recall: match field("recall") {
            Some(value @ ("index" | "mute" | "pin")) => value.to_owned(),
            _ => String::new(),
        },
        updated: field("updated")
            .or_else(|| field("created"))
            .unwrap_or_default()
            .to_owned(),
    })
}

/// Every renderable memory in a bank, in recall order.
fn bank_memories(dir: &Path) -> Vec<Memory> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok()?.file_name().to_str().map(ToOwned::to_owned))
        .filter(|name| {
            crate::bank::is_memory_filename(name)
                && name != INDEX_FILE_NAME
                && !name.starts_with('_')
        })
        .collect();
    names.sort();

    let mut memories: Vec<Memory> = names
        .into_iter()
        .filter_map(|name| {
            let raw = fs::read_to_string(dir.join(&name)).ok()?;
            parse_memory(&raw, &name)
        })
        .filter(|memory| memory.recall != "mute")
        .collect();
    memories.sort_by(|left, right| {
        left.rank()
            .cmp(&right.rank())
            .then_with(|| right.updated.cmp(&left.updated))
    });
    memories
}

/// The cwd's bank first, then its ancestors ascending toward `$HOME` — their
/// memories still apply, more loosely. Matched case-insensitively: the store
/// has casing drift.
fn bank_chain(memories_dir: &Path, home: &Path, cwd: &Path) -> Vec<(String, bool)> {
    let Ok(entries) = fs::read_dir(memories_dir) else {
        return Vec::new();
    };
    let banks: Vec<String> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.file_type().ok()?.is_dir() {
                return None;
            }
            let name = entry.file_name().to_str()?.to_owned();
            (!name.starts_with('.') && !name.starts_with('_')).then_some(name)
        })
        .collect();

    let mut chain: Vec<(String, bool)> = Vec::new();
    let mut dir: &Path = cwd;
    loop {
        let want = crate::bank::Bank::key_for(dir).to_lowercase();
        if let Some(hit) = banks.iter().find(|bank| bank.to_lowercase() == want)
            && !chain.iter().any(|(bank, _)| bank == hit)
        {
            chain.push((hit.clone(), dir == cwd));
        }
        if dir == home {
            break;
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }
    chain
}

/// One section per bank in the chain, both renderings built up front.
fn graph_sections(data_root: &Path, home: &Path, cwd: &Path) -> Vec<GraphSection> {
    let memories_dir = data_root.join(MEMORIES_DIR_NAME);
    bank_chain(&memories_dir, home, cwd)
        .into_iter()
        .filter_map(|(bank, exact)| {
            let dir = memories_dir.join(&bank);
            let memories = bank_memories(&dir);
            if memories.is_empty() {
                return None;
            }
            let label = if exact {
                "this directory's bank"
            } else {
                "ancestor bank"
            };
            let where_from = format!("{}/", paths::tildify(&dir, home));
            // recall wins over type: pin → always full, index → always an
            // index line; else type policy.
            let full: Vec<String> = memories
                .iter()
                .map(|memory| match memory.recall.as_str() {
                    "pin" => memory.full(),
                    "index" => memory.index(&bank),
                    _ if matches!(memory.kind.as_str(), "user" | "feedback") => memory.full(),
                    _ => memory.index(&bank),
                })
                .collect();
            // Degraded, a pinned memory keeps its body above the index lines.
            let index: Vec<String> = memories
                .iter()
                .map(|memory| {
                    if memory.recall == "pin" {
                        memory.full()
                    } else {
                        memory.index(&bank)
                    }
                })
                .collect();
            Some(GraphSection {
                full: format!("## Long-term · {label} · {where_from}\n{}", full.join("\n")),
                index: format!(
                    "## Long-term index · {label} · {where_from}\n{}",
                    index.join("\n")
                ),
            })
        })
        .collect()
}

// ─── surface · short-term (the pointers) ──────────────────────────────────

/// One `.recent/` pointer, as far as recall cares.
struct Pointer {
    /// Where the session ran.
    cwd: String,
    /// When it ended.
    ended: Timestamp,
    /// Its ISO form, verbatim from the pointer.
    ended_iso: String,
    /// The session's first prompt, or its id.
    title: String,
}

/// Pointers younger than [`POINTER_HOURS`], newest first.
fn recent_sessions(data_root: &Path, now: Timestamp) -> Option<String> {
    let dir = paths::recent_dir(data_root);
    let cutoff = now.unix_seconds() - POINTER_HOURS * 3600;
    let mut pointers: Vec<Pointer> = fs::read_dir(&dir)
        .ok()?
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()? != "json" {
                return None;
            }
            let raw = fs::read_to_string(&path).ok()?;
            let value = json::parse(raw.trim()).ok()?;
            let text = |key: &str| value.get(key).and_then(Value::as_str);
            // A pointer with no readable end date cannot be claimed to be
            // inside the window, so it drops out.
            let ended_iso = text("ended")?;
            let ended = Timestamp::parse_iso8601(ended_iso)?;
            if ended.unix_seconds() < cutoff {
                return None;
            }
            let stem = path.file_stem()?.to_str()?.to_owned();
            Some(Pointer {
                cwd: text("cwd").unwrap_or("(unknown cwd)").to_owned(),
                ended,
                ended_iso: ended_iso.to_owned(),
                title: text("title").map_or(stem, ToOwned::to_owned),
            })
        })
        .collect();
    if pointers.is_empty() {
        return None;
    }
    pointers.sort_by(|left, right| right.ended.cmp(&left.ended));
    pointers.truncate(POINTERS_MAX);

    let lines: Vec<String> = pointers
        .iter()
        .map(|pointer| {
            format!(
                "- {} · ended {} · {}",
                pointer.title, pointer.ended_iso, pointer.cwd
            )
        })
        .collect();
    Some(format!("## Recent sessions (3 days)\n{}", lines.join("\n")))
}

// ─── surface · chronological and tools ────────────────────────────────────

/// Frontmatter is for the store's own readers; recall renders bodies.
fn unfront(raw: &str) -> &str {
    let (_, body) = split_frontmatter(raw);
    body.trim()
}

/// The voyage log's most recent day pages.
fn chronological(data_root: &Path, home: &Path) -> Option<String> {
    let dir = data_root.join(paths::LOG_DIR_NAME);
    let raw = fs::read_to_string(dir.join(LOG_INDEX_FILE_NAME)).ok()?;
    // `reflect` writes the index ascending, so the most recent pages are its
    // tail — orrery's index was newest-first, and this is the one line of the
    // port that had to invert with it.
    let all: Vec<&str> = unfront(&raw)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let lines = &all[all.len().saturating_sub(LOG_INDEX_LINES)..];
    if lines.is_empty() {
        return None;
    }
    Some(format!(
        "## Chronological · the voyage log's most recent day pages · {}/\n{}",
        paths::tildify(&dir, home),
        lines.join("\n")
    ))
}

/// The tool/skill surface, once it has content. It is emitted as an
/// HTML-comment placeholder until its scope is settled, so "non-empty" means
/// non-empty after the comments come out.
fn tools(data_root: &Path, home: &Path) -> Option<String> {
    let path = tools_path(data_root);
    let raw = fs::read_to_string(&path).ok()?;
    let body = strip_comments(unfront(&raw));
    let body = body.trim();
    if body.is_empty() {
        return None;
    }
    Some(format!(
        "## Tool index · {}\n{}",
        paths::tildify(&path, home),
        truncate_chars(body, TOOL_CHARS)
    ))
}

/// `<root>/memories/TOOLS.md`.
fn tools_path(data_root: &Path) -> PathBuf {
    data_root.join(MEMORIES_DIR_NAME).join(TOOLS_FILE_NAME)
}

/// Drop `<!-- … -->` blocks.
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::{BUDGET_CHARS, recall, split_frontmatter, strip_comments};
    use crate::bank::Bank;
    use crate::testutil::TempDir;
    use crate::time::Timestamp;
    use std::fs;
    use std::path::PathBuf;

    /// 2026-08-06T12:11:37Z.
    const NOW: i64 = 1_786_018_297;

    /// A fabricated `$HOME` with a data root and a working directory inside
    /// it — the tests never look at the operator's own trees.
    struct Root {
        _temp: TempDir,
        cwd: PathBuf,
        home: PathBuf,
        path: PathBuf,
    }

    impl Root {
        fn new(label: &str) -> Self {
            let temp = TempDir::new(label);
            let home = temp.path().join("home");
            let cwd = home.join("code").join("project");
            let path = home.join(".sandman");
            fs::create_dir_all(&cwd).expect("cwd");
            Self {
                _temp: temp,
                cwd,
                home,
                path,
            }
        }

        fn bank(&self) -> String {
            Bank::key_for(&self.cwd)
        }

        fn parent_bank(&self) -> String {
            Bank::key_for(self.cwd.parent().expect("a parent"))
        }

        fn home_bank(&self) -> String {
            Bank::key_for(&self.home)
        }

        fn memory(&self, bank: &str, file: &str, frontmatter: &str, body: &str) {
            let dir = self.path.join("memories").join(bank);
            fs::create_dir_all(&dir).expect("bank dir");
            fs::write(dir.join(file), format!("---\n{frontmatter}---\n\n{body}"))
                .expect("memory file");
        }

        fn pointer(&self, sid: &str, ended: &str, title: &str, cwd: &str) {
            let dir = self.path.join("memories").join(".recent");
            fs::create_dir_all(&dir).expect("recent dir");
            fs::write(
                dir.join(format!("{sid}.json")),
                format!(
                    r#"{{"archived":"/archive/{sid}.jsonl","cwd":"{cwd}","ended":"{ended}","title":"{title}","highlights":[]}}"#
                ),
            )
            .expect("pointer");
        }

        fn log_index(&self, contents: &str) {
            let dir = self.path.join("log");
            fs::create_dir_all(&dir).expect("log dir");
            fs::write(dir.join("INDEX.md"), contents).expect("log index");
        }

        fn tools(&self, contents: &str) {
            let dir = self.path.join("memories");
            fs::create_dir_all(&dir).expect("memories dir");
            fs::write(dir.join("TOOLS.md"), contents).expect("tools");
        }

        fn recall(&self) -> String {
            recall(
                &self.path,
                &self.home,
                &self.cwd,
                Timestamp::from_unix_seconds(NOW),
            )
        }
    }

    #[test]
    fn an_empty_root_recalls_nothing() {
        assert_eq!(Root::new("recall-empty").recall(), "");
    }

    #[test]
    fn the_cwd_bank_comes_first_then_ancestors_up_to_home() {
        let root = Root::new("recall-chain");
        root.memory(
            &root.bank(),
            "user_here.md",
            "name: here\ndescription: the cwd bank\ntype: user\n",
            "the cwd body\n",
        );
        root.memory(
            &root.parent_bank(),
            "feedback_parent.md",
            "name: parent\ndescription: an ancestor\ntype: feedback\n",
            "the parent body\n",
        );
        root.memory(
            &root.home_bank(),
            "project_home.md",
            "name: home\ndescription: the home bank\ntype: project\n",
            "the home body\n",
        );
        // Outside the ancestor chain: never recalled here.
        root.memory(
            &Bank::key_for(&root.home.join("elsewhere")),
            "user_other.md",
            "name: other\ndescription: another directory\ntype: user\n",
            "the other body\n",
        );

        let text = root.recall();
        let headers: Vec<&str> = text
            .lines()
            .filter(|line| line.starts_with("## "))
            .collect();
        assert_eq!(
            headers,
            [
                format!(
                    "## Long-term · this directory's bank · ~/.sandman/memories/{}/",
                    root.bank()
                ),
                format!(
                    "## Long-term · ancestor bank · ~/.sandman/memories/{}/",
                    root.parent_bank()
                ),
                format!(
                    "## Long-term · ancestor bank · ~/.sandman/memories/{}/",
                    root.home_bank()
                ),
            ]
        );
        assert!(text.starts_with("Recalled context from past sessions"));
        assert!(text.contains("### here (user)\nthe cwd body"));
        assert!(text.contains("### parent (feedback)\nthe parent body"));
        // project/reference degrade to an index line even at full budget.
        assert!(text.contains(&format!(
            "- home (project) — the home bank  [{}/project_home.md]",
            root.home_bank()
        )));
        assert!(!text.contains("the home body"));
        assert!(!text.contains("another directory"));
    }

    #[test]
    fn memories_sort_by_recall_then_type_then_recency() {
        let root = Root::new("recall-order");
        let bank = root.bank();
        for (file, name, kind, updated) in [
            ("reference_r.md", "ref", "reference", "2026-08-01T00:00:00Z"),
            ("user_old.md", "old user", "user", "2026-01-01T00:00:00Z"),
            ("user_new.md", "new user", "user", "2026-08-05T00:00:00Z"),
            ("feedback_f.md", "fb", "feedback", "2026-08-04T00:00:00Z"),
        ] {
            root.memory(
                &bank,
                file,
                &format!("name: {name}\ndescription: d\ntype: {kind}\nupdated: {updated}\n"),
                "body\n",
            );
        }
        root.memory(
            &bank,
            "reference_pinned.md",
            "name: pinned\ndescription: d\ntype: reference\nrecall: pin\nupdated: 2020-01-01T00:00:00Z\n",
            "pinned body\n",
        );
        root.memory(
            &bank,
            "user_muted.md",
            "name: muted\ndescription: d\ntype: user\nrecall: mute\n",
            "muted body\n",
        );
        root.memory(
            &bank,
            "user_indexed.md",
            "name: indexed\ndescription: d\ntype: user\nrecall: index\nupdated: 2026-08-06T00:00:00Z\n",
            "indexed body\n",
        );
        // Not memories: the bank index, and `_`-prefixed files.
        root.memory(&bank, "MEMORY.md", "name: MEMORY index\n", "- nope\n");
        root.memory(&bank, "_reflect.md", "name: reflect\n", "internal\n");

        let text = root.recall();
        let names: Vec<&str> = text
            .lines()
            .filter_map(|line| {
                line.strip_prefix("### ")
                    .or_else(|| line.strip_prefix("- "))
                    .map(|rest| rest.split(" (").next().unwrap_or(rest))
            })
            .collect();
        assert_eq!(
            names,
            ["pinned", "indexed", "new user", "old user", "fb", "ref"]
        );
        assert!(text.contains("### pinned (reference)\npinned body"));
        assert!(!text.contains("muted"));
        assert!(!text.contains("indexed body"));
        assert!(!text.contains("MEMORY index"));
        assert!(!text.contains("internal"));
    }

    #[test]
    fn recent_pointers_follow_the_banks_and_expire_at_seventy_two_hours() {
        let root = Root::new("recall-recent");
        root.memory(
            &root.bank(),
            "user_here.md",
            "name: here\ndescription: d\ntype: user\n",
            "body\n",
        );
        root.pointer("sid-fresh", "2026-08-06T09:00:00Z", "the fresh one", "/a");
        root.pointer("sid-older", "2026-08-04T12:00:00Z", "still inside", "/b");
        root.pointer("sid-stale", "2026-08-03T11:00:00Z", "too old", "/c");
        fs::write(
            root.path.join("memories").join(".recent").join("junk.json"),
            "not json",
        )
        .expect("junk");

        let text = root.recall();
        let recent = text
            .split("## Recent sessions (3 days)\n")
            .nth(1)
            .expect("the recent section");
        assert_eq!(
            recent.lines().collect::<Vec<_>>(),
            [
                "- the fresh one · ended 2026-08-06T09:00:00Z · /a",
                "- still inside · ended 2026-08-04T12:00:00Z · /b",
            ]
        );
        // It follows the long-term sections.
        assert!(text.find("## Long-term") < text.find("## Recent sessions"));
    }

    #[test]
    fn the_recent_section_is_omitted_when_no_pointer_is_young_enough() {
        let root = Root::new("recall-recent-empty");
        root.memory(
            &root.bank(),
            "user_here.md",
            "name: here\ndescription: d\ntype: user\n",
            "body\n",
        );
        root.pointer("sid-stale", "2026-07-01T00:00:00Z", "ancient", "/a");
        assert!(!root.recall().contains("Recent sessions"));
    }

    #[test]
    fn the_log_and_tool_surfaces_come_last_and_only_with_content() {
        let root = Root::new("recall-log-tools");
        root.memory(
            &root.bank(),
            "user_here.md",
            "name: here\ndescription: d\ntype: user\n",
            "body\n",
        );
        // `reflect` writes the index ascending, so the tail is the recent end.
        root.log_index(concat!(
            "---\nname: log\n---\n\n",
            "- [2026-08-01](2026-08-01.md)\n- [2026-08-02](2026-08-02.md)\n\n",
            "- [2026-08-03](2026-08-03.md)\n- [2026-08-04](2026-08-04.md)\n",
            "- [2026-08-05](2026-08-05.md)\n- [2026-08-06](2026-08-06.md)\n"
        ));
        root.tools("---\nname: TOOLS\n---\n\n<!-- placeholder -->\n");

        let text = root.recall();
        assert!(text.contains(concat!(
            "## Chronological · the voyage log's most recent day pages · ",
            "~/.sandman/log/\n- [2026-08-02]"
        )));
        assert!(text.contains("- [2026-08-06](2026-08-06.md)"), "the newest");
        assert!(
            !text.contains("- [2026-08-01](2026-08-01.md)"),
            "only five lines are carried, and the oldest is the one dropped"
        );
        // A comment-only tool index is empty content, so no section.
        assert!(!text.contains("## Tool index"));

        root.tools("---\nname: TOOLS\n---\n\n<!-- x -->\n- stele: the graph\n");
        let text = root.recall();
        assert!(text.contains("## Tool index · ~/.sandman/memories/TOOLS.md\n- stele: the graph"));
        assert!(text.find("## Chronological") < text.find("## Tool index"));
    }

    #[test]
    fn an_oversized_graph_floors_then_reinstates_what_still_fits() {
        let root = Root::new("recall-budget-floor");
        root.memory(
            &root.bank(),
            "user_huge.md",
            "name: huge\ndescription: one enormous memory\ntype: user\n",
            &format!("{}\n", "x".repeat(BUDGET_CHARS)),
        );
        root.pointer("sid-fresh", "2026-08-06T09:00:00Z", "the fresh one", "/a");
        root.log_index("- a day page\n");
        root.tools("- a tool line\n");

        let text = root.recall();
        assert!(text.chars().count() <= BUDGET_CHARS);
        // The only bank is at its floor…
        assert!(text.contains("## Long-term index · this directory's bank"));
        assert!(!text.contains(&"x".repeat(100)));
        // …so the trimmed surfaces come back in reverse order.
        assert!(text.contains("## Recent sessions (3 days)"));
        assert!(text.contains("## Chronological"));
        assert!(text.contains("## Tool index"));
    }

    #[test]
    fn degrading_one_ancestor_is_enough_and_the_cheap_surfaces_stay_out() {
        let root = Root::new("recall-budget-partial");
        root.memory(
            &root.bank(),
            "user_here.md",
            "name: here\ndescription: the cwd memory\ntype: user\n",
            "a short body\n",
        );
        root.memory(
            &root.parent_bank(),
            "user_huge.md",
            "name: huge\ndescription: an enormous ancestor memory\ntype: user\n",
            &format!("{}\n", "x".repeat(BUDGET_CHARS)),
        );
        root.pointer("sid-fresh", "2026-08-06T09:00:00Z", "the fresh one", "/a");
        root.log_index("- a day page\n");
        root.tools("- a tool line\n");

        let text = root.recall();
        assert!(text.chars().count() <= BUDGET_CHARS);
        // The graph never reached its floor, so nothing is reinstated: the
        // cwd bank keeps its bodies and the cheap surfaces stay trimmed.
        assert!(text.contains("## Long-term · this directory's bank"));
        assert!(text.contains("a short body"));
        assert!(text.contains("## Long-term index · ancestor bank"));
        assert!(!text.contains("## Recent sessions"));
        assert!(!text.contains("## Chronological"));
        assert!(!text.contains("## Tool index"));
    }

    #[test]
    fn a_bank_too_large_for_any_budget_is_cut_at_the_ceiling() {
        let root = Root::new("recall-ceiling");
        root.memory(
            &root.bank(),
            "user_huge.md",
            &format!(
                "name: huge\ndescription: {}\ntype: user\n",
                "d".repeat(BUDGET_CHARS * 2)
            ),
            &format!("{}\n", "y".repeat(BUDGET_CHARS * 2)),
        );
        assert_eq!(root.recall().chars().count(), BUDGET_CHARS);
    }

    #[test]
    fn a_broken_memory_file_costs_its_own_line_only() {
        let root = Root::new("recall-broken");
        let dir = root.path.join("memories").join(root.bank());
        fs::create_dir_all(&dir).expect("bank dir");
        fs::write(
            dir.join("user_bodyless.md"),
            "---\nname: bodyless\ntype: user\n---\n",
        )
        .expect("frontmatter only");
        fs::write(dir.join("user_raw.md"), "no frontmatter at all\n").expect("raw file");
        fs::write(
            dir.join("user_quoted.md"),
            "---\nname: \"quoted name\"\ntype: user\n---\n\nq\n",
        )
        .expect("quoted frontmatter");
        fs::write(dir.join("user_empty.md"), "").expect("empty file");

        let text = root.recall();
        assert!(text.contains("### bodyless (user)"));
        assert!(text.contains("### quoted name (user)\nq"));
        // No frontmatter: the filename names it and it reads as a reference.
        assert!(text.contains("- user_raw (reference) — "), "{text}");
        assert!(!text.contains("user_empty"));
    }

    #[test]
    fn the_frontmatter_split_and_comment_strip_stand_alone() {
        assert_eq!(split_frontmatter("no fence").0, None);
        assert_eq!(
            split_frontmatter("---\na: 1\n---\n\nbody\n"),
            (Some("a: 1\n"), "\nbody\n")
        );
        assert_eq!(split_frontmatter("---\nunterminated\n").0, None);
        assert_eq!(strip_comments("a<!--b-->c"), "ac");
        assert_eq!(strip_comments("a<!--b"), "a");
        assert_eq!(strip_comments("plain"), "plain");
    }
}
