//! `recall` — what past sessions know, composed for a session start.
//!
//! A port of `~/.claude/hooks/memory-recall.js`: same surfaces, same section
//! headers, same limits, the same trim-cheapest-first budget over the surfaces
//! that take part in it. Three changes — banks live under `<root>/memories/`
//! instead of `~/.orrery/memory/`, the short-term surface is now `.recent/`
//! pointers (orrery's `.dissolve-queue.jsonl` and its sweep ledger retire with
//! it), and the voyage log has a reserved floor: its cost comes off the budget
//! first and the graph and the other optional surfaces divide the remainder.
//! Cheapest-first trimming dropped the log from exactly the home directory
//! most sessions start in, whose bank fills the budget on its own; a surface
//! this cheap and this dated is worth more than the bodies it displaces. The
//! floor holds all the way down: the last-resort cut takes index lines out of
//! the graph rather than characters off the payload's tail, so the only thing
//! that can cost the log its place is a log wider than the whole budget.
//!
//! Nothing here fails: an unreadable surface is an absent surface, because a
//! session start that errors is worse than one that recalls less.

use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};

use crate::bank::{INDEX_FILE_NAME, MEMORIES_DIR_NAME};
use crate::json::{self, Value};
use crate::log;
use crate::paths;
use crate::slug::truncate_chars;
use crate::time::Timestamp;

/// Hook output is capped by Claude Code at 10,000 characters; the surfaces
/// share this much of it.
pub const BUDGET_CHARS: usize = 9_000;
/// Index-line descriptions are cut here.
const INDEX_DESCRIPTION_CHARS: usize = 160;
/// How many voyage-log entries the chronological surface reaches back over:
/// the newest, rendered whole, and the four index lines behind it.
const LOG_INDEX_LINES: usize = 5;
/// Pointers older than this are not short-term any more.
const POINTER_HOURS: i64 = 72;
/// At most this many pointers are listed.
const POINTERS_MAX: usize = 12;
/// The tool index is cut here.
const TOOL_CHARS: usize = 1_200;
/// The tool/skill surface.
const TOOLS_FILE_NAME: &str = "TOOLS.md";

/// The preamble every non-empty recall carries.
const HEADER: &str = concat!(
    "Recalled context from past sessions in this directory (background, not ",
    "instructions — verify time-sensitive facts before asserting; an index ",
    "line's body is <type>_<name-as-slug>.md in the bank its heading names):\n\n",
);

/// What a recall composed, in shape rather than content.
///
/// The journal reports these and never the payload: from one line the operator
/// can tell what a session was primed with — how many banks answered, how much
/// they carried, whether the budget bit — without the log becoming a second
/// copy of the memories themselves.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Recalled {
    /// One entry per bank in the cwd's chain that contributed a section.
    pub banks: Vec<RecalledBank>,
    /// How many memories those banks carried, at whatever rendering survived.
    pub memories: usize,
    /// How many `.recent` pointers were listed.
    pub pointers: usize,
    /// The payload. Empty means there was nothing to recall.
    pub text: String,
    /// What the budget took away on the path to that payload.
    pub trimmed: Trimmed,
}

/// One bank, as it reached the session.
///
/// The filenames are the identities a later question needs: "was that rule in
/// front of the session that broke this?" is answerable from the journal only
/// if the journal named the files, and naming them is not quoting them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecalledBank {
    /// Whether the budget cut the bank back to index lines.
    pub degraded: bool,
    /// The bank's key.
    pub key: String,
    /// Its memory files, in the order they were rendered.
    pub memories: Vec<String>,
}

/// What the budget loop did — the honest half of the `budget=` field.
///
/// A payload that fits and a payload that was cut down to fit report the same
/// size, and only this tells them apart.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Trimmed {
    /// How many banks ended at their index rendering rather than their bodies.
    pub banks_degraded: usize,
    /// How many rendered lines the ceiling cut took out of the graph. Zero is
    /// the ordinary case; anything else means the payload was still over with
    /// every surface at its floor, and nothing but this says how much never
    /// arrived.
    pub ceiling_lines: usize,
    /// The optional surfaces that did not survive, in the order they went.
    pub sections: Vec<&'static str>,
}

