//! `score` — the week's numbers, derived from what the machine already keeps:
//! the trail, `shifts/*/meta.json`, the breath store, `~/.threads` and the
//! routines' `runs.jsonl`. No model, no network, no git.
//!
//! The page is `score/<yyyy>-W<ww>.md`, rewritten whole under `.commit.lock`
//! on every run. The contract is `docs/BANK-FORMAT.md` § score.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use crate::atomic;
use crate::error::{Error, Result};
use crate::json::{self, Value};
use crate::lock::CommitLock;
use crate::paths;
use crate::time::{Timestamp, civil_from_days, days_from_civil};
use crate::verbs::breathe;
use crate::verbs::shift::{self, Local};
use crate::verbs::trail;

/// The stale-context ratio above which the week is off target.
const STALE_TARGET: f64 = 0.5;

/// An ISO week, its days counted in New York.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Week {
    /// ISO week number, 1–53.
    pub week: i64,
    /// ISO week-numbering year.
    pub year: i64,
}

impl Week {
    /// The week `at` falls in, in New York.
    #[must_use]
    pub fn of(at: Timestamp) -> Self {
        let (year, week) = Local::at(at).iso_week;
        Self { week, year }
    }

    /// The week a targets file plans for: the next New York day's, so Sunday —
    /// the Hebrew week's first day — reads the plan the Sabbath before it wrote.
    #[must_use]
    pub fn targets(at: Timestamp) -> Self {
        Self::of(Timestamp::from_unix_seconds(at.unix_seconds() + 86_400))
    }

    /// The week holding the date `days` since the epoch.
    fn of_day(days: i64) -> Self {
        Self::of(Timestamp::from_unix_seconds(days * 86_400 + 12 * 3600))
    }

    /// `yyyy-Www`, refusing a week its year does not have.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (year, week) = text.split_once("-W")?;
        if year.len() != 4
            || week.len() != 2
            || !format!("{year}{week}").bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let parsed = Self {
            week: week.parse().ok()?,
            year: year.parse().ok()?,
        };
        (parsed.week >= 1 && Self::of_day(parsed.monday()) == parsed).then_some(parsed)
    }

    /// Its Monday, days since the epoch.
    fn monday(self) -> i64 {
        let fourth = days_from_civil(self.year, 1, 4);
        fourth - (fourth + 3).rem_euclid(7) + (self.week - 1) * 7
    }

    /// `yyyy-Www`.
    #[must_use]
    pub fn name(self) -> String {
        format!("{:04}-W{:02}", self.year, self.week)
    }

    /// The week `weeks` after this one, before it when negative.
    #[must_use]
    pub fn offset(self, weeks: i64) -> Self {
        Self::of_day(self.monday() + 7 * weeks)
    }

    /// Whether `at` falls in it.
    #[must_use]
    pub fn contains(self, at: Timestamp) -> bool {
        Self::of(at) == self
    }

    /// Every seventh week is the Shemitah's.
    #[must_use]
    pub fn shemitah(self) -> bool {
        self.week % 7 == 0
    }

    /// Its first and last day, `yyyy-mm-dd`.
    fn bounds(self) -> (String, String) {
        let ymd = |days| {
            let (year, month, day) = civil_from_days(days);
            format!("{year:04}-{month:02}-{day:02}")
        };
        (ymd(self.monday()), ymd(self.monday() + 6))
    }
}

/// Breaths filed, spiked, adopted and dropped in `week`, by their stamps.
pub(crate) fn breath_counts(breaths: &[breathe::Breath], week: Week) -> [usize; 4] {
    let within = |at: &str| Timestamp::parse_iso8601(at).is_some_and(|at| week.contains(at));
    let moved = |state: &str| {
        breaths
            .iter()
            .filter(|b| b.state == state && within(&b.updated))
            .count()
    };
    [
        breaths.iter().filter(|b| within(&b.filed)).count(),
        moved("spiked"),
        breaths
            .iter()
            .filter(|b| b.adopted.as_deref().is_some_and(within))
            .count(),
        moved("dropped"),
    ]
}

/// Counts by key.
type Tally = BTreeMap<String, usize>;

fn tally(map: &mut Tally, key: &str) {
    *map.entry(key.to_owned()).or_default() += 1;
}

