//! `breathe` — the ruach's store and the pass composer.
//!
//! The ruach is the research engine that runs beside the shifts: a Claude
//! session reads the feeds in `breath/sources.toml` against the problems the
//! shift engine keeps in `breath/problems.md`, and files what bears on a repo
//! as a **breath** — `breath/<date>-<slug>.md`, an idea with provenance and a
//! state (`new`, `spiked`, `adopted`, `dropped`). This module is the store and
//! the prompt composer, and `tick` its runner, which calls no model or
//! network itself but starts sessions through `dispatch`. `pass`
//! fills `prompts/breathe.md` for the session the runner starts; `file`,
//! `spike`, `adopt` and `drop` are what that session and the shifts call;
//! `close` and `reweight` feed what happened back into the sources.
//!
//! Every write holds `.commit.lock` and goes tmp → rename; the `breath` trail
//! events are appended after the lock is released, through the trail's own
//! append. The contract is `docs/bank-format.html#breath`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use crate::atomic;
use crate::error::{Error, Result};
use crate::json::{self, Value};
use crate::lock::CommitLock;
use crate::paths;
use crate::slug::{slug, truncate_chars};
use crate::time::Timestamp;
use crate::toml;
use crate::verbs::recall;
use crate::verbs::shift;
use crate::verbs::trail;

/// The seed `breathe init` copies in when the store has no `sources.toml`.
pub const SEED: &str = include_str!("../../breath/sources.toml");
/// `$SANDMAN_BREATH_BUDGET_USD`'s default.
pub const DEFAULT_BUDGET_USD: f64 = 10.0;
/// `$SANDMAN_BREATH_DEAD_COST_USD`'s default: what a dead pass is charged.
pub const DEFAULT_DEAD_COST_USD: f64 = 2.0;
/// Title-token Jaccard at or above this is the same breath.
pub const JACCARD_SAME: f64 = 0.6;
/// The problems slot's ceiling.
const PROBLEMS_CHARS: usize = 6_000;
/// A skim reads a source again once its `last_read` is this old.
const SKIM_SECONDS: i64 = 3_600;
/// `silent_since` older than this lowers a source.
const SILENCE_SECONDS: i64 = 30 * 86_400;
/// What one breath name's slug is cut to.
const SLUG_CHARS: usize = 60;
/// How many `-<n>` suffixes a breath name tries.
const MAX_COLLISIONS: u32 = 1_000;
/// Adoption raises a weight no higher than this.
const WEIGHT_CAP: f64 = 2.0;
/// Silence lowers a weight no lower than this.
const WEIGHT_FLOOR: f64 = 0.2;
/// What one adoption, or one month of silence, moves a weight by.
const WEIGHT_STEP: f64 = 0.1;

/// Every state a breath may be in.
pub const STATES: [&str; 4] = ["adopted", "dropped", "new", "spiked"];

/// Everything the breathe verbs read, resolved by the caller.
#[derive(Clone, Debug)]
pub struct Context {
    /// The data root.
    pub data_root: PathBuf,
    /// The instant the verb acts at.
    pub now: Timestamp,
    /// The directory holding `breathe.md`.
    pub prompts: PathBuf,
    /// `$SANDMAN_SHIFT` — the shift a transition is made in, if any.
    pub shift: Option<String>,
}

// ─── the sources ─────────────────────────────────────────────────────────

/// One feed.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    /// Breaths from it adopted, ever.
    pub adopted: u64,
    /// Repo slugs it bears on, or `*`.
    pub bears_on: Vec<String>,
    /// Items a pass reads from it at most.
    pub budget: u64,
    /// Where the last pass stopped, in the feed's own terms.
    pub cursor: Option<String>,
    /// Breaths filed from it, ever.
    pub filed: u64,
    /// `atom` · `rss` · `json` · `html` — how a reader parses it.
    pub kind: String,
    /// When a pass last read it.
    pub last_read: Option<Timestamp>,
    /// The table name.
    pub name: String,
    /// Since when it has gone without an adoption.
    pub silent_since: Option<Timestamp>,
    /// The feed.
    pub url: String,
    /// Its learned weight.
    pub weight: f64,
}

/// The keys a source table may carry, in the order they are written.
const SOURCE_KEYS: [&str; 10] = [
    "kind",
    "url",
    "weight",
    "budget",
    "bears_on",
    "cursor",
    "last_read",
    "filed",
    "adopted",
    "silent_since",
];

impl Source {
    /// Whether a pass of `kind` reads it at `now`: a deep pass reads every
    /// source, a skim those unread for an hour.
    #[must_use]
    pub fn due(&self, kind: PassKind, now: Timestamp) -> bool {
        kind == PassKind::Deep
            || self
                .last_read
                .is_none_or(|at| now.unix_seconds() - at.unix_seconds() >= SKIM_SECONDS)
    }

    /// The `sources --json` object.
    fn json(&self) -> Value {
        let text = |value: &Option<String>| value.clone().map_or(Value::Null, Value::string);
        let at =
            |value: Option<Timestamp>| value.map_or(Value::Null, |at| Value::string(at.iso8601()));
        Value::object([
            ("adopted", count(self.adopted)),
            (
                "bears_on",
                Value::array(self.bears_on.iter().cloned().map(Value::string)),
            ),
            ("budget", count(self.budget)),
            ("cursor", text(&self.cursor)),
            ("filed", count(self.filed)),
            ("kind", Value::string(self.kind.clone())),
            ("last_read", at(self.last_read)),
            ("name", Value::string(self.name.clone())),
            ("silent_since", at(self.silent_since)),
            ("url", Value::string(self.url.clone())),
            ("weight", Value::Number(self.weight)),
        ])
    }

    /// The `sources` line.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "{}  weight={}  budget={}  cursor={}  last_read={}",
            self.name,
            weight_text(self.weight),
            self.budget,
            self.cursor.as_deref().unwrap_or("-"),
            self.last_read
                .map_or_else(|| "never".to_owned(), Timestamp::iso8601)
        )
    }
}

/// A count as a JSON number.
fn count(value: u64) -> Value {
    Value::count(usize::try_from(value).unwrap_or(usize::MAX))
}

/// A weight with at least one decimal.
fn weight_text(weight: f64) -> String {
    let text = format!("{weight}");
    if text.contains('.') {
        text
    } else {
        format!("{text}.0")
    }
}

/// `sources.toml`, read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sources {
    /// When `reweight` last ran — the root key `reweighted`.
    pub reweighted: Option<Timestamp>,
    /// The feeds, sorted by name.
    pub sources: Vec<Source>,
}

