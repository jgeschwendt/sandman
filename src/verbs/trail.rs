//! `trail` — the append-only event log the machine answers to.
//!
//! One JSONL file per UTC day at `<root>/trail/<yyyy>/<mm>/<dd>.jsonl`, one
//! event per line, keys alphabetical. Lines are only ever added: a revoke is
//! a new `revoke` event naming its target as `parent`, never an edit. Every
//! append holds `.commit.lock` and rewrites the day's file tmp → rename, so a
//! reader never sees half a line.
//!
//! The contract is `docs/BANK-FORMAT.md` § trail.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::hash::{BuildHasher as _, RandomState};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::atomic;
use crate::error::{Error, Result};
use crate::json::{self, Value};
use crate::lock::CommitLock;
use crate::paths;
use crate::slug::truncate_chars;
use crate::time::Timestamp;

/// Every kind an event may carry.
pub const KINDS: [&str; 10] = [
    "breath",
    "change",
    "correction",
    "decay",
    "guard",
    "promote",
    "reconcile",
    "revoke",
    "shift.handoff",
    "shift.start",
];

/// Kinds that speak *about* an event rather than descend from its work: a
/// revoke or reconcile of X is not X's downstream, and walking into them
/// would have a second reconcile revert the first one's reverts.
const META_KINDS: [&str; 2] = ["reconcile", "revoke"];

/// How many fresh ids an append tries before giving up on a collision.
const MINT_ATTEMPTS: u32 = 16;
/// How much of a rationale one `log` line keeps.
const RATIONALE_CHARS: usize = 80;
/// How much of a sha one `log` line keeps.
const SHA_CHARS: usize = 7;

/// Disambiguates ids minted within the same process.
static MINTED: AtomicU64 = AtomicU64::new(0);

/// Whether `kind` is one of [`KINDS`].
#[must_use]
pub fn is_kind(kind: &str) -> bool {
    KINDS.contains(&kind)
}

/// An event before it has an id and an instant — what a caller supplies.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    /// The branch the work landed on.
    pub branch: Option<String>,
    /// What the work cost, in US dollars.
    pub cost_usd: Option<f64>,
    /// One of [`KINDS`].
    pub kind: String,
    /// The model that did the work.
    pub model: Option<String>,
    /// The id of the event this one descends from.
    pub parent: Option<String>,
    /// Why.
    pub rationale: Option<String>,
    /// Free references — thread ids, URLs, memory files, `repo@sha`.
    pub refs: Vec<String>,
    /// The repo the work touched, `owner/repo`.
    pub repo: Option<String>,
    /// The commit the work produced.
    pub sha: Option<String>,
    /// The shift the event belongs to.
    pub shift: Option<String>,
}

impl Draft {
    /// A draft from a JSON object — the `--json` form, the whole event minus
    /// `id` and `at`. Unknown keys are refused rather than dropped.
    pub fn from_json(text: &str) -> Result<Self> {
        let value = json::parse(text).map_err(|error| Error::Json {
            path: None,
            message: error.to_string(),
        })?;
        let entries = value
            .as_object()
            .ok_or_else(|| Error::invalid("--json", "not an object"))?;
        let mut draft = Self::default();
        for (key, value) in entries {
            draft.set(key, value)?;
        }
        Ok(draft)
    }

    /// Fill one key off a JSON value.
    fn set(&mut self, key: &str, value: &Value) -> Result<()> {
        let text = || -> Result<Option<String>> {
            match value {
                Value::Null => Ok(None),
                Value::String(text) => Ok(Some(text.clone())),
                _ => Err(Error::invalid("event key", key.to_owned())),
            }
        };
        match key {
            "branch" => self.branch = text()?,
            "cost_usd" => {
                self.cost_usd = match value {
                    Value::Null => None,
                    Value::Number(cost) => Some(*cost),
                    _ => return Err(Error::invalid("event key", "cost_usd")),
                };
            }
            "kind" => self.kind = text()?.unwrap_or_default(),
            "model" => self.model = text()?,
            "parent" => self.parent = text()?,
            "rationale" => self.rationale = text()?,
            "refs" => {
                self.refs = match value {
                    Value::Null => Vec::new(),
                    Value::Array(items) => items
                        .iter()
                        .map(|item| {
                            item.as_str()
                                .map(ToOwned::to_owned)
                                .ok_or_else(|| Error::invalid("event key", "refs"))
                        })
                        .collect::<Result<_>>()?,
                    _ => return Err(Error::invalid("event key", "refs")),
                };
            }
            "repo" => self.repo = text()?,
            "sha" => self.sha = text()?,
            "shift" => self.shift = text()?,
            other => return Err(Error::invalid("event key", other.to_owned())),
        }
        Ok(())
    }
}