/// `part / whole`, 0 when there is no whole.
#[allow(clippy::cast_precision_loss, reason = "weekly counts are tiny")]
fn share(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

/// Two decimals.
fn cents(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// One metric off its target.
#[derive(Clone, Debug, PartialEq)]
pub struct Miss {
    /// How far off, 0–1, for the ranking.
    pub distance: f64,
    /// Its name.
    pub metric: &'static str,
    /// What it should be.
    pub target: String,
    /// What it is.
    pub value: String,
}

/// The week's metrics.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Score {
    /// Breaths filed, spiked, adopted, dropped this week.
    pub breath: [usize; 4],
    /// Per source: name, weight, filed and adopted, all time.
    pub breath_sources: Vec<(String, f64, u64, u64)>,
    /// `breath` events by their `state:` ref.
    pub breath_states: Tally,
    /// `change` events carrying a `sha`.
    pub changes_with_sha: usize,
    /// Context-slot fingerprints, and how many had not appeared earlier this week.
    pub context: (usize, usize),
    /// Corrections this week, and those echoing an earlier one.
    pub corrections: (usize, usize),
    /// Shift handoffs and breath pass closes — the edges a cost is read at.
    pub cost_edges: usize,
    /// Edges carrying no `cost_usd`.
    pub cost_unrecorded: usize,
    /// `cost_usd` summed.
    pub cost_usd: f64,
    /// Each rotation trunk and its weekday shifts this week.
    pub coverage: Vec<(String, usize)>,
    /// Events this week.
    pub events: usize,
    /// Events by kind, every kind listed.
    pub kinds: Tally,
    /// Events by model.
    pub models: Tally,
    /// Events by repo.
    pub repos: Tally,
    /// Per routine: ticks this week, non-zero exits.
    pub routines: Vec<(String, usize, usize)>,
    /// Shifts opened, handed off, abandoned; mean minutes start to handoff;
    /// `cost_usd` per handoff.
    pub shifts: (usize, usize, usize, Option<i64>, Option<f64>),
    /// Open threads by kind.
    pub threads: Tally,
    /// Open threads a shift opened.
    pub threads_from_shifts: usize,
    /// The week.
    pub week: Week,
}

/// Compute `week`'s score.
pub fn score(ctx: &shift::Context, routines: &Path, week: Week) -> Result<Score> {
    let last = week.offset(-1);
    let (since, _) = last.bounds();
    let entries = trail::read(&ctx.data_root, Some(&previous_day(&since)))?;
    let all: Vec<&trail::Event> = entries.iter().map(|entry| &entry.event).collect();
    let events: Vec<&trail::Event> = all
        .iter()
        .copied()
        .filter(|e| week.contains(e.at))
        .collect();
    let mut score = Score {
        events: events.len(),
        kinds: trail::KINDS
            .iter()
            .map(|kind| ((*kind).to_owned(), 0))
            .collect(),
        week,
        ..Score::default()
    };
    for event in &events {
        let draft = &event.draft;
        tally(&mut score.kinds, &draft.kind);
        tally(&mut score.models, draft.model.as_deref().unwrap_or("-"));
        tally(&mut score.repos, draft.repo.as_deref().unwrap_or("-"));
        if draft.kind == "breath" {
            let state = draft.refs.iter().find_map(|r| r.strip_prefix("state:"));
            tally(&mut score.breath_states, state.unwrap_or("-"));
        }
        score.changes_with_sha += usize::from(draft.kind == "change" && draft.sha.is_some());
        score.cost_usd += draft.cost_usd.unwrap_or_default();
        let edge = draft.kind == "shift.handoff"
            || (draft.kind == "breath" && draft.refs.iter().any(|r| r.starts_with("pass:")));
        score.cost_edges += usize::from(edge);
        score.cost_unrecorded += usize::from(edge && draft.cost_usd.is_none());
    }

    let of_kind = |kind: &'static str| events.iter().copied().filter(move |e| e.draft.kind == kind);
    let abandoned = of_kind("shift.handoff")
        .filter(|e| {
            e.draft.refs.iter().any(|r| r == "abandoned")
                || e.draft
                    .rationale
                    .as_deref()
                    .is_some_and(|r| r.starts_with("Abandoned:"))
        })
        .count();
    let handoffs = of_kind("shift.handoff").count();
    let minutes: Vec<i64> = of_kind("shift.start")
        .filter_map(|start| {
            let end = all
                .iter()
                .find(|e| e.draft.kind == "shift.handoff" && e.draft.shift == start.draft.shift)?;
            Some(end.at.unix_seconds() - start.at.unix_seconds())
        })
        .collect();
    let mean = i64::try_from(minutes.len())
        .ok()
        .filter(|n| *n > 0)
        .map(|n| (minutes.iter().sum::<i64>() + n * 30) / (n * 60));
    let spent: f64 = of_kind("shift.handoff")
        .filter_map(|e| e.draft.cost_usd)
        .sum();
    score.shifts = (
        of_kind("shift.start").count(),
        handoffs - abandoned,
        abandoned,
        mean,
        (handoffs > 0).then(|| spent * share(1, handoffs)),
    );

    let corrections: Vec<&trail::Event> = all
        .iter()
        .copied()
        .filter(|e| e.draft.kind == "correction" && (week.contains(e.at) || last.contains(e.at)))
        .collect();
    let said = |e: &trail::Event| e.draft.rationale.clone().unwrap_or_default();
    score.corrections = (
        of_kind("correction").count(),
        corrections
            .iter()
            .enumerate()
            .filter(|(at, now)| {
                week.contains(now.at)
                    && corrections[..*at].iter().any(|earlier| {
                        breathe::jaccard(&said(earlier), &said(now)) >= breathe::JACCARD_SAME
                    })
            })
            .count(),
    );

    let mut seen = BTreeSet::new();
    for fingerprint in of_kind("shift.start")
        .flat_map(|e| e.draft.refs.iter().filter_map(|r| r.strip_prefix("fp:")))
    {
        if shift::is_context(fingerprint.split('=').next().unwrap_or_default()) {
            score.context.0 += 1;
            score.context.1 += usize::from(seen.insert(fingerprint));
        }
    }

    let mut shifted = Tally::new();
    for meta in shift::metas(&ctx.data_root) {
        if let (Some(repo), Some(started)) = (&meta.repo, meta.started)
            && week.contains(started)
            && !ctx.night.sabbath(started)
        {
            tally(&mut shifted, repo);
        }
    }
    score.coverage = shift::trunks(ctx)
        .into_iter()
        .map(|(label, _)| {
            let count = shifted.get(&label).copied().unwrap_or_default();
            (label, count)
        })
        .collect();

    score.breath = breath_counts(&breathe::breaths(&ctx.data_root)?, week);
    score.breath_sources = match breathe::Sources::read(&ctx.data_root) {
        Ok(sources) => sources
            .sources
            .into_iter()
            .map(|s| (s.name, s.weight, s.filed, s.adopted))
            .collect(),
        Err(Error::NotFound { .. }) => Vec::new(),
        Err(error) => return Err(error),
    };

    for (_, fields, _) in shift::front_files(&ctx.threads) {
        if fields.get("state").map(String::as_str) == Some("open") {
            tally(
                &mut score.threads,
                fields.get("kind").map_or("-", String::as_str),
            );
            score.threads_from_shifts += usize::from(["anchor", "key"].iter().any(|key| {
                fields
                    .get(*key)
                    .is_some_and(|value| value.starts_with("shift:"))
            }));
        }
    }
    score.routines = routine_runs(routines, week);
    Ok(score)
}

/// The day before `ymd` — trail files are UTC days, the week New York's.
fn previous_day(ymd: &str) -> String {
    let parts: Vec<i64> = ymd
        .split('-')
        .filter_map(|part| part.parse().ok())
        .collect();
    let (year, month, day) = civil_from_days(days_from_civil(parts[0], parts[1], parts[2]) - 1);
    format!("{year:04}-{month:02}-{day:02}")
}

/// The `shift` and `breathe-*` routines' ticks and failures in `week`.
fn routine_runs(dir: &Path, week: Week) -> Vec<(String, usize, usize)> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().map(ToOwned::to_owned))
        .filter(|name| name == "shift" || name.starts_with("breathe-"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let text = fs::read_to_string(dir.join(&name).join("runs.jsonl")).unwrap_or_default();
            let runs: Vec<Value> = text
                .lines()
                .filter_map(|line| json::parse(line).ok())
                .filter(|run| {
                    run.get("started")
                        .and_then(Value::as_str)
                        .and_then(Timestamp::parse_iso8601)
                        .is_some_and(|at| week.contains(at))
                })
                .collect();
            let failed = runs
                .iter()
                .filter(|run| {
                    run.get("exit")
                        .and_then(Value::as_number)
                        .is_some_and(|exit| exit.abs() > f64::EPSILON)
                })
                .count();
            (name, runs.len(), failed)
        })
        .collect()
}