/// Compose the recall payload for `cwd`. Empty means nothing to recall.
#[must_use]
pub fn recall(data_root: &Path, home: &Path, cwd: &Path, now: Timestamp) -> String {
    compose(data_root, home, cwd, now).text
}

/// Compose, and report the shape of what was composed.
#[must_use]
pub fn compose(data_root: &Path, home: &Path, cwd: &Path, now: Timestamp) -> Recalled {
    let recent = recent_sessions(data_root, now);
    let sections = Sections {
        chronological: chronological(data_root, home),
        graph: graph_sections(data_root, home, cwd),
        pointers: recent.as_ref().map_or(0, |(_, count)| *count),
        recent: recent.map(|(section, _)| section),
        tools: tools(data_root, home),
    };
    let (text, budget, ceiling_lines) = sections.compose();
    Recalled {
        banks: sections
            .graph
            .iter()
            .zip(&budget.graph)
            .map(|(section, full)| RecalledBank {
                degraded: !full,
                key: section.key.clone(),
                memories: section.files.clone(),
            })
            .collect(),
        memories: sections.graph.iter().map(|section| section.memories).sum(),
        pointers: sections.pointers,
        trimmed: Trimmed {
            banks_degraded: budget.graph.iter().filter(|full| !**full).count(),
            ceiling_lines,
            sections: sections.dropped(&budget),
        },
        text,
    }
}

/// One bank's section, in both renderings.
struct GraphSection {
    /// Its memory files, in the order they were rendered.
    files: Vec<String>,
    /// Bodies for the types that carry behavioral rules.
    full: String,
    /// One line per memory — the degraded form.
    index: String,
    /// The bank's key.
    key: String,
    /// How many memories the bank contributed, at either rendering.
    memories: usize,
}

