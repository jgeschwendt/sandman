# Bank format — the on-disk contract

The format the commit path writes and every reader assumes. Each numeric claim below
was measured against the production banks — 232 memory files across 33 banks (measured
2026-08-12 · the `real_banks` round-trip suite, re-runnable with
`cargo test -- --ignored`).

## Bank key

One bank per working directory: the cwd with every non-alphanumeric character replaced
by `-` (same encoding Claude Code uses for `~/.claude/projects/`).
`/Users/you/.code/project` → `-Users-you--code-project`.
A linked git worktree (a `.git` file pointing into `worktrees/`) keys as its parent
directory, so every branch of one repo shares one bank:
`~/.grove/code/o/r/main` → `-Users-you--grove-code-o-r`.

## Memory file

`<type>_<slug>.md`, where slug = `name` lowercased, every non-alphanumeric run → `_`,
trimmed of leading/trailing `_`, truncated at 60 **characters** — characters, not bytes,
here and in the index truncation below; byte truncation would split UTF-8 mid-sequence
(48 of 232 descriptions carry non-ASCII inside the window). 39 of 232 files sit exactly
at 60; the rule reproduced all 232 filenames with zero violations (measured 2026-08-12 ·
M1 round-trip suite).

```
---
name: <freeform — often a slug, sometimes prose>
description: <one line>
type: user | feedback | project | reference
created: <ISO-8601 Z>          (183/232)
source: <session/provenance>   (219/232)
updated: <ISO-8601 Z>          (183/232)
---

<markdown body>
```

- Frontmatter values are single-line — zero multiline values in 232 files.
- `name`, `description`, `type` are always present; the rest vary. Unknown keys occur
  (one file carries `recall:`) — **parse must preserve unknown keys and key order**;
  round-trip is byte-identical.
- A blank line always separates the closing `---` from the body (0 violations).
- Collision suffixes: `_2`, `_3`, … append to the slug when the target filename
  exists and is not the file being replaced.

## MEMORY.md — the bank index

Regenerated, never hand-edited. Fixed frontmatter, then one line per memory file,
sorted by filename:

```
---
name: MEMORY index
description: One-line map of all durable memories in this knowledge bank
type: reference
---

- [<name>](<filename>) — <description, truncated at 150 chars>
```

(150 measured: max entry description is exactly 150 characters, hard cut, no ellipsis.)
Descriptions are raw text end to end — no quoting layer ever interprets them.

## _archive/ — supersession lineage

A replaced file moves (never copies) to `_archive/<YYYYMMDDTHHMMSS>_<filename>` —
UTC timestamp prefix, archived content verbatim as it stood. Nothing in `_archive/`
is ever deleted.

## _reflect.json

Per-bank due-baseline: `{"at": <ISO>, "count": <int>, "last_ops": <int>}` — written
only by reflect, seeded on first sight of a bank.

## log/ — the voyage log

Not a bank. `log/` is memory's derivative: one entry per UTC day, written by reflect from
the memories that landed that day, and re-derivable from them at any time.

`log/<yyyy-mm-dd>.md` — single-line frontmatter, keys alphabetical, a blank line, then
the body:

```
---
date: 2026-08-31
day: 19
fingerprint: 9f2c…
kind: setback
mind: claude-opus-5
position: day 19 · 3 sessions · 2 memories landed · 254 memories in 32 banks
sources: -Users-jlg/feedback_shared_trunk_write_discipline.md, -Users-jlg--grove-code-jgeschwendt-bridge--trunk/feedback_bridge_xterm_styling_rules.md
title: Two writers, one trunk
written: 2026-09-01T03:30:12Z
---

A subagent undid its own edit with a tree discard and took another feature's uncommitted
scene down with it. I have kept the rule — undo by re-editing, never by discarding the
tree — but the rule is the cheap half: the trunk has two writers and no lock, and day 12
already noticed the shape of this once.

Next: the scene is rebuilt from its plate by hand.
```