impl Score {
    /// The share of context-slot fingerprints new this week; 1 with none.
    fn fresh_ratio(&self) -> f64 {
        let (slots, fresh) = self.context;
        if slots == 0 { 1.0 } else { share(fresh, slots) }
    }

    /// The metrics off their target, furthest first, at most three.
    #[must_use]
    pub fn worst(&self) -> Vec<Miss> {
        let stale = 1.0 - self.fresh_ratio();
        let covered = self.coverage.iter().filter(|(_, n)| *n > 0).count();
        let open: usize = self.threads.values().sum();
        let count = |metric, value: usize, whole: usize| Miss {
            distance: share(value, whole.max(value)),
            metric,
            target: "0".to_owned(),
            value: value.to_string(),
        };
        let mut misses = vec![
            Miss {
                distance: share(self.coverage.len() - covered, self.coverage.len()),
                metric: "coverage",
                target: format!("{0}/{0}", self.coverage.len()),
                value: format!("{covered}/{}", self.coverage.len()),
            },
            count("abandoned shifts", self.shifts.2, self.shifts.0),
            count(
                "recurring corrections",
                self.corrections.1,
                self.corrections.0,
            ),
            count("cost unrecorded", self.cost_unrecorded, self.cost_edges),
            Miss {
                distance: ((stale - STALE_TARGET) / STALE_TARGET).max(0.0),
                metric: "stale context ratio",
                target: format!("≤ {STALE_TARGET:.2}"),
                value: format!("{stale:.2}"),
            },
            count("open shift threads", self.threads_from_shifts, open),
        ];
        misses.retain(|miss| miss.distance > 0.0);
        misses.sort_by(|a, b| {
            b.distance
                .total_cmp(&a.distance)
                .then(a.metric.cmp(b.metric))
        });
        misses.truncate(3);
        misses
    }