/// One event on the trail.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// When it was appended.
    pub at: Timestamp,
    /// Everything the caller supplied.
    pub draft: Draft,
    /// `e-<UTC compact stamp>Z-<4 hex>`.
    pub id: String,
}

impl Event {
    /// A fresh event: `draft` stamped with `at` and a newly minted id.
    #[must_use]
    pub fn new(draft: Draft, at: Timestamp) -> Self {
        Self {
            at,
            draft,
            id: mint(at),
        }
    }

    /// The one line it occupies on disk — keys alphabetical, absent keys
    /// omitted, `parent` always present and `refs` always an array.
    #[must_use]
    pub fn render(&self) -> String {
        let draft = &self.draft;
        let mut entries: BTreeMap<&str, Value> = BTreeMap::new();
        entries.insert("at", Value::string(self.at.iso8601()));
        entries.insert("id", Value::string(self.id.clone()));
        entries.insert("kind", Value::string(draft.kind.clone()));
        entries.insert(
            "parent",
            draft.parent.clone().map_or(Value::Null, Value::string),
        );
        entries.insert(
            "refs",
            Value::array(draft.refs.iter().cloned().map(Value::string)),
        );
        if let Some(cost) = draft.cost_usd {
            entries.insert("cost_usd", Value::Number(cost));
        }
        for (key, value) in [
            ("branch", &draft.branch),
            ("model", &draft.model),
            ("rationale", &draft.rationale),
            ("repo", &draft.repo),
            ("sha", &draft.sha),
            ("shift", &draft.shift),
        ] {
            if let Some(value) = value {
                entries.insert(key, Value::string(value.clone()));
            }
        }
        Value::object(entries).render()
    }

    /// Read one line back.
    pub fn parse(line: &str) -> Result<Self> {
        let value = json::parse(line).map_err(|error| Error::Json {
            path: None,
            message: error.to_string(),
        })?;
        let entries = value
            .as_object()
            .ok_or_else(|| Error::invalid("event", line.to_owned()))?;
        let mut draft = Draft::default();
        let mut at = None;
        let mut id = None;
        for (key, value) in entries {
            match key.as_str() {
                "at" => at = value.as_str().and_then(Timestamp::parse_iso8601),
                "id" => id = value.as_str().map(ToOwned::to_owned),
                other => draft.set(other, value)?,
            }
        }
        match (at, id) {
            (Some(at), Some(id)) if !draft.kind.is_empty() => Ok(Self { at, draft, id }),
            _ => Err(Error::invalid("event", line.to_owned())),
        }
    }

    /// The `log` line: `at  id  kind  repo  sha(7)  rationale(≤80)`.
    #[must_use]
    pub fn summary(&self) -> String {
        let draft = &self.draft;
        let rationale = draft
            .rationale
            .as_deref()
            .and_then(|text| text.lines().next())
            .map_or("-", |line| truncate_chars(line.trim(), RATIONALE_CHARS));
        format!(
            "{}  {}  {}  {}  {}  {rationale}",
            self.at.iso8601(),
            self.id,
            draft.kind,
            draft.repo.as_deref().unwrap_or("-"),
            draft
                .sha
                .as_deref()
                .map_or("-", |sha| truncate_chars(sha, SHA_CHARS)),
        )
    }

    /// The event one `key: value` per line, for `show`.
    #[must_use]
    pub fn pretty(&self) -> String {
        let draft = &self.draft;
        let mut lines = vec![
            format!("at: {}", self.at.iso8601()),
            format!("id: {}", self.id),
        ];
        let mut push = |key: &str, value: Option<String>| {
            if let Some(value) = value {
                lines.push(format!("{key}: {value}"));
            }
        };
        push("branch", draft.branch.clone());
        push("cost_usd", draft.cost_usd.map(|cost| cost.to_string()));
        push("kind", Some(draft.kind.clone()));
        push("model", draft.model.clone());
        push(
            "parent",
            Some(draft.parent.clone().unwrap_or_else(|| "null".to_owned())),
        );
        push("rationale", draft.rationale.clone());
        push("refs", Some(draft.refs.join(", ")));
        push("repo", draft.repo.clone());
        push("sha", draft.sha.clone());
        push("shift", draft.shift.clone());
        lines.sort();
        lines.join("\n")
    }