/// Everything recall could say, before the budget has its say.
struct Sections {
    /// The voyage log's newest entry, and the index lines behind it. Costed
    /// first: the budget never trims it, only the log itself can be too big.
    chronological: Option<String>,
    /// The cwd's bank, then its ancestors.
    graph: Vec<GraphSection>,
    /// How many pointers [`Self::recent`] lists.
    pointers: usize,
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
    /// The optional surfaces that had content and did not make it in, in the
    /// order the budget went after them.
    fn dropped(&self, budget: &Budget) -> Vec<&'static str> {
        [
            // Chronological is settled before the trim sequence starts —
            // reserved, unless the log outgrew the whole budget by itself.
            (Surface::Chronological, self.chronological.is_some()),
            (Surface::Tools, self.tools.is_some()),
            (Surface::Recent, self.recent.is_some()),
        ]
        .into_iter()
        .filter(|(surface, had)| *had && !surface.get(budget))
        .map(|(surface, _)| surface.name())
        .collect()
    }

    /// Render, then trim cheapest-surface-first until the payload fits — the
    /// chronological surface excepted, whose cost is reserved before the trim
    /// sequence begins.
    ///
    /// The budget it settled on comes back with the text, along with the graph
    /// lines the ceiling cut took: what was cut is not recoverable from the
    /// payload, and it is exactly what the journal has to say for `budget=` to
    /// mean anything.
    fn compose(&self) -> (String, Budget, usize) {
        let mut budget = Budget {
            // The voyage log is never trimmed while it fits under the budget
            // on its own: a dated, page-cheap surface outranks the bodies it
            // costs, and the trim sequence used to drop it from the one cwd
            // whose bank fills the budget alone. The single way out is a log
            // bigger than the entire budget, where reserving it would leave
            // the graph nothing — that drops, and `dropped` names it.
            //
            // "On its own" is the payload it would be the whole of, preamble
            // included: the preamble is not optional, and reserving a section
            // that leaves no room for it would put the ceiling cut back on the
            // log, which is exactly what the floor exists to prevent.
            chronological: self
                .chronological
                .as_ref()
                .is_some_and(|section| !over(&format!("{HEADER}{section}"))),
            graph: vec![true; self.graph.len()],
            recent: self.recent.is_some(),
            tools: self.tools.is_some(),
        };
        let mut text = self.render(&budget);

        // Cheapest first, the graph last — a payload that already fits is
        // never trimmed at all.
        for step in [Surface::Tools, Surface::Recent] {
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
            for step in [Surface::Recent, Surface::Tools] {
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

        // Over even with every surface at its floor, so the graph gives up
        // index lines until the payload fits. The cut lands on the graph and
        // never on the payload's tail: a blunt cut at the end would take the
        // voyage log, the one surface whose cost was reserved, and a reserved
        // floor that the last resort can eat is not a floor.
        //
        // Lines go whole: an index entry cut mid-word is a pointer that names
        // no file, worse to a session than an entry it never saw. The smallest
        // cut that fits is the one taken — binary search, since a wider cut is
        // never a longer payload.
        if !over(&text) {
            return (text, budget, 0);
        }
        let lines = text.lines().count();
        let (mut low, mut high) = (0, self.graph_lines(&budget));
        while low < high {
            let mid = low + (high - low) / 2;
            if over(&self.render_cut(&budget, mid)) {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        // The widest cut always fits: every bank is gone and what is left is
        // the preamble plus the chronological surface, which was reserved only
        // on the condition that the two of them fit together. The other
        // optional surfaces cannot be in — reinstatement admits one only when
        // the payload it makes fits, and this payload did not.
        let text = self.render_cut(&budget, low);
        let cut_lines = lines - text.lines().count();
        (text, budget, cut_lines)
    }

    /// The payload at this budget.
    fn render(&self, budget: &Budget) -> String {
        self.render_cut(budget, 0)
    }

    /// The payload at this budget, with `cut` of the graph's lines gone.
    ///
    /// The cut eats the last bank's tail first and reaches the cwd's own bank
    /// only once every ancestor behind it has nothing left to give — the
    /// ranking the floor loop already established, where the most distant
    /// ancestor is the cheapest thing to lose. A bank cut to nothing loses its
    /// heading with its lines: a heading over no memories names a bank and
    /// says nothing about it.
    ///
    /// Only [`Self::compose`]'s ceiling passes a non-zero cut, and only with
    /// every bank already at its index rendering.
    fn render_cut(&self, budget: &Budget, cut: usize) -> String {
        let mut keep: Vec<usize> = Vec::with_capacity(self.graph.len());
        let mut remaining = cut;
        for (section, full) in self.graph.iter().zip(&budget.graph).rev() {
            let lines = body_lines(rendering(section, *full));
            let taken = remaining.min(lines);
            remaining -= taken;
            keep.push(lines - taken);
        }
        keep.reverse();

        let mut parts: Vec<Cow<'_, str>> = Vec::new();
        for ((section, full), kept) in self.graph.iter().zip(&budget.graph).zip(&keep) {
            let body = rendering(section, *full);
            if *kept == body_lines(body) {
                parts.push(Cow::Borrowed(body));
            } else if *kept > 0 {
                let head: Vec<&str> = body.lines().take(kept + 1).collect();
                parts.push(Cow::Owned(head.join("\n")));
            }
        }
        for (enabled, section) in [
            (budget.recent, self.recent.as_ref()),
            (budget.chronological, self.chronological.as_ref()),
            (budget.tools, self.tools.as_ref()),
        ] {
            if let (true, Some(section)) = (enabled, section) {
                parts.push(Cow::Borrowed(section));
            }
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!("{HEADER}{}", parts.join("\n\n"))
        }
    }

    /// How many lines the graph can give the ceiling cut — every rendered line
    /// but each section's heading, which goes only with the last of them.
    fn graph_lines(&self, budget: &Budget) -> usize {
        self.graph
            .iter()
            .zip(&budget.graph)
            .map(|(section, full)| body_lines(rendering(section, *full)))
            .sum()
    }
}

/// The rendering a bank is at: bodies while it is `full`, index lines after.
fn rendering(section: &GraphSection, full: bool) -> &str {
    if full { &section.full } else { &section.index }
}

/// A section's cuttable lines — everything under its heading.
fn body_lines(section: &str) -> usize {
    section.lines().count().saturating_sub(1)
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
    /// What the journal calls it.
    fn name(self) -> &'static str {
        match self {
            Self::Chronological => "chronological",
            Self::Recent => "recent",
            Self::Tools => "tools",
        }
    }

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

    /// The one-line form: name, type, description. No file pointer — the
    /// bank is named once in the section header and the filename follows
    /// from type and name (the preamble says so); a bank-key-plus-filename
    /// suffix repeated fifty times was a quarter of the budget (measured
    /// 2026-09-09 · the ablation).
    ///
    /// The description is cut to a line's worth here rather than left to the
    /// payload's ceiling: descriptions run as long as whole bodies, and a
    /// handful of those would crowd a hundred short memories out of an index
    /// that exists precisely to name them all.
    fn index(&self) -> String {
        let description = truncate_chars(&self.description, INDEX_DESCRIPTION_CHARS);
        let ellipsis = if description.len() < self.description.len() {
            "…"
        } else {
            ""
        };
        format!("- {} ({}) — {description}{ellipsis}", self.name, self.kind)
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
                    "index" => memory.index(),
                    _ if matches!(memory.kind.as_str(), "user" | "feedback") => memory.full(),
                    _ => memory.index(),
                })
                .collect();
            Some(GraphSection {
                files: memories.iter().map(|memory| memory.file.clone()).collect(),
                full: format!("## Long-term · {label} · {where_from}\n{}", full.join("\n")),
                index: format!(
                    "## Long-term index · {label} · {where_from}\n{}",
                    index_lines(&memories)
                ),
                key: bank.clone(),
                memories: memories.len(),
            })
        })
        .collect()
}