    /// The metrics object `--json` prints, keys alphabetical.
    #[must_use]
    pub fn json(&self) -> String {
        let tally =
            |map: &Tally| Value::object(map.iter().map(|(k, v)| (k.as_str(), Value::count(*v))));
        let week = self.week;
        let (from, to) = week.bounds();
        let [filed, spiked, adopted, dropped] = self.breath;
        let (slots, fresh) = self.context;
        Value::object([
            (
                "breath",
                Value::object([
                    ("adopted", Value::count(adopted)),
                    ("dropped", Value::count(dropped)),
                    ("filed", Value::count(filed)),
                    (
                        "sources",
                        Value::array(self.breath_sources.iter().map(
                            |(name, weight, filed, adopted)| {
                                Value::object([
                                    (
                                        "adopted",
                                        Value::count(
                                            usize::try_from(*adopted).unwrap_or(usize::MAX),
                                        ),
                                    ),
                                    (
                                        "filed",
                                        Value::count(usize::try_from(*filed).unwrap_or(usize::MAX)),
                                    ),
                                    ("name", Value::string(name.clone())),
                                    ("weight", Value::Number(*weight)),
                                ])
                            },
                        )),
                    ),
                    ("spiked", Value::count(spiked)),
                ]),
            ),
            (
                "context",
                Value::object([
                    ("fresh", Value::count(fresh)),
                    ("fresh_ratio", Value::Number(cents(self.fresh_ratio()))),
                    ("slots", Value::count(slots)),
                ]),
            ),
            (
                "corrections",
                Value::object([
                    ("recurring", Value::count(self.corrections.1)),
                    ("total", Value::count(self.corrections.0)),
                ]),
            ),
            (
                "coverage",
                Value::object(
                    self.coverage
                        .iter()
                        .map(|(repo, n)| (repo.as_str(), Value::count(*n))),
                ),
            ),
            ("from", Value::string(from)),
            (
                "routines",
                Value::object(self.routines.iter().map(|(name, ticks, failed)| {
                    (
                        name.as_str(),
                        Value::object([
                            ("failed", Value::count(*failed)),
                            ("ticks", Value::count(*ticks)),
                        ]),
                    )
                })),
            ),
            (
                "shifts",
                Value::object([
                    ("abandoned", Value::count(self.shifts.2)),
                    (
                        "cost_per_shift",
                        self.shifts
                            .4
                            .map_or(Value::Null, |c| Value::Number(cents(c))),
                    ),
                    ("handed_off", Value::count(self.shifts.1)),
                    (
                        "mean_minutes",
                        self.shifts.3.map_or(Value::Null, |m| {
                            Value::count(usize::try_from(m).unwrap_or_default())
                        }),
                    ),
                    ("opened", Value::count(self.shifts.0)),
                ]),
            ),
            (
                "threads",
                Value::object([
                    ("by_kind", tally(&self.threads)),
                    ("from_shifts", Value::count(self.threads_from_shifts)),
                ]),
            ),
            ("to", Value::string(to)),
            (
                "trail",
                Value::object([
                    ("breath_states", tally(&self.breath_states)),
                    ("by_kind", tally(&self.kinds)),
                    ("by_model", tally(&self.models)),
                    ("by_repo", tally(&self.repos)),
                    ("changes_with_sha", Value::count(self.changes_with_sha)),
                    ("cost_unrecorded", Value::count(self.cost_unrecorded)),
                    ("cost_usd", Value::Number(cents(self.cost_usd))),
                    ("events", Value::count(self.events)),
                ]),
            ),
            ("week", Value::string(week.name())),
            (
                "worst",
                Value::array(self.worst().into_iter().map(|miss| {
                    Value::object([
                        ("metric", Value::string(miss.metric)),
                        ("target", Value::string(miss.target)),
                        ("value", Value::string(miss.value)),
                    ])
                })),
            ),
        ])
        .render()
    }