impl Sources {
    /// Read `<root>/breath/sources.toml`.
    pub fn read(data_root: &Path) -> Result<Self> {
        let path = paths::breath_sources(data_root);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(source) if source.kind() == ErrorKind::NotFound => {
                return Err(Error::not_found(
                    "sources.toml",
                    format!("{} — run `sandman breathe init`", path.display()),
                ));
            }
            Err(source) => return Err(Error::io(&path, source)),
        };
        Self::parse(&text).map_err(|message| {
            Error::invalid("sources.toml", format!("{}: {message}", path.display()))
        })
    }

    /// The flat TOML, checked against the schema.
    pub fn parse(text: &str) -> std::result::Result<Self, String> {
        let document = toml::parse(text).map_err(|error| error.to_string())?;
        let mut reweighted = None;
        for (key, value) in &document.root {
            match (key.as_str(), value) {
                ("reweighted", toml::Value::String(at)) => {
                    reweighted = Some(instant(at, "reweighted")?);
                }
                _ => return Err(format!("unknown root key `{key}`")),
            }
        }
        let mut sources = document
            .tables
            .iter()
            .map(source)
            .collect::<std::result::Result<Vec<_>, String>>()?;
        sources.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(Self {
            reweighted,
            sources,
        })
    }

    /// The TOML document: root key, then the tables alpha-sorted, keys in
    /// [`SOURCE_KEYS`] order, state keys only when set.
    #[must_use]
    pub fn document(&self) -> toml::Document {
        let mut sources: Vec<&Source> = self.sources.iter().collect();
        sources.sort_by(|left, right| left.name.cmp(&right.name));
        let tables = sources
            .into_iter()
            .map(|source| {
                let mut entries = vec![
                    ("kind", toml::Value::String(source.kind.clone())),
                    ("url", toml::Value::String(source.url.clone())),
                    ("weight", toml::Value::Float(source.weight)),
                    ("budget", toml::Value::Integer(integer(source.budget))),
                    ("bears_on", toml::Value::Strings(source.bears_on.clone())),
                ];
                if let Some(cursor) = &source.cursor {
                    entries.push(("cursor", toml::Value::String(cursor.clone())));
                }
                if let Some(at) = source.last_read {
                    entries.push(("last_read", toml::Value::String(at.iso8601())));
                }
                if source.filed > 0 {
                    entries.push(("filed", toml::Value::Integer(integer(source.filed))));
                }
                if source.adopted > 0 {
                    entries.push(("adopted", toml::Value::Integer(integer(source.adopted))));
                }
                if let Some(at) = source.silent_since {
                    entries.push(("silent_since", toml::Value::String(at.iso8601())));
                }
                toml::Table {
                    entries: entries
                        .into_iter()
                        .map(|(key, value)| (key.to_owned(), value))
                        .collect(),
                    name: source.name.clone(),
                }
            })
            .collect();
        toml::Document {
            root: self
                .reweighted
                .map(|at| ("reweighted".to_owned(), toml::Value::String(at.iso8601())))
                .into_iter()
                .collect(),
            tables,
        }
    }

    /// The file as `write` puts it on disk.
    #[must_use]
    pub fn render(&self) -> String {
        format!(
            "# The ruach's feeds — rewritten by `sandman breathe`; the schema is docs/bank-format.html#breath.\n\n{}",
            self.document().render()
        )
    }

    /// Replace `sources.toml`. The caller holds the lock.
    fn write(&self, data_root: &Path) -> Result<()> {
        let path = paths::breath_sources(data_root);
        atomic::ensure_parent(&path)?;
        atomic::write(&path, &self.render())
    }

    /// The source named `name`.
    fn get_mut(&mut self, name: &str) -> Option<&mut Source> {
        self.sources.iter_mut().find(|source| source.name == name)
    }
}

/// A count as a TOML integer.
fn integer(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// An ISO-8601 Z instant out of a TOML string.
fn instant(text: &str, key: &str) -> std::result::Result<Timestamp, String> {
    Timestamp::parse_iso8601(text).ok_or_else(|| format!("`{key}` wants ISO-8601, got `{text}`"))
}

/// One `[name]` table as a source.
fn source(table: &toml::Table) -> std::result::Result<Source, String> {
    let name = &table.name;
    if let Some((key, _)) = table
        .entries
        .iter()
        .find(|(key, _)| !SOURCE_KEYS.contains(&key.as_str()))
    {
        return Err(format!("[{name}]: unknown key `{key}`"));
    }
    let wrong = |key: &str, want: &str| format!("[{name}]: `{key}` wants {want}");
    let text = |key: &str| -> std::result::Result<Option<String>, String> {
        table.get(key).map_or(Ok(None), |value| {
            value
                .as_str()
                .map(|text| Some(text.to_owned()))
                .ok_or_else(|| wrong(key, "a string"))
        })
    };
    let counter = |key: &str| -> std::result::Result<u64, String> {
        table.get(key).map_or(Ok(0), |value| {
            value
                .as_i64()
                .and_then(|number| u64::try_from(number).ok())
                .ok_or_else(|| wrong(key, "a non-negative integer"))
        })
    };
    let at = |key: &str| -> std::result::Result<Option<Timestamp>, String> {
        text(key)?
            .map(|value| instant(&value, key).map_err(|message| format!("[{name}]: {message}")))
            .transpose()
    };
    let required = |key: &str| format!("[{name}]: `{key}` is required");
    let weight = table
        .get("weight")
        .ok_or_else(|| required("weight"))?
        .as_f64()
        .filter(|weight| weight.is_finite() && *weight >= 0.0)
        .ok_or_else(|| wrong("weight", "a non-negative number"))?;
    let bears_on = table
        .get("bears_on")
        .ok_or_else(|| required("bears_on"))?
        .as_strings()
        .ok_or_else(|| wrong("bears_on", "an array of strings"))?
        .to_vec();
    if let Some(bad) = bears_on.iter().find(|repo| !valid_repo(repo)) {
        return Err(format!(
            "[{name}]: `bears_on` names `{bad}`, not `*` or an owner/name slug"
        ));
    }
    if table.get("budget").is_none() {
        return Err(required("budget"));
    }
    Ok(Source {
        adopted: counter("adopted")?,
        bears_on,
        budget: counter("budget")?,
        cursor: text("cursor")?,
        filed: counter("filed")?,
        kind: text("kind")?.ok_or_else(|| required("kind"))?,
        last_read: at("last_read")?,
        name: name.clone(),
        silent_since: at("silent_since")?,
        url: text("url")?.ok_or_else(|| required("url"))?,
        weight,
    })
}

/// `*`, or an `owner/name` slug of ASCII letters, digits, `.`, `_`, `-`.
#[must_use]
pub fn valid_repo(repo: &str) -> bool {
    if repo == "*" {
        return true;
    }
    let part = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    };
    matches!(repo.split_once('/'), Some((owner, name)) if part(owner) && part(name))
}

/// The sources a pass of `kind` reads now, sorted by name.
pub fn sources(ctx: &Context, due: Option<PassKind>) -> Result<Vec<Source>> {
    Ok(Sources::read(&ctx.data_root)?
        .sources
        .into_iter()
        .filter(|source| due.is_none_or(|kind| source.due(kind, ctx.now)))
        .collect())
}

/// The `sources --json` array.
#[must_use]
pub fn sources_json(sources: &[Source]) -> String {
    Value::array(sources.iter().map(Source::json)).render()
}

// ─── init ────────────────────────────────────────────────────────────────

/// Create the store's directories and seed `sources.toml` if absent; the
/// paths it created.
pub fn init(data_root: &Path) -> Result<Vec<PathBuf>> {
    let _lock = CommitLock::acquire(data_root)?;
    let mut made = Vec::new();
    for dir in [
        paths::breath_dir(data_root),
        paths::breath_ledger(data_root),
        paths::breath_passes(data_root),
    ] {
        if !dir.is_dir() {
            fs::create_dir_all(&dir).map_err(|source| Error::io(&dir, source))?;
            made.push(dir);
        }
    }
    let sources = paths::breath_sources(data_root);
    if !sources.exists() {
        atomic::write(&sources, SEED)?;
        made.push(sources);
    }
    Ok(made)
}

// ─── breaths ─────────────────────────────────────────────────────────────

/// One breath file.
#[derive(Clone, Debug, PartialEq)]
pub struct Breath {
    /// When `adopt` ran, ISO-8601 Z — what `reweight` counts by.
    pub adopted: Option<String>,
    /// 0–1, the judge's "does it bear on the repo".
    pub bearing: Option<f64>,
    /// `<date>-<slug>.md`.
    pub file: String,
    /// When it was filed, ISO-8601 Z.
    pub filed: String,
    /// 0–1, the judge's "is it new to us".
    pub novelty: Option<f64>,
    /// The open problem it speaks to.
    pub problem: String,
    /// Why it was dropped — `dropped` only.
    pub reason: Option<String>,
    /// The repo it bears on, `owner/name` or `*`.
    pub repo: String,
    /// How many later filings matched it.
    pub seen_again: u64,
    /// The `sources.toml` table it came from.
    pub source: Option<String>,
    /// One of [`STATES`].
    pub state: String,
    /// The four-line summary — the body.
    pub summary: String,
    /// Its title.
    pub title: String,
    /// When it last changed, ISO-8601 Z.
    pub updated: String,
    /// The primary source.
    pub url: String,
}