| key | what |
| --- | --- |
| `date` | the UTC day the entry covers — the same day the filename names |
| `day` | which day of the voyage this is, counting `began:` as day 1 |
| `fingerprint` | 16 hex characters hashed over the day's sources, names and bodies both — what idempotence rests on |
| `kind` | the entry's spine: `discovery` · `milestone` · `reflection` · `setback` |
| `mind` | the model that wrote it |
| `position` | stamped, never recomputed: sessions taken that day, memories landed that day, live memories, banks |
| `sources` | bank-relative `<bank>/<file>`, comma-separated, sorted — the derivative's provenance, carried on the derivative |
| `title` | 2–6 words, the way a chapter is named |
| `written` | when the entry was written, ISO-8601 Z |

The four kinds are the four things the reference logs mark: `discovery` — something new
exists or was named; `milestone` — something shipped, was adopted, or was retired for
good; `reflection` — the day stepped back and changed how the work is done; `setback` —
something was lost, broke, or went wrong, and what it cost.

The body is the entry: 2–4 sentences of prose. A final line beginning `Next:` is the
optional hand-off to the day after.

### INDEX.md

Regenerated behind every entry write. Fixed frontmatter — `began:` is the voyage's day 1,
set once on the first entry ever written and preserved by every regeneration after it —
then one line per entry, ascending:

```
---
name: voyage log
description: What this memory engine lived through, one entry per day the banks moved — newest last
began: 2026-08-13
type: reference
---

- day 19 · [Two writers, one trunk](2026-08-31.md) — setback · 2026-08-31
```

### The rules

- **Idempotent for the day.** The entry is a function of the day's sources. Reflect
  rewrites `<date>.md` only when the `fingerprint` on disk no longer matches the one the
  day's sources hash to — same sources, same entry, no model call.
- **Never hand-edited.** Pipeline-owned like `memories/`: an edit is lost on the next pass
  that finds the sources changed. A day the operator wants marked is marked by what
  `remember` puts in the banks that day.
- **A quiet day gets no file.** No sources, no entry — the index shows the gap. A filler
  entry would be prose about nothing, and every reader of the log pays for it in context.

## trail/ — the event log

Not a bank. `trail/` is the machine's ledger: every autonomous decision is one event,
appended and never edited, and every commit the machine makes names its event by
trailer, so `git log` in any repo joins back to it.

`trail/<yyyy>/<mm>/<dd>.jsonl` — one file per UTC day, one JSON object per line, keys
alphabetical, newest last:

```
{"at":"2026-10-09T23:41:02Z","branch":"perpetual","cost_usd":0.12,"id":"e-20261009T234102Z-7f3a","kind":"change","model":"opus","parent":null,"rationale":"…","refs":[],"repo":"jgeschwendt/sandman","sha":"9b32c84…","shift":"s-20261009T2300Z"}
```

| key | what |
| --- | --- |
| `at` | when the event was appended, ISO-8601 Z; the file is the UTC day it falls on |
| `branch` | the branch the work landed on |
| `cost_usd` | what the work cost, US dollars, a non-negative number |
| `id` | `e-<yyyymmddThhmmss>Z-<4 hex>` — the instant plus four random hex digits, unique within the day's file and so across the trail |
| `kind` | what happened — one of the kinds below |
| `model` | the model that did the work |
| `parent` | the id of the event this one descends from, or `null`; must already be on the trail |
| `rationale` | why, in prose |
| `refs` | free strings — thread ids, URLs, memory files, `repo@sha` |
| `repo` | the repo touched, `owner/repo` |
| `sha` | the commit the work produced |
| `shift` | the shift the event belongs to |

`at`, `id`, `kind`, `parent` and `refs` are always present (`refs` may be `[]`); every
other key is omitted when it has no value, never `null`.

Kinds: `breath` · `change` · `correction` · `decay` · `guard` · `promote` · `reconcile`
· `revoke` · `shift.handoff` · `shift.start`. Any other kind is refused.

### The parent chain

An event's descendants are every event whose `parent` chain reaches it — a `change`
made because of a `promote`, a `guard` made because of a `correction`. `revoke` and
`reconcile` events name their target as `parent` too, but they speak *about* it rather
than descend from it: the walk never enters them, so a second reconcile never reverts
the first one's reverts.

### Trailers

Every commit the machine makes carries two git trailers:

```
Shift: <shift-id>
Event: <event-id>
```

`sandman trail trailers --shift S --event E` prints exactly those lines, for appending
to a message or for `git commit --trailer`.