    /// The score page: frontmatter, the worst three, then one table per section.
    #[must_use]
    pub fn page(&self, written: Timestamp) -> String {
        let week = self.week;
        let (from, to) = week.bounds();
        let mut out = format!(
            "---\nweek: {}\nwritten: {}\nfrom: {from}\nto: {to}\n---\n",
            week.name(),
            written.iso8601()
        );
        let mut worst: Vec<Vec<String>> = self
            .worst()
            .into_iter()
            .map(|miss| vec![miss.metric.to_owned(), miss.value, miss.target])
            .collect();
        if worst.is_empty() {
            worst.push(vec!["none".to_owned(), "-".to_owned(), "-".to_owned()]);
        }
        table(
            &mut out,
            "Worst three",
            &["metric", "value", "target"],
            worst,
        );
        let covered = self.coverage.iter().filter(|(_, n)| *n > 0).count();
        let mut coverage: Vec<Vec<String>> = self
            .coverage
            .iter()
            .map(|(repo, n)| vec![repo.clone(), n.to_string()])
            .collect();
        coverage.push(vec![
            "covered".to_owned(),
            format!("{covered}/{}", self.coverage.len()),
        ]);
        table(&mut out, "Coverage", &["repo", "shifts"], coverage);
        let (opened, handed, abandoned, mean, cost) = self.shifts;
        table(
            &mut out,
            "Shifts",
            &[
                "opened",
                "handed off",
                "abandoned",
                "mean minutes",
                "cost per shift",
            ],
            vec![vec![
                opened.to_string(),
                handed.to_string(),
                abandoned.to_string(),
                mean.map_or_else(|| "-".to_owned(), |m| m.to_string()),
                cost.map_or_else(|| "-".to_owned(), |c| format!("{c:.2}")),
            ]],
        );
        let rows = |map: &Tally| {
            map.iter()
                .map(|(k, v)| vec![k.clone(), v.to_string()])
                .collect()
        };
        table(&mut out, "Trail", &["kind", "events"], rows(&self.kinds));
        table(
            &mut out,
            "",
            &["events", "changes with sha", "cost usd", "cost unrecorded"],
            vec![vec![
                self.events.to_string(),
                self.changes_with_sha.to_string(),
                format!("{:.2}", self.cost_usd),
                self.cost_unrecorded.to_string(),
            ]],
        );
        table(&mut out, "", &["repo", "events"], rows(&self.repos));
        table(&mut out, "", &["model", "events"], rows(&self.models));
        table(
            &mut out,
            "",
            &["breath state", "events"],
            rows(&self.breath_states),
        );
        table(
            &mut out,
            "Corrections",
            &["corrections", "recurring"],
            vec![vec![
                self.corrections.0.to_string(),
                self.corrections.1.to_string(),
            ]],
        );
        let (slots, fresh) = self.context;
        table(
            &mut out,
            "Context",
            &["slots", "fresh", "fresh ratio"],
            vec![vec![
                slots.to_string(),
                fresh.to_string(),
                format!("{:.2}", self.fresh_ratio()),
            ]],
        );
        let [filed, spiked, adopted, dropped] = self.breath;
        table(
            &mut out,
            "Breath",
            &["filed", "spiked", "adopted", "dropped"],
            vec![
                [filed, spiked, adopted, dropped]
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
            ],
        );
        table(
            &mut out,
            "",
            &["source", "weight", "filed", "adopted"],
            self.breath_sources
                .iter()
                .map(|(name, weight, filed, adopted)| {
                    vec![
                        name.clone(),
                        format!("{weight:.2}"),
                        filed.to_string(),
                        adopted.to_string(),
                    ]
                })
                .collect(),
        );
        let mut threads: Vec<Vec<String>> = rows(&self.threads);
        threads.push(vec![
            "from shifts".to_owned(),
            self.threads_from_shifts.to_string(),
        ]);
        table(&mut out, "Threads", &["kind", "open"], threads);
        table(
            &mut out,
            "Routines",
            &["routine", "ticks", "non-zero exits"],
            self.routines
                .iter()
                .map(|(name, ticks, failed)| {
                    vec![name.clone(), ticks.to_string(), failed.to_string()]
                })
                .collect(),
        );
        out
    }
}