impl Breath {
    /// The file's text: frontmatter in the contract's order, then the summary.
    /// A text that starts or ends with a quote is wrapped in one more pair, the
    /// pair the reader strips.
    #[must_use]
    pub fn render(&self) -> String {
        let quote = |text: &str| {
            if text.starts_with(['"', '\'']) || text.ends_with(['"', '\'']) {
                format!("\"{text}\"")
            } else {
                text.to_owned()
            }
        };
        let mut lines = vec![
            "---".to_owned(),
            format!("title: {}", quote(&self.title)),
            format!("url: {}", quote(&self.url)),
        ];
        if let Some(source) = &self.source {
            lines.push(format!("source: {source}"));
        }
        lines.push(format!("repo: {}", self.repo));
        lines.push(format!("problem: {}", quote(&self.problem)));
        lines.push(format!("state: {}", self.state));
        if let Some(bearing) = self.bearing {
            lines.push(format!("bearing: {bearing}"));
        }
        if let Some(novelty) = self.novelty {
            lines.push(format!("novelty: {novelty}"));
        }
        lines.push(format!("filed: {}", self.filed));
        lines.push(format!("updated: {}", self.updated));
        if let Some(adopted) = &self.adopted {
            lines.push(format!("adopted: {adopted}"));
        }
        lines.push(format!("seen_again: {}", self.seen_again));
        if let Some(reason) = &self.reason {
            lines.push(format!("reason: {}", quote(reason)));
        }
        lines.push("---".to_owned());
        format!("{}\n\n{}\n", lines.join("\n"), self.summary.trim_end())
    }

    /// Read one file back; `None` when it is not a breath.
    #[must_use]
    pub fn parse(file: &str, raw: &str) -> Option<Self> {
        let (front, body) = recall::split_frontmatter(raw);
        let fields: BTreeMap<&str, &str> = recall::frontmatter_fields(front?).into_iter().collect();
        let text = |key: &str| fields.get(key).map(|value| (*value).to_owned());
        let score = |key: &str| fields.get(key).and_then(|value| value.parse::<f64>().ok());
        Some(Self {
            adopted: text("adopted"),
            bearing: score("bearing"),
            file: file.to_owned(),
            filed: text("filed").unwrap_or_default(),
            novelty: score("novelty"),
            problem: text("problem").unwrap_or_default(),
            reason: text("reason"),
            repo: text("repo")?,
            seen_again: fields
                .get("seen_again")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
            source: text("source"),
            state: text("state")?,
            summary: body.trim().to_owned(),
            title: text("title")?,
            updated: text("updated").unwrap_or_default(),
            url: text("url")?,
        })
    }

    /// The `list` line: `state  repo  file  title`.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "{}  {}  {}  {}",
            self.state, self.repo, self.file, self.title
        )
    }

    /// The `list --json` object.
    fn json(&self) -> Value {
        let text = |value: &Option<String>| value.clone().map_or(Value::Null, Value::string);
        let score = |value: Option<f64>| value.map_or(Value::Null, Value::Number);
        Value::object([
            ("adopted", text(&self.adopted)),
            ("bearing", score(self.bearing)),
            ("file", Value::string(self.file.clone())),
            ("filed", Value::string(self.filed.clone())),
            ("novelty", score(self.novelty)),
            ("problem", Value::string(self.problem.clone())),
            ("reason", text(&self.reason)),
            ("repo", Value::string(self.repo.clone())),
            ("seen_again", count(self.seen_again)),
            ("source", text(&self.source)),
            ("state", Value::string(self.state.clone())),
            ("summary", Value::string(self.summary.clone())),
            ("title", Value::string(self.title.clone())),
            ("updated", Value::string(self.updated.clone())),
            ("url", Value::string(self.url.clone())),
        ])
    }
}

/// Every breath in the store, by file name.
pub fn breaths(data_root: &Path) -> Result<Vec<Breath>> {
    let dir = paths::breath_dir(data_root);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(source) if source.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(Error::io(&dir, source)),
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().map(ToOwned::to_owned))
        .filter(|name| is_breath_name(name))
        .collect();
    names.sort();
    let mut found = Vec::new();
    for name in names {
        let path = dir.join(&name);
        let raw = fs::read_to_string(&path).map_err(|source| Error::io(&path, source))?;
        if let Some(breath) = Breath::parse(&name, &raw) {
            found.push(breath);
        }
    }
    Ok(found)
}

/// `yyyy-mm-dd-<slug>.md` — what a breath is named, and `problems.md` is not.
fn is_breath_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let dated = bytes.len() > 14
        && bytes[..11].iter().enumerate().all(|(index, byte)| {
            if matches!(index, 4 | 7 | 10) {
                *byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        });
    dated
        && Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
}

/// The breaths `list` shows.
pub fn list(data_root: &Path, repo: Option<&str>, state: Option<&str>) -> Result<Vec<Breath>> {
    if let Some(state) = state
        && !STATES.contains(&state)
    {
        return Err(Error::invalid("state", state.to_owned()));
    }
    Ok(breaths(data_root)?
        .into_iter()
        .filter(|breath| repo.is_none_or(|repo| breath.repo == repo))
        .filter(|breath| state.is_none_or(|state| breath.state == state))
        .collect())
}

/// The `list --json` array.
#[must_use]
pub fn list_json(breaths: &[Breath]) -> String {
    Value::array(breaths.iter().map(Breath::json)).render()
}

/// Lowercase alphanumeric runs.
#[must_use]
pub fn tokens(title: &str) -> BTreeSet<String> {
    title
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// `|A ∩ B| / |A ∪ B|` over two [`tokens`] sets; 0 when both are empty. Sets,
/// not strings, so a caller comparing many pairs tokenizes each text once.
#[must_use]
pub fn jaccard(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f64 {
    let shared = left.intersection(right).count();
    let union = left.len() + right.len() - shared;
    if union == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss, reason = "title token counts are tiny")]
    let share = shared as f64 / union as f64;
    share
}

/// Whether two titles name the same release: their numbers — `\d+(\.\d+)*`
/// runs — agree in order, so `Release 1.0.0` is never a repeat of `1.1.0`.
fn same_numbers(left: &str, right: &str) -> bool {
    fn numbers(title: &str) -> impl Iterator<Item = &str> {
        title
            .split(|ch: char| !ch.is_ascii_digit() && ch != '.')
            .flat_map(|run| run.split(".."))
            .map(|run| run.trim_matches('.'))
            .filter(|run| !run.is_empty())
    }
    numbers(left).eq(numbers(right))
}

/// A URL as dedupe compares it: `http` read as `https`, the scheme and host
/// lowercased, a trailing `/` and the `utm_*` query parameters dropped, and
/// an arXiv id's version suffix (`v2`) cut. The fragment and `ref` stay: a
/// `#/post/1` route or a `?ref=v1` tag names a different page.
#[must_use]
pub fn normal_url(url: &str) -> String {
    let url = url.trim();
    let (url, fragment) = url.split_at(url.find('#').unwrap_or(url.len()));
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => (String::new(), url),
    };
    let scheme = if scheme == "http" {
        "https".to_owned()
    } else {
        scheme
    };
    let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let host = host.to_ascii_lowercase();
    let mut path = path.trim_end_matches('/').to_owned();
    if host == "arxiv.org" || host.ends_with(".arxiv.org") {
        path = unversioned(&path);
    }
    let query: Vec<&str> = query
        .split('&')
        .filter(|pair| {
            let key = pair.split_once('=').map_or(*pair, |(key, _)| key);
            !key.is_empty() && !key.starts_with("utm_")
        })
        .collect();
    let scheme = if scheme.is_empty() {
        String::new()
    } else {
        format!("{scheme}://")
    };
    let query = if query.is_empty() {
        String::new()
    } else {
        format!("?{}", query.join("&"))
    };
    format!("{scheme}{host}{path}{query}{fragment}")
}

/// An arXiv path with the version cut off its id: `/abs/2510.01234v2` →
/// `/abs/2510.01234`, a `.pdf` suffix kept.
fn unversioned(path: &str) -> String {
    let (stem, pdf) = path
        .strip_suffix(".pdf")
        .map_or((path, ""), |stem| (stem, ".pdf"));
    let digits = stem.trim_end_matches(|ch: char| ch.is_ascii_digit());
    match digits.strip_suffix('v') {
        Some(id) if digits.len() < stem.len() && id.ends_with(|ch: char| ch.is_ascii_digit()) => {
            format!("{id}{pdf}")
        }
        _ => path.to_owned(),
    }
}