    /// The UTC day it is filed under, `yyyy-mm-dd`.
    #[must_use]
    pub fn day(&self) -> String {
        let (year, month, day, ..) = self.at.parts();
        format!("{year:04}-{month:02}-{day:02}")
    }
}

/// `e-<yyyymmddThhmmss>Z-<4 hex>` — the hex from a per-process random hash
/// over a counter, so two ids minted in one second still differ.
#[must_use]
pub fn mint(at: Timestamp) -> String {
    let serial = MINTED.fetch_add(1, Ordering::Relaxed);
    let hex = RandomState::new().hash_one((serial, std::process::id())) & 0xffff;
    format!("e-{}Z-{hex:04x}", at.stamp())
}

/// One line read off the trail: the event and the bytes it was written as.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The parsed event.
    pub event: Event,
    /// The line exactly as stored.
    pub raw: String,
}

/// Every event on the trail, oldest file first and file order within — so
/// newest last. Files before `since` (`yyyy-mm-dd`) are skipped unread.
pub fn read(data_root: &Path, since: Option<&str>) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for path in day_files(data_root)? {
        if let Some(since) = since
            && day_of(data_root, &path).is_some_and(|day| day.as_str() < since)
        {
            continue;
        }
        entries.extend(read_file(&path)?);
    }
    Ok(entries)
}

/// One day's file; absent reads as empty.
fn read_file(path: &Path) -> Result<Vec<Entry>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(source) if source.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(Error::io(path, source)),
    };
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            Event::parse(line)
                .map(|event| Entry {
                    event,
                    raw: line.to_owned(),
                })
                .map_err(|error| Error::Json {
                    path: Some(path.to_path_buf()),
                    message: format!("line {}: {error}", index + 1),
                })
        })
        .collect()
}

/// Every `<yyyy>/<mm>/<dd>.jsonl` under the trail, in calendar order.
fn day_files(data_root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for year in sorted_children(&paths::trail_dir(data_root))? {
        for month in sorted_children(&year)? {
            for day in sorted_children(&month)? {
                if day.extension().is_some_and(|ext| ext == "jsonl") {
                    files.push(day);
                }
            }
        }
    }
    Ok(files)
}

/// A directory's entries, sorted; a missing directory has none.
fn sorted_children(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(source) if source.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) if source.kind() == ErrorKind::NotADirectory => return Ok(Vec::new()),
        Err(source) => return Err(Error::io(dir, source)),
    };
    let mut children: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    children.sort();
    Ok(children)
}

/// The `yyyy-mm-dd` a day file covers, read off its path.
fn day_of(data_root: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(paths::trail_dir(data_root)).ok()?;
    let parts: Vec<&str> = rest.iter().filter_map(|part| part.to_str()).collect();
    match parts.as_slice() {
        [year, month, day] => Some(format!(
            "{year}-{month}-{}",
            day.strip_suffix(".jsonl").unwrap_or(day)
        )),
        _ => None,
    }
}

/// The event with `id`, if the trail holds one.
#[must_use]
pub fn find<'a>(entries: &'a [Entry], id: &str) -> Option<&'a Event> {
    entries
        .iter()
        .map(|entry| &entry.event)
        .find(|event| event.id == id)
}

/// Every event whose `parent` chain reaches `id`, in trail order — `revoke`
/// and `reconcile` events excluded, along with anything below them.
#[must_use]
pub fn descendants<'a>(entries: &'a [Entry], id: &str) -> Vec<&'a Event> {
    let mut children: BTreeMap<&str, Vec<&Event>> = BTreeMap::new();
    for entry in entries {
        let event = &entry.event;
        if META_KINDS.contains(&event.draft.kind.as_str()) {
            continue;
        }
        if let Some(parent) = &event.draft.parent {
            children.entry(parent).or_default().push(event);
        }
    }
    let mut reached: BTreeSet<&str> = BTreeSet::new();
    let mut queue = VecDeque::from([id]);
    while let Some(next) = queue.pop_front() {
        for child in children.get(next).into_iter().flatten() {
            if child.id != id && reached.insert(&child.id) {
                queue.push_back(&child.id);
            }
        }
    }
    entries
        .iter()
        .map(|entry| &entry.event)
        .filter(|event| reached.contains(event.id.as_str()))
        .collect()
}