/// A bank at its floor: one line per memory, except a pinned one, which keeps
/// its body.
fn index_lines(memories: &[Memory]) -> String {
    memories
        .iter()
        .map(|memory| {
            if memory.recall == "pin" {
                memory.full()
            } else {
                memory.index()
            }
        })
        .collect::<Vec<String>>()
        .join("\n")
}

/// What the bank at `dir` costs a recall once the budget has cut it back to
/// [`index_lines`] — the floor it cannot go under while it still says anything.
///
/// Upkeep quotes this to the mind reworking the bank. `MEMORY.md` would be the
/// easier number and the wrong one: it counts memories recall mutes and carries
/// none of the type and pointer text recall pays for.
#[must_use]
pub fn index_chars(dir: &Path) -> usize {
    index_lines(&bank_memories(dir)).chars().count()
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

/// Pointers younger than [`POINTER_HOURS`], newest first, and how many.
fn recent_sessions(data_root: &Path, now: Timestamp) -> Option<(String, usize)> {
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
    Some((
        format!("## Recent sessions (3 days)\n{}", lines.join("\n")),
        lines.len(),
    ))
}

// ─── surface · chronological and tools ────────────────────────────────────

/// Frontmatter is for the store's own readers; recall renders bodies.
fn unfront(raw: &str) -> &str {
    let (_, body) = split_frontmatter(raw);
    body.trim()
}

/// The voyage log's latest entries — the newest one whole, then the four
/// index lines before it.
///
/// A line names a day; only the newest entry's prose is worth the budget, and
/// it is the one a session starting now most needs. The lines behind it carry
/// the titles a session can ask for by name.
fn chronological(data_root: &Path, home: &Path) -> Option<String> {
    let dir = paths::log_dir(data_root);
    let raw = fs::read_to_string(dir.join(log::INDEX_FILE_NAME)).ok()?;
    // `reflect` writes the index ascending, so the newest entry is its last
    // line — orrery's index was newest-first, and this is the one line of the
    // port that had to invert with it.
    let all: Vec<&str> = unfront(&raw)
        .lines()
        .map(str::trim_end)
        .filter(|line| line.starts_with("- "))
        .collect();
    let lines = &all[all.len().saturating_sub(LOG_INDEX_LINES)..];
    let (newest, earlier) = lines.split_last()?;
    let header = format!(
        "## Voyage log · the latest entries · {}/",
        paths::tildify(&dir, home)
    );
    // The entry the newest line points at is what carries the prose; without
    // it the section is still worth having, so a missing or unreadable page
    // degrades to the lines alone rather than dropping the surface.
    let Some(body) = newest_body(&dir, newest) else {
        return Some(format!("{header}\n{}", lines.join("\n")));
    };
    let mut parts = vec![header, (*newest).to_owned(), body];
    parts.extend(earlier.iter().map(|line| (*line).to_owned()));
    Some(parts.join("\n"))
}

/// The prose of the entry an index line links to, if there is one to read.
fn newest_body(dir: &Path, line: &str) -> Option<String> {
    let text = fs::read_to_string(linked_entry(dir, line)?).ok()?;
    let body = log::Entry::parse(&text)?.body.trim().to_owned();
    (!body.is_empty()).then_some(body)
}

/// The entry file an index line links to. `reflect` writes the target as a
/// `yyyy-mm-dd.md` page beside `INDEX.md`; anything else — a link out of the
/// directory, a name that is not a page — is not an entry of this log.
fn linked_entry(dir: &Path, line: &str) -> Option<PathBuf> {
    let target = line.split_once("](")?.1.split_once(')')?.0;
    let date = target.strip_suffix(".md")?;
    let dated = date.len() == 10
        && date.bytes().enumerate().all(|(index, byte)| {
            if index == 4 || index == 7 {
                byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        });
    dated.then(|| dir.join(target))
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
    use super::{
        BUDGET_CHARS, INDEX_DESCRIPTION_CHARS, Recalled, compose, recall, split_frontmatter,
        strip_comments,
    };
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

        fn log_entry(&self, date: &str, contents: &str) {
            let dir = self.path.join("log");
            fs::create_dir_all(&dir).expect("log dir");
            fs::write(dir.join(format!("{date}.md")), contents).expect("log entry");
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

        fn compose(&self) -> Recalled {
            compose(
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
        assert!(text.contains("- home (project) — the home bank"), "{text}");
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
            "## Voyage log · the latest entries · ",
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
        assert!(text.find("## Voyage log") < text.find("## Tool index"));
    }

    /// An index whose newest line resolves to an entry on disk.
    fn voyage_log(root: &Root, with_entry: bool) {
        root.log_index(concat!(
            "---\nname: voyage log\nbegan: 2026-08-02\ntype: reference\n---\n\n",
            "- day 1 · [First light](2026-08-02.md) — discovery · 2026-08-02\n",
            "- day 2 · [Two writers, one trunk](2026-08-03.md) — setback · 2026-08-03\n",
        ));
        if with_entry {
            root.log_entry(
                "2026-08-03",
                concat!(
                    "---\ndate: 2026-08-03\nday: 2\nfingerprint: 9f2c\nkind: setback\n",
                    "mind: opus\nposition: day 2 · 3 sessions\nsources: bank/one.md\n",
                    "title: Two writers, one trunk\nwritten: 2026-08-04T03:30:12Z\n---\n\n",
                    "The trunk has two writers and no lock. Next: rebuild the scene.\n",
                ),
            );
        }
    }

    #[test]
    fn the_newest_entry_is_carried_whole_before_the_lines_behind_it() {
        let root = Root::new("recall-log-entry");
        voyage_log(&root, true);

        assert!(root.recall().contains(concat!(
            "## Voyage log · the latest entries · ~/.sandman/log/\n",
            "- day 2 · [Two writers, one trunk](2026-08-03.md) — setback · 2026-08-03\n",
            "The trunk has two writers and no lock. Next: rebuild the scene.\n",
            "- day 1 · [First light](2026-08-02.md) — discovery · 2026-08-02",
        )));
    }

    #[test]
    fn a_newest_entry_that_cannot_be_read_degrades_to_the_index_lines() {
        let root = Root::new("recall-log-missing");
        voyage_log(&root, false);

        let text = root.recall();
        assert!(text.contains(concat!(
            "## Voyage log · the latest entries · ~/.sandman/log/\n",
            "- day 1 · [First light](2026-08-02.md) — discovery · 2026-08-02\n",
            "- day 2 · [Two writers, one trunk](2026-08-03.md) — setback · 2026-08-03",
        )));
        assert!(
            !text.contains("two writers and no lock"),
            "no body to carry"
        );
    }

    #[test]
    fn the_voyage_log_section_is_absent_when_the_index_lists_nothing() {
        let root = Root::new("recall-log-bare");
        root.memory(
            &root.bank(),
            "user_here.md",
            "name: here\ndescription: d\ntype: user\n",
            "body\n",
        );
        root.log_index("---\nname: voyage log\ntype: reference\n---\n\n");

        let text = root.recall();
        assert!(!text.is_empty(), "the bank still recalls");
        assert!(!text.contains("## Voyage log"));
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
        root.log_index("- day 1 · [An entry](2026-08-01.md) — discovery · 2026-08-01\n");
        root.tools("- a tool line\n");

        let composed = root.compose();
        let text = &composed.text;
        assert!(text.chars().count() <= BUDGET_CHARS);
        // The only bank is at its floor…
        assert!(text.contains("## Long-term index · this directory's bank"));
        assert!(!text.contains(&"x".repeat(100)));
        // …so the trimmed surfaces come back in reverse order, and the voyage
        // log — reserved, so never trimmed — was in the whole time.
        assert!(text.contains("## Recent sessions (3 days)"));
        assert!(text.contains("## Voyage log"));
        assert!(text.contains("## Tool index"));
        // And the report says exactly that: one bank degraded, nothing left
        // out in the end — the reinstatement is not a trim.
        assert_eq!(composed.trimmed.banks_degraded, 1);
        assert!(composed.trimmed.sections.is_empty(), "{composed:?}");
        assert_eq!(composed.banks.len(), 1);
        assert!(composed.banks[0].degraded);
        assert_eq!(composed.banks[0].key, root.bank());
        assert_eq!(composed.banks[0].memories, ["user_huge.md"]);
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
        root.log_index("- day 1 · [An entry](2026-08-01.md) — discovery · 2026-08-01\n");
        root.tools("- a tool line\n");

        let composed = root.compose();
        let text = &composed.text;
        assert!(text.chars().count() <= BUDGET_CHARS);
        // The graph never reached its floor, so nothing is reinstated: the
        // cwd bank keeps its bodies and the cheap surfaces stay trimmed. The
        // voyage log is not one of them — its floor was reserved first.
        assert!(text.contains("## Long-term · this directory's bank"));
        assert!(text.contains("a short body"));
        assert!(text.contains("## Long-term index · ancestor bank"));
        assert!(!text.contains("## Recent sessions"));
        assert!(text.contains("## Voyage log"));
        assert!(!text.contains("## Tool index"));
        // The report names both, in the order the budget went after them, and
        // the one bank that had to give up its bodies.
        assert_eq!(composed.trimmed.sections, ["tools", "recent"]);
        assert_eq!(composed.trimmed.banks_degraded, 1);
        assert!(!composed.banks[0].degraded, "the cwd bank kept its bodies");
        assert!(composed.banks[1].degraded, "the ancestor gave them up");
    }

    #[test]
    fn the_voyage_log_keeps_its_floor_while_the_bank_degrades_around_it() {
        let root = Root::new("recall-log-floor");
        // A bank whose bodies alone fill the budget — the shape of the `~`
        // bank on the operator's machine, which used to cost the log its
        // place. Twenty short-index memories: the floor is cheap, the full
        // rendering is not.
        for index in 0..20 {
            root.memory(
                &root.bank(),
                &format!("user_{index:02}.md"),
                &format!("name: memory {index:02}\ndescription: d\ntype: user\n"),
                &format!("{}\n", "x".repeat(600)),
            );
        }
        voyage_log(&root, true);

        let composed = root.compose();
        let text = &composed.text;
        assert!(text.chars().count() <= BUDGET_CHARS);
        // The bank is at its floor…
        assert!(text.contains("## Long-term index · this directory's bank"));
        assert!(!text.contains(&"x".repeat(100)));
        // …and the log arrived whole anyway, newest entry's prose and all.
        assert!(text.contains("## Voyage log"), "{text}");
        assert!(text.contains("The trunk has two writers and no lock."));
        // The report says what it cost: the bank gave up its bodies, and the
        // chronological surface was never a candidate for the trim.
        assert_eq!(composed.trimmed.banks_degraded, 1);
        assert!(
            !composed.trimmed.sections.contains(&"chronological"),
            "{composed:?}"
        );
        assert_eq!(composed.trimmed.ceiling_lines, 0);
    }

    #[test]
    fn a_voyage_log_bigger_than_the_whole_budget_is_the_one_case_it_drops() {
        let root = Root::new("recall-log-oversized");
        root.memory(
            &root.bank(),
            "user_here.md",
            "name: here\ndescription: d\ntype: user\n",
            "a short body\n",
        );
        // The newest entry's body at the cap a mind is held to, and an index
        // line past the whole budget behind it. `reflect` writes neither —
        // titles are capped at 60 characters — but recall reads the log
        // leniently off disk, so a hand-edited page can reach here.
        root.log_index(&format!(
            "---\nname: voyage log\n---\n\n\
- day 1 · [{}](2026-08-02.md) — discovery · 2026-08-02\n\
- day 2 · [Two writers, one trunk](2026-08-03.md) — setback · 2026-08-03\n",
            "t".repeat(BUDGET_CHARS)
        ));
        root.log_entry(
            "2026-08-03",
            &format!(
                "---\ndate: 2026-08-03\nday: 2\nfingerprint: 9f2c\nkind: setback\n\
mind: opus\nposition: day 2 · 3 sessions\nsources: bank/one.md\n\
title: Two writers, one trunk\nwritten: 2026-08-04T03:30:12Z\n---\n\n{}\n",
                "b".repeat(crate::log::BODY_MAX_CHARS)
            ),
        );

        let composed = root.compose();
        // Reserving a surface wider than the budget would leave the graph
        // nothing, so this one drops — and the journal is told.
        assert!(!composed.text.contains("## Voyage log"), "{composed:?}");
        assert!(composed.text.contains("### here (user)\na short body"));
        assert_eq!(composed.trimmed.sections, ["chronological"]);
        assert_eq!(composed.trimmed.banks_degraded, 0);
        assert_eq!(composed.trimmed.ceiling_lines, 0);
    }

    #[test]
    fn a_payload_that_fits_reports_nothing_trimmed_and_names_its_memories() {
        let root = Root::new("recall-untrimmed");
        root.memory(
            &root.bank(),
            "user_here.md",
            "name: here\ndescription: d\ntype: user\n",
            "a short body\n",
        );
        root.memory(
            &root.bank(),
            "reference_there.md",
            "name: there\ndescription: d\ntype: reference\n",
            "another short body\n",
        );
        root.pointer("sid-fresh", "2026-08-06T09:00:00Z", "the fresh one", "/a");

        let composed = root.compose();
        assert_eq!(composed.trimmed, super::Trimmed::default());
        assert_eq!(composed.memories, 2);
        assert_eq!(composed.pointers, 1);
        // Named, never quoted: the filenames are here and the bodies are not.
        assert_eq!(composed.banks.len(), 1);
        assert_eq!(
            composed.banks[0].memories,
            ["user_here.md", "reference_there.md"]
        );
    }

    #[test]
    fn a_bank_too_large_for_any_budget_is_cut_at_the_ceiling() {
        let root = Root::new("recall-ceiling");
        // Many short memories, not one long one: the index rendering is the
        // floor, and enough entries overflow it however short each line is.
        for index in 0..200 {
            root.memory(
                &root.bank(),
                &format!("user_{index:03}.md"),
                &format!(
                    "name: memory {index:03}\ndescription: {}\ntype: user\n",
                    "d".repeat(150)
                ),
                &format!("{}\n", "b".repeat(150)),
            );
        }

        let composed = root.compose();
        let text = &composed.text;
        assert!(text.chars().count() <= BUDGET_CHARS);
        // The last thing a session reads is a whole entry — a pointer it can
        // follow — and never the front half of one.
        let last = text.lines().next_back().expect("a last line");
        assert!(last.starts_with("- "), "{last}");
        assert!(last.ends_with(&"d".repeat(150)), "{last}");
        assert!(composed.trimmed.ceiling_lines > 0, "{composed:?}");
    }

    #[test]
    fn the_ceiling_cut_comes_out_of_the_graph_and_never_out_of_the_voyage_log() {
        let root = Root::new("recall-ceiling-log");
        // Index lines alone past the budget: the graph is at its floor and
        // still over, which is the one place the last-resort cut runs.
        for index in 0..200 {
            root.memory(
                &root.bank(),
                &format!("user_{index:03}.md"),
                &format!(
                    "name: memory {index:03}\ndescription: {}\ntype: user\n",
                    "d".repeat(150)
                ),
                &format!("{}\n", "b".repeat(150)),
            );
        }
        voyage_log(&root, true);

        let composed = root.compose();
        let text = &composed.text;
        assert!(text.chars().count() <= BUDGET_CHARS);
        assert!(text.contains("## Long-term index · this directory's bank"));
        // The log is carried whole — heading, the newest entry's prose, the
        // line behind it — and the graph paid for it.
        assert!(
            text.contains(concat!(
                "## Voyage log · the latest entries · ~/.sandman/log/\n",
                "- day 2 · [Two writers, one trunk](2026-08-03.md) — setback · 2026-08-03\n",
                "The trunk has two writers and no lock. Next: rebuild the scene.\n",
                "- day 1 · [First light](2026-08-02.md) — discovery · 2026-08-02",
            )),
            "{text}"
        );
        assert!(composed.trimmed.ceiling_lines > 0, "{composed:?}");
        assert!(
            !composed.trimmed.sections.contains(&"chronological"),
            "{composed:?}"
        );
    }

    #[test]
    fn an_index_description_longer_than_its_cap_ends_in_an_ellipsis() {
        let root = Root::new("recall-index-description");
        let capped = "d".repeat(INDEX_DESCRIPTION_CHARS);
        let overlong = "e".repeat(INDEX_DESCRIPTION_CHARS + 40);
        // reference renders as an index line even at full budget.
        root.memory(
            &root.bank(),
            "reference_capped.md",
            &format!("name: capped\ndescription: {capped}\ntype: reference\n"),
            "a short body\n",
        );
        root.memory(
            &root.bank(),
            "reference_overlong.md",
            &format!("name: overlong\ndescription: {overlong}\ntype: reference\n"),
            "a short body\n",
        );

        let text = root.recall();
        assert!(
            text.contains(&format!("- capped (reference) — {capped}\n")),
            "{text}"
        );
        assert!(
            text.contains(&format!(
                "- overlong (reference) — {}…",
                "e".repeat(INDEX_DESCRIPTION_CHARS)
            )),
            "{text}"
        );
        assert!(
            !text.contains(&"e".repeat(INDEX_DESCRIPTION_CHARS + 1)),
            "{text}"
        );
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