/// The breath a new filing repeats, if any: the same [`normal_url`], else —
/// within the same repo only — a title whose token Jaccard reaches
/// [`JACCARD_SAME`] with no numeric token differing. A dropped breath is
/// matched only for its own repo, so a drop never blocks another repo.
fn repeat_of<'a>(existing: &'a [Breath], repo: &str, url: &str, title: &str) -> Option<&'a Breath> {
    let (url, said) = (normal_url(url), tokens(title));
    let live = || {
        existing
            .iter()
            .filter(move |breath| breath.state != "dropped" || breath.repo == repo)
    };
    live()
        .find(|breath| normal_url(&breath.url) == url)
        .or_else(|| {
            live().find(|breath| {
                breath.repo == repo
                    && jaccard(&tokens(&breath.title), &said) >= JACCARD_SAME
                    && same_numbers(&breath.title, title)
            })
        })
}

/// What `file` was handed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filing {
    /// 0–1.
    pub bearing: Option<f64>,
    /// 0–1.
    pub novelty: Option<f64>,
    /// The open problem it speaks to.
    pub problem: String,
    /// `owner/name` or `*`.
    pub repo: String,
    /// The `sources.toml` table it came from.
    pub source: Option<String>,
    /// File it `spiked` rather than `new`.
    pub spike: bool,
    /// The four-line summary.
    pub summary: String,
    /// Its title.
    pub title: String,
    /// The primary source.
    pub url: String,
}

/// What `file` did.
#[derive(Clone, Debug, PartialEq)]
pub enum Filed {
    /// A new breath at this path, and its trail event.
    New(PathBuf, Box<trail::Event>),
    /// An existing breath matched; its `seen_again` went up.
    Seen(PathBuf),
}

/// One line, newlines folded — frontmatter values are single-line.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A judge score in 0–1.
fn unit(what: &'static str, value: Option<f64>) -> Result<Option<f64>> {
    match value {
        Some(score) if !(0.0..=1.0).contains(&score) => {
            Err(Error::invalid(what, score.to_string()))
        }
        other => Ok(other),
    }
}

/// File a breath, or count it seen again — see [`repeat_of`]. `repo` is the
/// command line's to check.
pub fn file(ctx: &Context, filing: &Filing) -> Result<Filed> {
    let title = one_line(&filing.title);
    let url = one_line(&filing.url);
    let problem = one_line(&filing.problem);
    for (what, value) in [("title", &title), ("url", &url), ("problem", &problem)] {
        if value.is_empty() {
            return Err(Error::invalid(what, "empty"));
        }
    }
    if filing.summary.trim().is_empty() {
        return Err(Error::invalid("summary", "empty"));
    }
    let bearing = unit("bearing", filing.bearing)?;
    let novelty = unit("novelty", filing.novelty)?;
    let dir = paths::breath_dir(&ctx.data_root);
    let now = ctx.now.iso8601();
    let (path, name) = {
        let _lock = CommitLock::acquire(&ctx.data_root)?;
        let existing = breaths(&ctx.data_root)?;
        if let Some(matched) = repeat_of(&existing, &filing.repo, &url, &title) {
            let mut breath = matched.clone();
            breath.seen_again += 1;
            breath.updated.clone_from(&now);
            let path = dir.join(&breath.file);
            atomic::write(&path, &breath.render())?;
            return Ok(Filed::Seen(path));
        }
        let mut sources = match &filing.source {
            Some(name) => {
                let mut sources = Sources::read(&ctx.data_root)?;
                let source = sources
                    .get_mut(name)
                    .ok_or_else(|| Error::not_found("source", name.clone()))?;
                source.filed += 1;
                Some(sources)
            }
            None => None,
        };
        let (year, month, day, ..) = ctx.now.parts();
        let cut = slug(&title).replace('_', "-");
        let cut = truncate_chars(&cut, SLUG_CHARS).trim_end_matches('-');
        let stem = format!(
            "{year:04}-{month:02}-{day:02}-{}",
            if cut.is_empty() { "breath" } else { cut }
        );
        let taken: BTreeSet<&str> = existing.iter().map(|breath| breath.file.as_str()).collect();
        let name = (1..=MAX_COLLISIONS)
            .map(|nth| {
                if nth == 1 {
                    format!("{stem}.md")
                } else {
                    format!("{stem}-{nth}.md")
                }
            })
            .find(|name| !taken.contains(name.as_str()) && !dir.join(name).exists())
            .ok_or_else(|| Error::TooManyCollisions {
                filename: format!("{stem}.md"),
            })?;
        let breath = Breath {
            adopted: None,
            bearing,
            file: name.clone(),
            filed: now.clone(),
            novelty,
            problem,
            reason: None,
            repo: filing.repo.clone(),
            seen_again: 0,
            source: filing.source.clone(),
            state: if filing.spike { "spiked" } else { "new" }.to_owned(),
            summary: filing.summary.trim().to_owned(),
            title: title.clone(),
            updated: now.clone(),
            url: url.clone(),
        };
        fs::create_dir_all(&dir).map_err(|source| Error::io(&dir, source))?;
        let path = dir.join(&name);
        atomic::write(&path, &breath.render())?;
        if let Some(sources) = sources.as_mut() {
            sources.write(&ctx.data_root)?;
        }
        (path, name)
    };
    let event = trail::append(
        &ctx.data_root,
        trail::Draft {
            kind: "breath".to_owned(),
            rationale: Some(format!("filed: {title}")),
            refs: vec![format!("url:{url}"), format!("breath:{name}")],
            repo: Some(filing.repo.clone()),
            shift: ctx.shift.clone(),
            ..trail::Draft::default()
        },
    )?;
    Ok(Filed::New(path, Box::new(event)))
}

/// A state change `spike`, `adopt` or `drop` asks for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Transition {
    /// `new` or `spiked` → `adopted`.
    Adopt,
    /// anything not yet dropped → `dropped`, with a reason.
    Drop,
    /// `new` → `spiked`.
    Spike,
}

impl Transition {
    /// The state it lands in.
    #[must_use]
    pub fn state(self) -> &'static str {
        match self {
            Self::Adopt => "adopted",
            Self::Drop => "dropped",
            Self::Spike => "spiked",
        }
    }

    /// The subcommand's name.
    #[must_use]
    pub fn verb(self) -> &'static str {
        match self {
            Self::Adopt => "adopt",
            Self::Drop => "drop",
            Self::Spike => "spike",
        }
    }

    /// The states it may leave.
    fn from(self) -> &'static [&'static str] {
        match self {
            Self::Adopt => &["new", "spiked"],
            Self::Drop => &["adopted", "new", "spiked"],
            Self::Spike => &["new"],
        }
    }
}

/// `file` as a breath name: a path or a bare name, `.md` optional.
fn breath_name(file: &str) -> Result<String> {
    let name = Path::new(file)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Error::invalid("breath", file.to_owned()))?;
    let name = if Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
    {
        name.to_owned()
    } else {
        format!("{name}.md")
    };
    if is_breath_name(&name) {
        Ok(name)
    } else {
        Err(Error::invalid("breath", file.to_owned()))
    }
}

/// Move a breath along; `adopt` and `drop` also touch its source and append a
/// `breath` event, which is returned.
pub fn transition(
    ctx: &Context,
    file: &str,
    to: Transition,
    reason: Option<&str>,
) -> Result<(Breath, Option<trail::Event>)> {
    let name = breath_name(file)?;
    // a drop without a reason is the command line's to refuse
    let reason = reason.map(one_line).filter(|reason| !reason.is_empty());
    let breath = {
        let _lock = CommitLock::acquire(&ctx.data_root)?;
        let path = paths::breath_dir(&ctx.data_root).join(&name);
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(source) if source.kind() == ErrorKind::NotFound => {
                return Err(Error::not_found("breath", name));
            }
            Err(source) => return Err(Error::io(&path, source)),
        };
        let mut breath =
            Breath::parse(&name, &raw).ok_or_else(|| Error::invalid("breath", name.clone()))?;
        if !to.from().contains(&breath.state.as_str()) {
            return Err(Error::refused(format!(
                "{name} is {}; {} takes only {}",
                breath.state,
                to.state(),
                to.from().join("|")
            )));
        }
        to.state().clone_into(&mut breath.state);
        breath.updated = ctx.now.iso8601();
        if to == Transition::Drop {
            breath.reason.clone_from(&reason);
        }
        if to == Transition::Adopt {
            breath.adopted = Some(breath.updated.clone());
        }
        atomic::write(&path, &breath.render())?;
        if to != Transition::Spike
            && let Some(name) = &breath.source
        {
            let mut sources = Sources::read(&ctx.data_root)?;
            if let Some(source) = sources.get_mut(name) {
                if to == Transition::Adopt {
                    source.adopted += 1;
                    source.silent_since = Some(ctx.now);
                } else if source.silent_since.is_none() {
                    source.silent_since = Some(ctx.now);
                }
                sources.write(&ctx.data_root)?;
            }
        }
        breath
    };
    if to == Transition::Spike {
        return Ok((breath, None));
    }
    let mut refs = vec![
        format!("breath:{}", breath.file),
        format!("state:{}", to.state()),
    ];
    if to == Transition::Adopt
        && let Some(shift) = &ctx.shift
    {
        refs.push(format!("adopted-by:{shift}"));
    }
    let rationale = match &reason {
        Some(reason) => format!("{}: {} — {reason}", to.state(), breath.title),
        None => format!("{}: {}", to.state(), breath.title),
    };
    let event = trail::append(
        &ctx.data_root,
        trail::Draft {
            kind: "breath".to_owned(),
            rationale: Some(rationale),
            refs,
            repo: Some(breath.repo.clone()),
            shift: ctx.shift.clone(),
            ..trail::Draft::default()
        },
    )?;
    Ok((breath, Some(event)))
}