/// Whether `id` names a shift (`s-…`) rather than an event (`e-…`).
#[must_use]
pub fn is_shift_id(id: &str) -> bool {
    id.starts_with("s-")
}

/// The ref a shift-wide `revoke` carries in place of a parent.
#[must_use]
pub fn shift_ref(shift: &str) -> String {
    format!("shift:{shift}")
}

/// What revoking `id` takes back, in trail order: an event and its
/// descendants, or — for a shift id — every event of that shift and their
/// descendants. `revoke` and `reconcile` events are never in it.
pub fn revoked_set(entries: &[Entry], id: &str) -> Result<Vec<String>> {
    let roots: Vec<&str> = if is_shift_id(id) {
        entries
            .iter()
            .map(|entry| &entry.event)
            .filter(|event| {
                event.draft.shift.as_deref() == Some(id)
                    && !META_KINDS.contains(&event.draft.kind.as_str())
            })
            .map(|event| event.id.as_str())
            .collect()
    } else {
        find(entries, id)
            .map(|event| event.id.as_str())
            .into_iter()
            .collect()
    };
    if roots.is_empty() {
        return Err(Error::not_found(
            if is_shift_id(id) { "shift" } else { "event" },
            id.to_owned(),
        ));
    }
    let mut set: BTreeSet<&str> = roots.iter().copied().collect();
    for root in &roots {
        set.extend(
            descendants(entries, root)
                .iter()
                .map(|event| event.id.as_str()),
        );
    }
    Ok(entries
        .iter()
        .map(|entry| entry.event.id.clone())
        .filter(|event| set.contains(event.as_str()))
        .collect())
}

/// The `revoke` events naming `id`, in trail order — by `parent` for an
/// event, by a `shift:<id>` ref for a shift.
#[must_use]
pub fn revocations<'a>(entries: &'a [Entry], id: &str) -> Vec<&'a Event> {
    let by_ref = shift_ref(id);
    entries
        .iter()
        .map(|entry| &entry.event)
        .filter(|event| event.draft.kind == "revoke")
        .filter(|event| {
            if is_shift_id(id) {
                event.draft.refs.contains(&by_ref)
            } else {
                event.draft.parent.as_deref() == Some(id)
            }
        })
        .collect()
}

/// Append a new event made from `draft`, stamped now.
pub fn append(data_root: &Path, draft: Draft) -> Result<Event> {
    append_event(data_root, Event::new(draft, Timestamp::now()?), true)
}

/// Append `event` to the file for its day.
///
/// Refuses an unknown kind, a non-finite or negative cost, and a parent the
/// trail does not hold. An id already in the day's file is re-minted when
/// `remint` allows it, else refused — a caller that stamped the id into
/// commits already cannot have it change underneath.
pub fn append_event(data_root: &Path, event: Event, remint: bool) -> Result<Event> {
    let draft = &event.draft;
    if !is_kind(&draft.kind) {
        return Err(Error::invalid("kind", draft.kind.clone()));
    }
    if draft
        .cost_usd
        .is_some_and(|cost| !cost.is_finite() || cost < 0.0)
    {
        return Err(Error::invalid(
            "cost_usd",
            draft.cost_usd.unwrap_or_default().to_string(),
        ));
    }
    let _lock = CommitLock::acquire(data_root)?;
    append_locked(data_root, event, remint)
}

/// [`append_event`]'s body, for a caller already holding `.commit.lock`.
fn append_locked(data_root: &Path, mut event: Event, remint: bool) -> Result<Event> {
    if let Some(parent) = &event.draft.parent
        && find(&read(data_root, None)?, parent).is_none()
    {
        return Err(Error::not_found("parent event", parent.clone()));
    }
    let path = paths::trail_day(data_root, event.at);
    let existing = read_file(&path)?;
    let mut attempts = 0;
    while existing.iter().any(|entry| entry.event.id == event.id) {
        attempts += 1;
        if !remint || attempts > MINT_ATTEMPTS {
            return Err(Error::refused(format!(
                "event id {} is already on the trail",
                event.id
            )));
        }
        event.id = mint(event.at);
    }
    let mut text = String::new();
    for entry in &existing {
        text.push_str(&entry.raw);
        text.push('\n');
    }
    text.push_str(&event.render());
    text.push('\n');
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|source| Error::io(dir, source))?;
    }
    atomic::write(&path, &text)?;
    Ok(event)
}

