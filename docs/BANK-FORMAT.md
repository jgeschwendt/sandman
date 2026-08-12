# Bank format — the on-disk contract

The format the commit path writes and every reader assumes. Each numeric claim below
was measured against the production banks — 232 memory files across 33 banks (measured
2026-08-12 · the `real_banks` round-trip suite, re-runnable with
`cargo test -- --ignored`).

## Bank key

One bank per working directory: the cwd with every non-alphanumeric character replaced
by `-` (same encoding Claude Code uses for `~/.claude/projects/`).
`/Users/you/.code/project` → `-Users-you--code-project`.

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

## Concurrency

All bank writes happen under `.commit.lock` at the data root; every file write is
atomic (tmp → rename in the same directory).