// ─── passes ──────────────────────────────────────────────────────────────

/// A pass's depth.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PassKind {
    /// The daily `/gigaresearch` pass over every source.
    Deep,
    /// The hourly read-and-judge pass over the sources due.
    Skim,
}

impl PassKind {
    /// `skim` or `deep`.
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "deep" => Ok(Self::Deep),
            "skim" => Ok(Self::Skim),
            other => Err(Error::invalid("pass kind", other.to_owned())),
        }
    }

    /// Its name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Deep => "deep",
            Self::Skim => "skim",
        }
    }

    /// How long an open pass is in flight before it is dead.
    #[must_use]
    pub fn timeout_seconds(self) -> i64 {
        match self {
            Self::Deep => 3_600,
            Self::Skim => 900,
        }
    }

    /// The `pass.cadence` slot.
    #[must_use]
    pub fn cadence(self) -> &'static str {
        match self {
            Self::Deep => "daily deep",
            Self::Skim => "hourly skim",
        }
    }
}

/// `b-<yyyymmddThhmmss>Z-<kind>`.
#[must_use]
pub fn pass_id(at: Timestamp, kind: PassKind) -> String {
    format!("b-{}Z-{}", at.stamp(), kind.name())
}

/// What `pass` composed.
#[derive(Clone, Debug, PartialEq)]
pub struct Composed {
    /// The pass id.
    pub id: String,
    /// The sources it lists.
    pub sources: Vec<String>,
    /// The prompt.
    pub text: String,
}

/// One pass file, read.
fn read_pass(data_root: &Path, id: &str) -> Result<(PathBuf, Vec<(String, Value)>)> {
    if id.contains(['/', '\\']) || id.starts_with('.') {
        return Err(Error::invalid("pass id", id.to_owned()));
    }
    let path = paths::breath_pass(data_root, id).join("pass.json");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(source) if source.kind() == ErrorKind::NotFound => {
            return Err(Error::not_found("pass", id.to_owned()));
        }
        Err(source) => return Err(Error::io(&path, source)),
    };
    let value = json::parse(&text).map_err(|error| Error::Json {
        path: Some(path.clone()),
        message: error.to_string(),
    })?;
    let entries = value
        .as_object()
        .map(<[(String, Value)]>::to_vec)
        .ok_or_else(|| Error::invalid("pass", id.to_owned()))?;
    Ok((path, entries))
}

/// Set `key` in a pass object.
fn set(entries: &mut Vec<(String, Value)>, key: &str, value: Value) {
    entries.retain(|(name, _)| name != key);
    entries.push((key.to_owned(), value));
}

/// Write a pass object, keys alphabetical. The caller holds the lock.
fn write_pass(path: &Path, mut entries: Vec<(String, Value)>) -> Result<()> {
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    atomic::write(path, &format!("{}\n", Value::Object(entries).render()))
}

/// The passes' ledger: `cost_usd` summed over those started on `now`'s UTC
/// day, and the ids of the passes still in flight. An open pass younger than
/// its kind's timeout is in flight, and refuses a second pass of `kind`
/// (`pass in flight`); an older one is dead — it costs `dead_cost_usd` and,
/// when `write`, is closed as `dead: true`. The caller holds the lock to write.
fn ledger(
    data_root: &Path,
    now: Timestamp,
    kind: PassKind,
    dead_cost_usd: f64,
    write: bool,
) -> Result<(f64, Vec<String>)> {
    let (year, month, day, ..) = now.parts();
    let today = format!("{year:04}-{month:02}-{day:02}");
    let mut ids: Vec<String> = fs::read_dir(paths::breath_passes(data_root))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    ids.sort();
    let (mut spent, mut in_flight) = (0.0, Vec::new());
    for id in ids {
        let Ok((path, mut entries)) = read_pass(data_root, &id) else {
            continue;
        };
        let pass = Value::Object(entries.clone());
        let text = |key: &str| pass.get(key).and_then(Value::as_str);
        let started = text("started").unwrap_or_default();
        let mut cost = pass.get("cost_usd").and_then(Value::as_number);
        if text("ended").is_none() {
            let of = PassKind::parse(text("kind").unwrap_or_default()).unwrap_or(PassKind::Skim);
            let age = Timestamp::parse_iso8601(started)
                .map(|started| now.unix_seconds() - started.unix_seconds());
            if age.is_some_and(|age| age < of.timeout_seconds()) {
                if of == kind {
                    return Err(Error::refused(format!(
                        "pass in flight: {id} started {started}"
                    )));
                }
                in_flight.push(id);
                continue;
            }
            cost = Some(dead_cost_usd);
            if write {
                set(&mut entries, "cost_usd", Value::Number(dead_cost_usd));
                set(&mut entries, "dead", Value::Bool(true));
                set(&mut entries, "ended", Value::string(now.iso8601()));
                write_pass(&path, entries)?;
            }
        }
        if started.starts_with(&today) {
            spent += cost.unwrap_or_default();
        }
    }
    Ok((spent, in_flight))
}