### Revoke and reconcile

- **Revoke** appends a `revoke` event with `parent: <id>` and the reason as its
  `rationale`. Nothing is edited or deleted; revoking a revoked event appends nothing —
  the check and the append share one hold of `.commit.lock`, so racing revokes write
  one event.
- **A shift id revokes the whole shift.** `revoke` and `reconcile` take an `s-…` id as
  readily as an `e-…` one: the revoked set is every event whose `shift` is that id
  (`revoke` and `reconcile` events aside) plus all their descendants, and a commit is a
  hit when its `Event:` is in the set *or* its `Shift:` is the id — so a commit whose
  event never reached the trail is still caught. `revoke s-…` appends one `revoke`
  event with `parent: null` and `refs: ["shift:<id>"]`; the `reconcile` event it leads
  to carries the same ref and no parent, on `reconcile/<shift-id>`.
- **Reconcile** `<id>` takes the event and its descendants as the revoked set. In every
  repo it reaches — each checkout under `~/.grove/code/<owner>/<repo>/<branch>/` (repos
  from the grove manifest's `[roots."owner/repo"]` tables and from the tree itself), plus
  `~`, `~/.routines` and the data root — it selects the commits whose `Event:` trailer is
  in the set (or, for a shift id, whose `Shift:` names it), one pass per repository
  however many worktrees it has.
- The base is the repo's **trunk**: the manifest root's `trunk = "…"`, else `main`, else
  the branch `refs/remotes/origin/HEAD` names, else `HEAD` with a note in the report and
  the journal — never the `HEAD` of whichever checkout reaches the hits, since a
  `shift/…` checkout would carry the shift's other commits onto the reconcile branch.
  A hit trunk does not reach is reported `off trunk` and left alone: its branch is
  simply never merged.
- The hits trunk reaches are reverted newest-first onto `reconcile/<id>` — cut from
  trunk, or reused if it exists and still contains trunk (a reconcile branch trunk has
  moved past fails that repo: merge trunk in or delete the branch, then rerun);
  already-reverted commits are skipped. A merge commit is reverted with `-m 1`, against
  the line it was merged into, and the hits it brought in are reported `in reverted
  merge` rather than reverted twice. The reverts happen in a scratch worktree, so no
  checkout is switched, and it is removed with `--force` however the pass ends. The
  repo's commit hooks are switched off (`core.hooksPath=/dev/null`, `--no-verify`): a
  revert is the owner's reconcile, not a shift's work, so a repo's hook suite neither
  blocks nor slows it. Each revert commit carries `Shift: reconcile` and `Event: <the
  reconcile event's id>`. A conflicting revert is aborted and ends that repo's pass.
  History is never reset or forced.
- Reconcile then appends one `reconcile` event — `parent: <id>`, `shift: reconcile`,
  `refs` every reverted commit as `repo@sha`, `rationale` the count and every conflict —
  and lists the memories under `memories/` whose `source:` names a revoked event, for
  reflect to retire. It modifies no memory. Every repo is worked before the verdict:
  reconcile exits 1 when any repo ended in a conflict or a failure, after the report has
  listed them all. `--dry-run` prints the plan and writes nothing.

`$SANDMAN_GROVE_CODE` moves the grove's code directory (the manifest is read beside it,
`<code>/../manifest.toml`, and may be absent); `$SANDMAN_RECONCILE_EXTRA`
(colon-separated) replaces the three extra repos.

## shifts/ — the shift engine

Not a bank. A **shift** is one autonomous session bounded by the five-hour usage
window; `sandman shift` opens it with a brief and closes it with a handoff. Its id is
`s-<yyyymmddThhmm>Z`, the UTC minute `shift brief` ran, and every machine commit it
makes carries that id in its `Shift:` trailer.

```
shifts/
  current.json            # the open shift; absent between shifts
  PAUSE                   # the owner's off switch — the runner starts nothing while it exists
  s-20261016T1600Z/
    brief.md              # the composed prompt the session started from
    handoff.md            # what the shift left the next one
    meta.json
```

`current.json` — one object, keys alphabetical, written by `shift brief`, extended by
`shift attach`, removed by `shift handoff` for the shift it names:

| key | what |
| --- | --- |
| `bg_id` | the background session's short id, once the runner has attached it |
| `handoff_by` | when the shift must hand off — fifteen minutes before the window resets, or before five hours pass when the window is unknown |
| `path` | the checkout the shift works |
| `repo` | `owner/repo` |
| `session_id` | the session's full id, once attached |
| `shift` | the shift id |
| `started` | when the brief was composed, ISO-8601 Z |
| `theme` | the day's theme |

`meta.json` — keys alphabetical: `shift`, `theme`, `instrument`, `repo`, `path`,
`started`, `mind` (the orchestrator model), `window` (`remaining` as prose, `resets_at`,
`stale`, `used` — the clock at the start); `shift handoff` adds `ended`, and `abandoned:
true` when the runner found the session gone. The rotation reads `repo` and `started`
from every `meta.json` to know which repo has rested longest.

### The rotation

`shift pick` covers the trunk checkout of every grove repo whose owner is in
`$SANDMAN_SHIFT_OWNERS` (colon-separated, default `jgeschwendt`) — the trunk is the
checkout named by the manifest root's `trunk = "…"`, else `main` — and nothing else:
only checkouts under `$SANDMAN_GROVE_CODE/<owner>/` are candidates. The home repos
(`~`, `~/.routines`, the data root) are never picked, though reconcile still scans
them: home-sync commits them every fifteen minutes without trailers, so a shift's work
there could not be told from the sync's or revoked cleanly, and a shift working
`~/.sandman` or `~/.routines` could edit the trail it answers to or the live runner
that supervises it. Least recently shifted first, never-shifted before any; a tie goes
to a failing `stele check` (run only where `.stele/graph.lock` exists, 20 s timeout, a
missing binary skipped), then the most open threads naming the slug, then the most
untried breaths (`state: new` or `spiked`) filed against it, then the name. The open shift's repo and
`$SANDMAN_SHIFT_SKIP` (colon-separated slugs) are passed over.

### The brief and its slots

`shift brief` fills `prompts/shift.md` — `$SANDMAN_PROMPTS`, else the `prompts/` of
sandman's trunk checkout in the grove — and refuses (exit 1) while `current.json` names
an open shift, when the template is missing, or when it names a slot outside this
table. The theme is the America/New_York weekday's (the US daylight-time rule,
computed); `$SANDMAN_NOW` pins the instant.

| slot | filled with |
| --- | --- |
| `brief.at` | the instant, ISO-8601 Z |
| `breath.pending` | `breath/*.md` with `repo:` the target and `state: new` or `spiked` (marked), one line each |
| `budget.spent` | `cost_usd` summed over today's UTC trail events whose `shift` is a shift id |
| `budget.usd` | `$SANDMAN_SHIFT_BUDGET_USD`, default 15 |
| `clock.handoff_by` | as in `current.json` |
| `clock.remaining` | the window's remaining time, `fresh` once it has reset, `unknown` without a window file |
| `handoff.previous` | the newest `shifts/*/handoff.md`, else `none` |
| `memories.bearing` | the target's bank chain as recall renders it, under 3,000 characters |
| `score.week` | the first 20 lines of `score/<yyyy>-W<ww>.md`, else `none yet` |
| `shift.creation` | the creation-day note — `Day 6 · Image; the work finished, very good` |
| `shift.id` | the shift id |
| `shift.instrument` | the day's instrument |
| `shift.mind` | `--mind`, else `$SANDMAN_SHIFT_MIND`, else `opus` |
| `shift.start_event` | the id of this shift's `shift.start` event — the `--parent` of every `trail append` the shift makes |
| `shift.theme` | Hunt · Redesign · Refactor · Measure · Harden · Ship · Rest, Sunday first |
| `shift.theme_notes` | the day's work; on Saturday "No code commits; remember, don't make.", and in an ISO week divisible by seven the Shemitah's ablation instead of the self-pass |
| `shift.weekday` | the local weekday's name |
| `target.last_shifted` | the target's newest `started`, else `never` |
| `target.path` | the target checkout |
| `target.plan_next` | the first paragraph under the first `docs/plan.md` heading that mentions "next", else its `Status:` line, cut at 600 characters |
| `target.reason` | why the rotation chose it |
| `target.repo` | the target's slug |
| `threads.open` | `~/.threads/*.md` with `state: open` whose text names the slug, one line each |
| `trail.recent` | the trail's summary lines from the 24 h before the instant, the newest 40 |