/// A markdown table under `## heading` (none when empty), skipped when it has no rows.
fn table(out: &mut String, heading: &str, header: &[&str], rows: Vec<Vec<String>>) {
    if rows.is_empty() {
        return;
    }
    if !heading.is_empty() {
        let _ = write!(out, "\n## {heading}\n");
    }
    let _ = write!(
        out,
        "\n| {} |\n|{}\n",
        header.join(" | "),
        " --- |".repeat(header.len())
    );
    for row in rows {
        let _ = writeln!(out, "| {} |", row.join(" | "));
    }
}

/// Rewrite `score`'s page; its path.
pub fn write(ctx: &shift::Context, score: &Score) -> Result<PathBuf> {
    let week = score.week;
    let dir = paths::score_dir(&ctx.data_root);
    let path = dir.join(format!("{}.md", week.name()));
    let _lock = CommitLock::acquire(&ctx.data_root)?;
    fs::create_dir_all(&dir).map_err(|source| Error::io(&dir, source))?;
    atomic::write(&path, &score.page(ctx.now))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::Week;
    use crate::time::Timestamp;

    fn at(text: &str) -> Timestamp {
        Timestamp::parse_iso8601(text).expect("timestamp")
    }

    #[test]
    fn weeks_parse_step_and_bound_in_new_york() {
        let week = Week::parse("2026-W42").expect("week");
        assert_eq!(week.name(), "2026-W42");
        assert_eq!(
            week.bounds(),
            ("2026-10-12".to_owned(), "2026-10-18".to_owned())
        );
        assert_eq!(week.offset(-1).name(), "2026-W41");
        assert_eq!(
            Week::parse("2026-W53").expect("53").offset(1).name(),
            "2027-W01"
        );
        assert!(week.shemitah());
        // 03:30Z Monday is Sunday 23:30 EDT, still W42; an hour later it is not.
        assert!(week.contains(at("2026-10-19T03:30:00Z")));
        assert!(!week.contains(at("2026-10-19T04:30:00Z")));
        // A Sunday plans by the next day's week; a Saturday by its own.
        assert_eq!(Week::targets(at("2026-10-11T16:00:00Z")).name(), "2026-W42");
        assert_eq!(Week::targets(at("2026-10-10T16:00:00Z")).name(), "2026-W41");
        for bad in [
            "2026-W00", "2027-W53", "2026-42", "26-W42", "2026-W4x", "+026-W42",
        ] {
            assert_eq!(Week::parse(bad), None, "{bad}");
        }
    }
}