/// Compose the pass prompt from `prompts/breathe.md`; unless `dry_run`, write
/// `passes/<id>/pass.json` and `passes/<id>/prompt.md`.
///
/// Refuses ([`Error::Refused`]) when no source is due — `nothing due` —, a
/// pass of `kind` is in flight — `pass in flight` — or the day's budget is
/// spent — `budget spent` — and the runner then starts
/// no session; any other error (a missing template, an invalid
/// `sources.toml`) is a fault for the runner to report.
pub fn pass(
    ctx: &Context,
    kind: PassKind,
    budget_usd: f64,
    dead_cost_usd: f64,
    dry_run: bool,
) -> Result<Composed> {
    let template_path = ctx.prompts.join("breathe.md");
    let template = fs::read_to_string(&template_path).map_err(|source| {
        if source.kind() == ErrorKind::NotFound {
            Error::not_found("breathe template", template_path.display().to_string())
        } else {
            Error::io(&template_path, source)
        }
    })?;
    let id = pass_id(ctx.now, kind);
    let due: Vec<Source> = sources(ctx, Some(kind))?;
    if due.is_empty() {
        return Err(Error::refused(format!(
            "nothing due: no source is due for a {} pass",
            kind.name()
        )));
    }
    let lock = (!dry_run)
        .then(|| CommitLock::acquire(&ctx.data_root))
        .transpose()?;
    let (spent, in_flight) = ledger(&ctx.data_root, ctx.now, kind, dead_cost_usd, lock.is_some())?;
    if spent >= budget_usd {
        return Err(Error::refused(format!(
            "budget spent: {spent:.2} of {budget_usd} USD today"
        )));
    }
    let problems = fs::read_to_string(paths::breath_problems(&ctx.data_root))
        .ok()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .map_or_else(
            || "none".to_owned(),
            |text| {
                let cut = truncate_chars(&text, PROBLEMS_CHARS);
                if cut.len() < text.len() {
                    format!("{cut}\n… cut at {PROBLEMS_CHARS} characters")
                } else {
                    text
                }
            },
        );
    let mut slots: BTreeMap<&'static str, String> = BTreeMap::new();
    slots.insert("budget.spent", format!("{spent:.2}"));
    slots.insert("budget.usd", format!("{budget_usd}"));
    slots.insert("pass.cadence", kind.cadence().to_owned());
    slots.insert("pass.id", id.clone());
    slots.insert("pass.kind", kind.name().to_owned());
    slots.insert("problems", shift::fenced("breath/problems.md", &problems));
    let mut due_lines: Vec<String> = Vec::new();
    if !in_flight.is_empty() {
        due_lines.push(format!(
            "in flight, not yet in spent: {}",
            in_flight.join(", ")
        ));
        due_lines.push(String::new());
    }
    due_lines.extend(due.iter().map(|source| {
        format!(
            "- `{}` · {} · cursor {} · budget {}",
            source.name,
            source.url,
            source.cursor.as_deref().unwrap_or("none"),
            source.budget
        )
    }));
    slots.insert(
        "sources.due",
        shift::fenced("breath/sources.toml", &due_lines.join("\n")),
    );
    let unknown: Vec<String> = shift::slot_names(&template)
        .into_iter()
        .filter(|name| !slots.contains_key(name.as_str()))
        .collect();
    if !unknown.is_empty() {
        return Err(Error::invalid("breathe template slot", unknown.join(", ")));
    }
    let text = shift::fill(&template, &slots);
    let names: Vec<String> = due.into_iter().map(|source| source.name).collect();
    if dry_run {
        return Ok(Composed {
            id,
            sources: names,
            text,
        });
    }
    let dir = paths::breath_pass(&ctx.data_root, &id);
    let path = dir.join("pass.json");
    if dir.exists() {
        return Err(Error::refused(format!("pass {id} already exists")));
    }
    fs::create_dir_all(&dir).map_err(|source| Error::io(&dir, source))?;
    atomic::write(&dir.join("prompt.md"), &text)?;
    write_pass(
        &path,
        vec![
            ("cost_usd".to_owned(), Value::Null),
            ("dropped".to_owned(), Value::Null),
            ("ended".to_owned(), Value::Null),
            ("filed".to_owned(), Value::Null),
            ("id".to_owned(), Value::string(id.clone())),
            ("kind".to_owned(), Value::string(kind.name())),
            ("read".to_owned(), Value::Null),
            (
                "sources".to_owned(),
                Value::array(names.iter().cloned().map(Value::string)),
            ),
            ("started".to_owned(), Value::string(ctx.now.iso8601())),
        ],
    )
    .map(|()| Composed {
        id,
        sources: names,
        text,
    })
}

/// Record the session the runner started for pass `id`.
pub fn attach(
    ctx: &Context,
    id: &str,
    bg_id: Option<&str>,
    session_id: Option<&str>,
) -> Result<()> {
    let _lock = CommitLock::acquire(&ctx.data_root)?;
    let (path, mut entries) = read_pass(&ctx.data_root, id)?;
    for (key, value) in [("bg_id", bg_id), ("session_id", session_id)] {
        if let Some(value) = value {
            set(&mut entries, key, Value::string(value));
        }
    }
    write_pass(&path, entries)
}

/// Stamp `reaped` into pass `id`: the runner has removed its session from
/// Claude Code's listing and taken its transcript.
pub fn reaped(ctx: &Context, id: &str) -> Result<()> {
    let _lock = CommitLock::acquire(&ctx.data_root)?;
    let (path, mut entries) = read_pass(&ctx.data_root, id)?;
    set(&mut entries, "reaped", Value::string(ctx.now.iso8601()));
    write_pass(&path, entries)
}

// ─── the tick ────────────────────────────────────────────────────────────

/// What one `breathe tick` reads beyond its context, resolved by the caller.
#[derive(Clone, Debug)]
pub struct Tick {
    /// `$SANDMAN_BREATH_BUDGET_USD`.
    pub budget_usd: f64,
    /// `$SANDMAN_BREATH_DEAD_COST_USD`.
    pub dead_cost_usd: f64,
    /// The `dispatch` binary.
    pub dispatch: PathBuf,
    /// `$SANDMAN_BREATH_MIND`: the pass model.
    pub mind: String,
    /// The `shift profile --kind breath` settings, rendered.
    pub settings: String,
    /// The wait after a stop, before the removal: a stopped session's
    /// `SessionEnd` hook races the removal of its job dir.
    pub settle: Duration,
}

/// One `dispatch run` request: its one answer line, or why there is none — a
/// failed spawn or exit, an empty or unparsable answer, or one with `error`.
fn ask(dispatch: &Path, request: &Value) -> std::result::Result<Value, String> {
    let mut child = Command::new(dispatch)
        .arg("run")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| format!("{}: {error}", dispatch.display()))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "{}", request.render());
    }
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    let line = String::from_utf8_lossy(&output.stdout);
    let line = line.lines().next().unwrap_or_default();
    json::parse(line)
        .ok()
        .filter(|answer| output.status.success() && answer.get("error").is_none())
        .ok_or_else(|| format!("answered `{line}` ({})", output.status))
}

/// A `claude` request to dispatch.
fn claude(action: &str, fields: Vec<(&str, Value)>) -> Value {
    Value::object(
        [
            ("verb", Value::string("claude")),
            ("action", Value::string(action)),
        ]
        .into_iter()
        .chain(fields),
    )
}

/// One tick of the ruach, what its launchd agent runs hourly: close, then
/// open. Close removes from Claude Code's listing every session named
/// `breath <kind> <pass-id>` whose pass has `ended`, stopping a working or
/// blocked one first — except a deep one whose pass was marked dead, an
/// overrunning `/gigaresearch` that is spared until 24 h after it started or
/// a newer deep pass starts; its failures are
/// `say`d and passed. Open, unless `shifts/PAUSE` exists, composes a skim
/// pass and — once per New York day, from 06:00 there — a deep one, starts
/// each pass's session through dispatch and attaches it. A pass's waits are
/// `say`d skips; its faults, and a start dispatch refuses, fail the tick after
/// both kinds are tried — the first error wins. `say(line, failed)` hears
/// every decision.
pub fn tick(ctx: &Context, tick: &Tick, say: &mut dyn FnMut(&str, bool)) -> Result<()> {
    let started = |pass: &Value| {
        pass.get("started")
            .and_then(Value::as_str)
            .and_then(Timestamp::parse_iso8601)
            .map(Timestamp::unix_seconds)
    };
    let deep_starts: Vec<i64> = fs::read_dir(paths::breath_passes(&ctx.data_root))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| read_pass(&ctx.data_root, &entry.file_name().to_string_lossy()).ok())
        .map(|(_, entries)| Value::Object(entries))
        .filter(|pass| pass.get("kind").and_then(Value::as_str) == Some("deep"))
        .filter_map(|pass| started(&pass))
        .collect();
    match ask(&tick.dispatch, &claude("list", vec![])) {
        Err(why) => say(&format!("tick list failed: {why}"), true),
        Ok(answer) => {
            let ended: Vec<(&str, &str, bool)> = answer
                .get("sessions")
                .and_then(Value::as_array)
                .unwrap_or_default()
                .iter()
                .filter_map(|row| {
                    let name = row.get("name")?.as_str()?;
                    let ["breath", "skim" | "deep", id] = name.split(' ').collect::<Vec<_>>()[..]
                    else {
                        return None;
                    };
                    let pass = Value::Object(read_pass(&ctx.data_root, id).ok()?.1);
                    pass.get("ended")?.as_str()?;
                    let bg = row.get("id")?.as_str()?;
                    let working = matches!(
                        row.get("state").and_then(Value::as_str),
                        Some("working" | "blocked")
                    );
                    let current = started(&pass).is_some_and(|at| {
                        ctx.now.unix_seconds() - at < 86_400
                            && deep_starts.iter().all(|other| *other <= at)
                    });
                    if working
                        && current
                        && pass.get("kind").and_then(Value::as_str) == Some("deep")
                        && pass.get("dead").and_then(Value::as_bool) == Some(true)
                    {
                        say(
                            &format!(
                                "tick spare {bg} ({name}): dead pass still working, under 24 h"
                            ),
                            false,
                        );
                        return None;
                    }
                    Some((bg, name, working))
                })
                .collect();
            let mut act = |action: &str, bg: &str, name: &str| {
                let done = ask(
                    &tick.dispatch,
                    &claude(action, vec![("id", Value::string(bg))]),
                );
                let why = done.as_ref().err().map_or("ok", String::as_str);
                say(
                    &format!("tick {action} {bg} ({name}): {why}"),
                    done.is_err(),
                );
            };
            let working: Vec<_> = ended.iter().filter(|session| session.2).collect();
            for (bg, name, _) in &working {
                act("stop", bg, name);
            }
            if !working.is_empty() {
                thread::sleep(tick.settle);
            }
            for (bg, name, _) in &ended {
                act("rm", bg, name);
            }
        }
    }
    if paths::shifts_dir(&ctx.data_root).join("PAUSE").exists() {
        say("tick skip: shifts/PAUSE", false);
        return Ok(());
    }
    let settings = json::parse(&tick.settings).map_err(|error| Error::Json {
        path: None,
        message: error.to_string(),
    })?;
    let local = |at: Timestamp| shift::local_seconds(at).div_euclid(3_600);
    let (today, hour) = (local(ctx.now).div_euclid(24), local(ctx.now).rem_euclid(24));
    let deep_today = deep_starts
        .iter()
        .any(|at| local(Timestamp::from_unix_seconds(*at)).div_euclid(24) == today);
    let mut outcome = Ok(());
    for kind in [PassKind::Skim, PassKind::Deep] {
        if kind == PassKind::Deep && (hour < 6 || deep_today) {
            say("tick skip deep: once a New York day, from 06:00", false);
        } else {
            outcome = outcome.and(open(ctx, tick, kind, &settings, say));
        }
    }
    outcome
}