The composed brief is capped at 24,000 characters. Over the cap, whole lines go —
`trail.recent` first (oldest lines first), then `memories.bearing`, then `threads.open`
(from the tail) — and a marker line says how many; a brief still over is cut at the
cap.

### Attribution

`shift brief` appends one `shift.start` event — `shift` the id, `repo` the target,
`model` the mind, `rationale` the theme and the rotation's reason. Its id is minted
before the text is composed, so the brief can name it as `{{shift.start_event}}`, and the
event is appended under that id (never re-minted) once the brief is written — a brief
that fails to compose leaves nothing on the trail. Its `refs` are:

- `brief:<path>` — the brief it wrote;
- `fp:<slot>=<8 hex>` — one per slot the template names, the 32-bit FNV-1a of the text
  that slot received (after any trim): the record of what the shift was primed with,
  for Wednesday's score to attribute outcomes to;
- `trimmed:<slot>` — one per slot the cap cut, `trimmed:brief` for a final cut.

`shift handoff` appends one `shift.handoff` event — `rationale` the handoff's first line,
`refs` `handoff:<path>` and `abandoned` when it was.

### breath/problems.md

The ruach's reading list, kept by `shift brief`: one `## <repo> · <yyyy-mm-dd>` section
per repo per local day — the target's plan-next, its open threads, and the newest five
`guard`/`correction` events touching it — replaced in place when the same heading is
briefed again.

## breath/ — the ruach

Not a bank. The ruach is the research engine beside the shifts: a Claude session reads
the feeds against the open problems and files what bears on a repo as a **breath**.
`sandman breathe` is the store and the prompt composer; it never calls a model or the
network.

```
breath/
  sources.toml                    # the feeds and their learned state
  problems.md                     # written by `shift brief` (§ shifts); read-only here
  <yyyy-mm-dd>-<slug>.md          # one breath
  ledger/                         # /gigaresearch's claim ledger
  passes/<pass-id>/pass.json      # one record per pass
  passes/<pass-id>/prompt.md      # the prompt the pass's session got
```

`breathe init` creates `breath/`, `ledger/` and `passes/` and copies the seed — the
repo's `breath/sources.toml`, compiled into the binary — when `sources.toml` is absent.

### sources.toml

Flat TOML: an optional root key `reweighted` (ISO-8601 Z, when `reweight` last ran),
then one `[name]` table per feed, a bare key. Values are strings, integers, floats and
one-line string arrays; anything else — a dotted or nested table, an inline table, a
boolean, a date, a multi-line value, an unknown key — is refused with its line.

| key | what |
| --- | --- |
| `kind` | `atom` · `rss` · `json` · `html` — how a reader parses the feed |
| `url` | the feed |
| `weight` | learned, ≥ 0; `reweight` moves it |
| `budget` | items a pass reads from it at most |
| `bears_on` | repo slugs (`owner/name`) or `"*"` |
| `cursor` | where the last pass stopped, in the feed's own terms — written by `close` |
| `last_read` | when a pass last read it, ISO-8601 Z — written by `close` |
| `filed` | breaths filed from it — counted by `file` |
| `adopted` | breaths from it adopted — counted by `adopt` |
| `silent_since` | since when it has gone without an adoption — set by `close` and `drop` when absent, restarted by `adopt` and by a silence penalty |

The first five are required. Any rewrite — `file --source`, `adopt`, `drop`, `close`,
`reweight` — sorts the tables by name, writes the keys in the order above with the
state keys only when set, and drops comments.

A source is **due** for a skim when its `last_read` is absent or at least an hour old;
a deep pass reads every source.

### A breath

`<yyyy-mm-dd>-<slug>.md` — the UTC date `file` ran, the title slugged with dashes (cut
at 60 characters, `breath` when nothing is left), `-2`, `-3`, … on a collision.
Frontmatter in this order, one line per key, newlines in a value folded to spaces:

| key | what |
| --- | --- |
| `title` | the idea |
| `url` | its primary source |
| `source` | the `sources.toml` table it came from, when named |
| `repo` | the repo it bears on, `owner/name` or `*` |
| `problem` | the open problem it speaks to |
| `state` | `new` · `spiked` · `adopted` · `dropped` |
| `bearing` | 0–1, the judge's "does it bear on the repo", when given |
| `novelty` | 0–1, the judge's "is it new to us", when given |
| `filed` | when it was filed, ISO-8601 Z |
| `updated` | when it last changed |
| `seen_again` | later filings that matched it |
| `reason` | why it was dropped — `dropped` only |

The body is the four-line summary.

**Dedupe.** `file` matches a new filing against every breath in the store — the same
URL (one trailing `/` aside), else a title whose token Jaccard (lowercase alphanumeric
runs) is at least 0.6. A match bumps that breath's `seen_again` and `updated`, prints
`seen_again <path>` and appends nothing to the trail.

**Transitions.** `spike` takes `new`; `adopt` takes `new` or `spiked`; `drop` takes
anything not yet dropped and requires `--reason`. Each stamps `updated`.

### passes/

`breathe pass --kind skim|deep` composes the pass prompt and writes
`passes/<id>/pass.json`, keys alphabetical:

| key | what |
| --- | --- |
| `bg_id` | the background session's short id, once `attach` records it |
| `cost_usd` | what the pass cost, US dollars — `null` until `close` |
| `dropped` · `filed` · `read` | the pass's counters — `null` until `close` |
| `ended` | when `close` ran — `null` until then |
| `id` | `b-<yyyymmddThhmmss>Z-<kind>`, the UTC instant `pass` ran |
| `kind` | `skim` or `deep` |
| `session_id` | the session's full id, once attached |
| `sources` | the due sources the prompt listed, by name |
| `started` | when the prompt was composed |

`pass` refuses (exit 1) when no source is due or when today's passes — those whose
`started` falls on the UTC day — have spent `$SANDMAN_BREATH_BUDGET_USD` (default 10);
`--dry-run` prints the prompt and writes nothing. `close` refuses a pass already closed
and a `--cursor` for a source the pass did not list.

### The pass prompt and its slots

`pass` fills `prompts/breathe.md` from the same directory as `shift.md` and refuses a
template naming a slot outside this table:

| slot | filled with |
| --- | --- |
| `budget.spent` | `cost_usd` summed over today's passes, two decimals |
| `budget.usd` | `$SANDMAN_BREATH_BUDGET_USD`, default 10 |
| `pass.cadence` | `hourly skim` or `daily deep` |
| `pass.id` | the pass id |
| `pass.kind` | `skim` or `deep` |
| `problems` | `breath/problems.md` verbatim, cut at 6,000 characters, else `none` |
| `sources.due` | one line per due source — name, feed, cursor (`none` before the first), budget |

### Reweight

Per source: `+0.1` for each breath from it in state `adopted` whose `updated` is after
`reweighted` (every one, the first time), capped at 2.0; `−0.1` when `silent_since` is
over thirty days old, floored at 0.2, and the silence clock restarts. Weights keep two
decimals. `reweight` prints `name  old → new  why` for each source it touched, stamps
`reweighted`, rewrites the file and appends one event; `--dry-run` prints the table only.

### Trail events

Every event is kind `breath`, appended after the store's write, with `shift` from
`$SANDMAN_SHIFT` when it is set:

| verb | `repo` | `rationale` | `refs` |
| --- | --- | --- | --- |
| `file` | the breath's | `filed: <title>` | `url:<url>`, `breath:<file>` |
| `adopt` · `drop` | the breath's | `adopted: <title>` · `dropped: <title>`, then ` — <reason>` when one was given | `breath:<file>`, `state:<new state>`, and `adopted-by:<shift>` on an adopt in a shift |
| `close` | — | `pass <kind>: read N filed M dropped K` | `pass:<id>`; `cost_usd` the pass's cost |
| `reweight` | — | `reweight: N source(s) moved` | `weight:<name>=<old>→<new>` per source touched |

`spike` and a `seen_again` append none. `breathe` journals to `.trace/breathe-<date>.log`.

## Concurrency

All bank, trail, shift and breath writes happen under `.commit.lock` at the data root; every file write is
atomic (tmp → rename in the same directory).