/// What a revoke did.
#[derive(Clone, Debug, PartialEq)]
pub enum Revoked {
    /// The target was already revoked by this earlier event; nothing written.
    Already(Event),
    /// A new `revoke` event was appended.
    Now(Event),
}

/// Append a `revoke` naming `id` — an event as `parent`, a shift as a
/// `shift:<id>` ref — unless one already does. The check and the append
/// share one hold of `.commit.lock`, so two revokes racing write one event.
pub fn revoke(data_root: &Path, id: &str, reason: &str) -> Result<Revoked> {
    let _lock = CommitLock::acquire(data_root)?;
    let entries = read(data_root, None)?;
    revoked_set(&entries, id)?;
    if let Some(earlier) = revocations(&entries, id).first() {
        return Ok(Revoked::Already((*earlier).clone()));
    }
    let (parent, refs) = if is_shift_id(id) {
        (None, vec![shift_ref(id)])
    } else {
        (Some(id.to_owned()), Vec::new())
    };
    let draft = Draft {
        kind: "revoke".to_owned(),
        parent,
        rationale: Some(reason.to_owned()),
        refs,
        ..Draft::default()
    };
    append_locked(data_root, Event::new(draft, Timestamp::now()?), true).map(Revoked::Now)
}

/// Today's file — its parents created, the file itself not.
pub fn today(data_root: &Path) -> Result<PathBuf> {
    let path = paths::trail_day(data_root, Timestamp::now()?);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|source| Error::io(dir, source))?;
    }
    Ok(path)
}

/// The two trailer lines every machine commit carries.
#[must_use]
pub fn trailers(shift: &str, event: &str) -> String {
    format!("Shift: {shift}\nEvent: {event}")
}

#[cfg(test)]
mod tests {
    use super::{
        Draft, Event, KINDS, Revoked, append, append_event, descendants, mint, read, revoke,
        revoked_set, trailers,
    };
    use crate::error::Error;
    use crate::testutil::TempDir;
    use crate::time::Timestamp;

    fn draft(kind: &str, parent: Option<&str>) -> Draft {
        Draft {
            kind: kind.to_owned(),
            parent: parent.map(ToOwned::to_owned),
            ..Draft::default()
        }
    }

    #[test]
    fn the_kinds_are_alphabetical() {
        let mut sorted = KINDS;
        sorted.sort_unstable();
        assert_eq!(sorted, KINDS);
    }

    #[test]
    fn a_line_renders_keys_sorted_and_parses_back() {
        let at = Timestamp::from_unix_seconds(1_791_589_262);
        let event = Event {
            at,
            draft: Draft {
                branch: Some("perpetual".to_owned()),
                cost_usd: Some(0.25),
                kind: "change".to_owned(),
                refs: vec!["t65".to_owned()],
                shift: Some("s-1".to_owned()),
                ..Draft::default()
            },
            id: "e-20261009T234102Z-7f3a".to_owned(),
        };
        let line = event.render();
        assert_eq!(
            line,
            r#"{"at":"2026-10-09T23:41:02Z","branch":"perpetual","cost_usd":0.25,"id":"e-20261009T234102Z-7f3a","kind":"change","parent":null,"refs":["t65"],"shift":"s-1"}"#
        );
        assert_eq!(Event::parse(&line).expect("parse"), event);
    }

    #[test]
    fn ids_carry_the_stamp_and_differ_within_a_second() {
        let at = Timestamp::from_unix_seconds(1_791_589_262);
        let first = mint(at);
        assert!(first.starts_with("e-20261009T234102Z-"), "{first}");
        assert_eq!(first.len(), "e-20261009T234102Z-7f3a".len());
        let many: std::collections::BTreeSet<String> = (0..64).map(|_| mint(at)).collect();
        assert!(many.len() > 32, "ids collide too often: {}", many.len());
    }