/// Compose a `kind` pass, start its session and attach it; a wait is a
/// `say`d skip.
fn open(
    ctx: &Context,
    tick: &Tick,
    kind: PassKind,
    settings: &Value,
    say: &mut dyn FnMut(&str, bool),
) -> Result<()> {
    let composed = match pass(ctx, kind, tick.budget_usd, tick.dead_cost_usd, false) {
        Ok(composed) => composed,
        Err(error @ (Error::Refused { .. } | Error::LockHeld { .. })) => {
            say(&format!("tick skip {}: {error}", kind.name()), false);
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let request = claude(
        "start",
        vec![
            ("prompt", Value::string(composed.text)),
            (
                "cwd",
                Value::string(paths::breath_dir(&ctx.data_root).to_string_lossy()),
            ),
            ("model", Value::string(tick.mind.clone())),
            ("permission_mode", Value::string("dontAsk")),
            ("settings", settings.clone()),
            (
                "name",
                Value::string(format!("breath {} {}", kind.name(), composed.id)),
            ),
        ],
    );
    let answer = ask(&tick.dispatch, &request)
        .and_then(|answer| {
            answer
                .get("id")
                .and_then(Value::as_str)
                .is_some()
                .then_some(answer)
                .ok_or_else(|| "answered no id".to_owned())
        })
        .map_err(|why| {
            Error::refused(format!(
                "dispatch start failed for pass {}: {why}",
                composed.id
            ))
        })?;
    let field = |key| answer.get(key).and_then(Value::as_str);
    attach(ctx, &composed.id, field("id"), field("session_id"))?;
    say(
        &format!(
            "tick opened {} bg={} session={}",
            composed.id,
            field("id").unwrap_or_default(),
            field("session_id").unwrap_or_default()
        ),
        false,
    );
    Ok(())
}

/// What `close` records.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Closing {
    /// What the pass cost, US dollars — required.
    pub cost_usd: f64,
    /// `(source, cursor)` pairs to advance.
    pub cursors: Vec<(String, String)>,
    /// Candidates dropped.
    pub dropped: u64,
    /// Breaths filed.
    pub filed: u64,
    /// Items read.
    pub read: u64,
}

/// Close pass `id`: its counters and `ended`, `last_read` (its `started`) and any cursor on
/// the sources it listed, and a `breath` event. A pass marked dead still
/// closes — an overrunning session's real cost and counters replace the
/// dead charge — but a pass closed by its session closes once.
pub fn close(ctx: &Context, id: &str, closing: &Closing) -> Result<trail::Event> {
    if !closing.cost_usd.is_finite() || closing.cost_usd < 0.0 {
        return Err(Error::invalid("cost", closing.cost_usd.to_string()));
    }
    // A cursor is rendered into the pass prompt's line list; a control
    // character there would forge a line.
    if let Some(pair) = closing
        .cursors
        .iter()
        .map(|(name, cursor)| format!("{name}={cursor}"))
        .find(|pair| pair.contains(char::is_control))
    {
        return Err(Error::invalid("cursor", pair));
    }
    let kind = {
        let _lock = CommitLock::acquire(&ctx.data_root)?;
        let (path, mut entries) = read_pass(&ctx.data_root, id)?;
        let field = |key: &str| {
            entries
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        };
        if let Some(ended) = field("ended").as_ref().and_then(Value::as_str)
            && field("dead").as_ref().and_then(Value::as_bool) != Some(true)
        {
            return Err(Error::refused(format!("pass {id} closed at {ended}")));
        }
        let kind = field("kind")
            .as_ref()
            .and_then(Value::as_str)
            .unwrap_or("skim")
            .to_owned();
        let listed: Vec<String> = field("sources")
            .as_ref()
            .and_then(Value::as_array)
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect();
        if let Some((name, _)) = closing
            .cursors
            .iter()
            .find(|(name, _)| !listed.contains(name))
        {
            return Err(Error::invalid(
                "cursor source (not in the pass)",
                name.clone(),
            ));
        }
        // read as of the pass's start: stamped at close, a minute after the
        // hourly tick, the next tick's skim would find nothing due
        let started = field("started")
            .as_ref()
            .and_then(Value::as_str)
            .and_then(Timestamp::parse_iso8601)
            .unwrap_or(ctx.now);
        let mut sources = Sources::read(&ctx.data_root)?;
        for name in &listed {
            if let Some(source) = sources.get_mut(name) {
                source.last_read = Some(started);
                if let Some((_, cursor)) = closing
                    .cursors
                    .iter()
                    .rev()
                    .find(|(cursor_of, _)| cursor_of == name)
                {
                    source.cursor = Some(cursor.clone());
                }
                if source.silent_since.is_none() {
                    source.silent_since = Some(ctx.now);
                }
            }
        }
        entries.retain(|(name, _)| name != "dead");
        set(&mut entries, "cost_usd", Value::Number(closing.cost_usd));
        set(&mut entries, "dropped", count(closing.dropped));
        set(&mut entries, "ended", Value::string(ctx.now.iso8601()));
        set(&mut entries, "filed", count(closing.filed));
        set(&mut entries, "read", count(closing.read));
        write_pass(&path, entries)?;
        sources.write(&ctx.data_root)?;
        kind
    };
    trail::append(
        &ctx.data_root,
        trail::Draft {
            cost_usd: Some(closing.cost_usd),
            kind: "breath".to_owned(),
            rationale: Some(format!(
                "pass {kind}: read {} filed {} dropped {}",
                closing.read, closing.filed, closing.dropped
            )),
            refs: vec![format!("pass:{id}")],
            ..trail::Draft::default()
        },
    )
}

// ─── reweight ────────────────────────────────────────────────────────────

/// One source's weight moved.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    /// Adoptions counted.
    pub adopted: u64,
    /// The source.
    pub name: String,
    /// Its weight after.
    pub new: f64,
    /// Its weight before.
    pub old: f64,
    /// Whether a month of silence lowered it.
    pub silent: bool,
}

impl Change {
    /// The table row.
    #[must_use]
    pub fn line(&self) -> String {
        let mut why = Vec::new();
        if self.adopted > 0 {
            why.push(format!("+{} adopted", self.adopted));
        }
        if self.silent {
            why.push("30 days silent".to_owned());
        }
        format!(
            "{}  {} → {}  {}",
            self.name,
            weight_text(self.old),
            weight_text(self.new),
            why.join(", ")
        )
    }
}

