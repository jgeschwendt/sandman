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
  `rationale`. Nothing is edited or deleted; revoking a revoked event appends nothing.
- **Reconcile** `<id>` takes the event and its descendants as the revoked set. In every
  repo it reaches — each checkout under `~/.grove/code/<owner>/<repo>/<branch>/` (repos
  from the grove manifest's `[roots."owner/repo"]` tables and from the tree itself), plus
  `~`, `~/.routines` and the data root — it selects the commits whose `Event:` trailer is
  in the set, one pass per repository however many worktrees it has. Those reachable
  from the checkout's `HEAD` are reverted newest-first onto `reconcile/<id>` (reused if
  it exists, already-reverted commits skipped), in a scratch worktree so no checkout is
  switched. Each revert commit carries `Shift: reconcile` and `Event: <the reconcile
  event's id>`. A conflicting revert is aborted and ends that repo's pass. History is
  never reset or forced.
- Reconcile then appends one `reconcile` event — `parent: <id>`, `shift: reconcile`,
  `refs` every reverted commit as `repo@sha`, `rationale` the count and every conflict —
  and lists the memories under `memories/` whose `source:` names a revoked event, for
  reflect to retire. It modifies no memory. `--dry-run` prints the plan and writes
  nothing.

`$SANDMAN_GROVE_CODE` moves the grove's code directory (the manifest is read beside it,
`<code>/../manifest.toml`, and may be absent); `$SANDMAN_RECONCILE_EXTRA`
(colon-separated) replaces the three extra repos.

## Concurrency

All bank and trail writes happen under `.commit.lock` at the data root; every file write is
atomic (tmp → rename in the same directory).