    #[test]
    fn append_refuses_unknown_kinds_and_missing_parents() {
        let temp = TempDir::new("trail-refuse");
        assert!(matches!(
            append(temp.path(), draft("nonsense", None)),
            Err(Error::InvalidInput { what: "kind", .. })
        ));
        assert!(matches!(
            append(temp.path(), draft("change", Some("e-nope"))),
            Err(Error::NotFound { .. })
        ));
        assert!(read(temp.path(), None).expect("read").is_empty());
    }

    #[test]
    fn a_fixed_id_already_on_the_trail_is_refused() {
        let temp = TempDir::new("trail-fixed");
        let event = Event::new(draft("change", None), Timestamp::now().expect("now"));
        append_event(temp.path(), event.clone(), false).expect("first");
        assert!(matches!(
            append_event(temp.path(), event, false),
            Err(Error::Refused { .. })
        ));
    }

    #[test]
    fn descendants_follow_parents_but_not_revokes() {
        let temp = TempDir::new("trail-descendants");
        let root = append(temp.path(), draft("promote", None)).expect("root");
        let child = append(temp.path(), draft("change", Some(&root.id))).expect("child");
        let grandchild = append(temp.path(), draft("change", Some(&child.id))).expect("grandchild");
        append(temp.path(), draft("change", None)).expect("unrelated");
        let first = revoke(temp.path(), &root.id, "wrong path").expect("revoke");
        let Revoked::Now(revocation) = first else {
            panic!("expected a new revoke");
        };
        assert_eq!(revocation.draft.parent.as_deref(), Some(root.id.as_str()));
        assert!(matches!(
            revoke(temp.path(), &root.id, "again").expect("again"),
            Revoked::Already(_)
        ));

        let entries = read(temp.path(), None).expect("read");
        assert_eq!(entries.len(), 5);
        let found: Vec<&str> = descendants(&entries, &root.id)
            .into_iter()
            .map(|event| event.id.as_str())
            .collect();
        assert_eq!(found, [child.id.as_str(), grandchild.id.as_str()]);
    }

    #[test]
    fn racing_revokes_write_one_event() {
        let temp = TempDir::new("trail-race");
        let target = append(temp.path(), draft("change", None)).expect("target");
        let root = temp.path().to_path_buf();
        let racers: Vec<_> = (0..8)
            .map(|_| {
                let (root, id) = (root.clone(), target.id.clone());
                std::thread::spawn(move || revoke(&root, &id, "race").expect("revoke"))
            })
            .collect();
        let fresh = racers
            .into_iter()
            .map(|racer| racer.join().expect("join"))
            .filter(|outcome| matches!(outcome, Revoked::Now(_)))
            .count();
        assert_eq!(fresh, 1);
        let entries = read(temp.path(), None).expect("read");
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.event.draft.kind == "revoke")
                .count(),
            1
        );
    }

    #[test]
    fn a_shift_id_revokes_its_events_and_their_descendants() {
        let temp = TempDir::new("trail-shift-set");
        let shifted = |parent: Option<&str>| Draft {
            shift: Some("s-1".to_owned()),
            ..draft("change", parent)
        };
        let first = append(temp.path(), shifted(None)).expect("first");
        let other = append(temp.path(), draft("change", None)).expect("other");
        let child = append(temp.path(), draft("change", Some(&first.id))).expect("child");
        let second = append(temp.path(), shifted(Some(&other.id))).expect("second");
        let entries = read(temp.path(), None).expect("read");
        assert_eq!(
            revoked_set(&entries, "s-1").expect("set"),
            [first.id.clone(), child.id, second.id]
        );
        assert!(matches!(
            revoked_set(&entries, "s-2"),
            Err(Error::NotFound { .. })
        ));
        assert_eq!(revoked_set(&entries, &other.id).expect("set").len(), 2);
    }

    #[test]
    fn json_drafts_refuse_unknown_keys() {
        let draft = Draft::from_json(r#"{"kind":"breath","refs":["a","b"],"cost_usd":1.5}"#)
            .expect("draft");
        assert_eq!(draft.kind, "breath");
        assert_eq!(draft.refs, ["a", "b"]);
        assert_eq!(draft.cost_usd, Some(1.5));
        assert!(Draft::from_json(r#"{"kind":"breath","id":"e-x"}"#).is_err());
        assert!(Draft::from_json("[]").is_err());
    }

    #[test]
    fn the_trailers_are_two_lines() {
        assert_eq!(trailers("s-1", "e-2"), "Shift: s-1\nEvent: e-2");
    }
}