/// Two decimals — weights move in tenths, and float drift must not show.
fn cents(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// Per source: `+0.1` per breath whose `adopted` stamp is after the last
/// reweight — never `updated`, which a `seen_again` moves — (cap 2.0),
/// `−0.1` when `silent_since` is over thirty days old (floor 0.2, and the
/// silence clock restarts). Unless `dry_run`, rewrite `sources.toml` with
/// `reweighted` stamped and append one `breath` event.
pub fn reweight(ctx: &Context, dry_run: bool) -> Result<(Vec<Change>, Option<trail::Event>)> {
    let changes = {
        let _lock = CommitLock::acquire(&ctx.data_root)?;
        let mut sources = Sources::read(&ctx.data_root)?;
        let since = sources.reweighted;
        let adopted = breaths(&ctx.data_root)?;
        let mut changes = Vec::new();
        for source in &mut sources.sources {
            let count = adopted
                .iter()
                .filter(|breath| {
                    breath.state == "adopted"
                        && breath.source.as_deref() == Some(source.name.as_str())
                        && breath
                            .adopted
                            .as_deref()
                            .and_then(Timestamp::parse_iso8601)
                            .is_some_and(|at| {
                                since.is_none_or(|since| at.unix_seconds() > since.unix_seconds())
                            })
                })
                .count();
            let count = u64::try_from(count).unwrap_or(u64::MAX);
            let silent = source
                .silent_since
                .is_some_and(|at| ctx.now.unix_seconds() - at.unix_seconds() > SILENCE_SECONDS);
            let old = source.weight;
            let mut new = old;
            if count > 0 {
                #[allow(clippy::cast_precision_loss, reason = "adoption counts are tiny")]
                let raise = WEIGHT_STEP * count as f64;
                new = cents(new + raise).min(WEIGHT_CAP.max(old));
            }
            if silent {
                new = cents(new - WEIGHT_STEP).max(WEIGHT_FLOOR.min(old));
                source.silent_since = Some(ctx.now);
            }
            if (new - old).abs() > f64::EPSILON || silent || count > 0 {
                source.weight = new;
                changes.push(Change {
                    adopted: count,
                    name: source.name.clone(),
                    new,
                    old,
                    silent,
                });
            }
        }
        if !dry_run {
            sources.reweighted = Some(ctx.now);
            sources.write(&ctx.data_root)?;
        }
        changes
    };
    if dry_run {
        return Ok((changes, None));
    }
    let moved = changes
        .iter()
        .filter(|change| (change.new - change.old).abs() > f64::EPSILON)
        .count();
    let event = trail::append(
        &ctx.data_root,
        trail::Draft {
            kind: "breath".to_owned(),
            rationale: Some(format!("reweight: {moved} source(s) moved")),
            refs: changes
                .iter()
                .map(|change| {
                    format!(
                        "weight:{}={}→{}",
                        change.name,
                        weight_text(change.old),
                        weight_text(change.new)
                    )
                })
                .collect(),
            ..trail::Draft::default()
        },
    )?;
    Ok((changes, Some(event)))
}

#[cfg(test)]
mod tests {
    use super::{
        Breath, PassKind, SEED, Sources, is_breath_name, jaccard, normal_url, pass_id,
        same_numbers, tokens, valid_repo,
    };
    use crate::time::Timestamp;

    #[test]
    fn jaccard_counts_shared_title_tokens() {
        let jaccard = |left, right| jaccard(&tokens(left), &tokens(right));
        assert!(
            (jaccard("Fast incremental parsing", "fast INCREMENTAL parsing!") - 1.0).abs() < 1e-9
        );
        // {a b c} vs {a b d}: 2 of 4.
        assert!((jaccard("a b c", "a b d") - 0.5).abs() < 1e-9);
        // {a b c d} vs {a b c e}: 3 of 5.
        assert!((jaccard("a b c d", "a-b-c-e") - 0.6).abs() < 1e-9);
        assert!(jaccard("", "") < f64::EPSILON);
        assert!(jaccard("one", "two") < f64::EPSILON);
    }

    #[test]
    fn urls_normalize_scheme_host_slash_tracking_and_arxiv_versions() {
        let same = "https://example.com/post";
        for url in [
            "http://example.com/post",
            "https://EXAMPLE.com/post/",
            "https://example.com/post?utm_source=hn&utm_medium=x",
        ] {
            assert_eq!(normal_url(url), same, "{url}");
        }
        assert_eq!(
            normal_url("https://example.com/p?id=3&utm_campaign=z&ref=y"),
            "https://example.com/p?id=3&ref=y"
        );
        assert_eq!(normal_url("Example.com/a/"), "example.com/a");
        for (left, right) in [
            (
                "https://example.com/#/post/1",
                "https://example.com/#/post/2",
            ),
            (
                "https://example.com/p?ref=v1",
                "https://example.com/p?ref=v2",
            ),
        ] {
            assert_ne!(normal_url(left), normal_url(right), "{left}");
        }
        assert_eq!(
            normal_url("https://example.com/p/?utm_source=x#top"),
            "https://example.com/p#top"
        );
        assert_eq!(
            normal_url("https://example.com/Post"),
            "https://example.com/Post"
        );
        assert_eq!(
            normal_url("http://arxiv.org/abs/2510.01234v2"),
            "https://arxiv.org/abs/2510.01234"
        );
        assert_eq!(
            normal_url("https://arxiv.org/pdf/2510.01234v11.pdf"),
            "https://arxiv.org/pdf/2510.01234.pdf"
        );
        assert_eq!(
            normal_url("https://arxiv.org/abs/cs/0101001v1"),
            "https://arxiv.org/abs/cs/0101001"
        );
        assert_eq!(
            normal_url("https://example.com/release/v2"),
            "https://example.com/release/v2",
            "the version cut is arXiv's alone"
        );
    }

    #[test]
    fn numbers_agree_in_order() {
        assert!(!same_numbers("Release 1.0.0", "Release 1.1.0"));
        assert!(!same_numbers("Claude Code 2.0.15", "Claude Code 2.0.14"));
        assert!(!same_numbers("2 of 10", "10 of 2"));
        assert!(same_numbers("v1.2. notes, 3 ways", "notes v1.2 in 3 ways."));
    }

    #[test]
    fn repos_are_star_or_owner_slash_name() {
        for good in ["*", "jgeschwendt/sandman", "jgeschwendt/jlg.io", "a_b/c-d"] {
            assert!(valid_repo(good), "{good}");
        }
        for bad in [
            "", "sandman", "a/b/c", "/x", "x/", "../x", "a/..", "a b/c", "**",
        ] {
            assert!(!valid_repo(bad), "{bad}");
        }
    }

    #[test]
    fn the_seed_is_valid_sorted_and_renders_back_to_itself() {
        let sources = Sources::parse(SEED).expect("seed");
        assert!(sources.sources.len() >= 10);
        let names: Vec<&str> = sources
            .sources
            .iter()
            .map(|source| source.name.as_str())
            .collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
        assert_eq!(Sources::parse(&sources.render()).expect("reparse"), sources);
    }

    #[test]
    fn breaths_render_and_parse_back() {
        let breath = Breath {
            adopted: Some("2026-10-09T13:00:00Z".to_owned()),
            bearing: Some(0.8),
            file: "2026-10-09-a-thing.md".to_owned(),
            filed: "2026-10-09T12:00:00Z".to_owned(),
            novelty: None,
            problem: "'recall' over budget".to_owned(),
            reason: Some("superseded by 'v2'".to_owned()),
            repo: "jgeschwendt/sandman".to_owned(),
            seen_again: 2,
            source: Some("releases-oxc".to_owned()),
            state: "new".to_owned(),
            summary: "one\ntwo\nthree\nfour".to_owned(),
            title: "A thing: \"with a colon\"".to_owned(),
            updated: "2026-10-09T13:00:00Z".to_owned(),
            url: "https://example.com/a?b=c".to_owned(),
        };
        assert_eq!(Breath::parse(&breath.file, &breath.render()), Some(breath));
    }

    #[test]
    fn names_and_ids() {
        assert!(is_breath_name("2026-10-09-x.md"));
        assert!(!is_breath_name("problems.md"));
        assert!(!is_breath_name("2026-10-09.md"));
        assert!(!is_breath_name("2026-1009-xx.md"));
        assert_eq!(
            pass_id(Timestamp::from_unix_seconds(1_791_589_262), PassKind::Skim),
            "b-20261009T234102Z-skim"
        );
    }
}
