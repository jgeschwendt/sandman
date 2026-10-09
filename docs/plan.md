# The perpetual shift — plan

Status: **drafted 2026-10-09**, phase 0 in flight. Owner's brief, verbatim: "we're going to build a perpetual motion machine for .grove/code … the most elaborate recursive self learning dynamic prompt … i need a trail to follow if you take the wrong path, i want to let you grow by yourself, i will check the history and we will go back in time and fix the event and reconcile the fractured history, perpetual motion means never stopping, we have 5hr work chunks so we are going to work in that shift … i am not opposed to growing sandman, as long as all my repos under jgeschwendt continually all get better at the same time … we get outside energy from the world, there are brilliant ideas published each day, breathe them in and grow … setup a weekly schedule, monday refactor, tuesday bug hunt, wed… use your giga skills as needed."

The plate is `docs/plates/perpetual-shift.png`, drawn in Claude Design.

## Premise, pressure-tested

A prompt that rewrites itself has no free energy. Without an external fitness signal it converges on bloat or noise. The machine therefore has three non-negotiable parts: an **energy source** (session transcripts, the owner's corrections, outcomes such as green CI and resolved threads, and ideas published outside), a **sink** (hard token budgets per context tier and a cost cap per shift), and a **trail** (every autonomous decision is an event the owner can revoke and the machine can reconcile). Everything else is plumbing around those three.

Sandman grows into the machine. It already owns the loop's skeleton — take, dream, recall, reflect — and `~/.sandman` is the git-synced home where a trail belongs. Stele stays the per-repo knowledge store; routines stay the clock; threads stay the human valve; spawn and bridge stay the session launchers.

## Vocabulary

- **Shift** — one autonomous session bounded by the five-hour usage window. Shifts run back to back; the machine never stops. A shift opens with a *brief*, closes with a *handoff*.
- **Trail** — the append-only event log at `~/.sandman/trail/<yyyy>/<mm>/<dd>.jsonl`. Every machine commit in every repo carries `Shift:` and `Event:` git trailers, so `git log` anywhere joins back to the trail.
- **Reconcile** — the owner marks an event revoked; the machine reverts every commit downstream of it (new commits, never history rewrites) and re-derives memories from the events still standing.
- **Breath** — an idea taken in from outside, filed with provenance under `~/.sandman/breath/`, with a state: new, spiked, adopted, dropped.
- **Titer** — the evidence count behind a lesson. Probation needs two sessions of evidence to become active; an active lesson that is heeded decays toward an index line; one that keeps being violated escalates to a mechanical guard.

## The weekly rhythm

Local time (America/New_York). Four to five shifts a day; the day's theme is the shift's instrument, the repo rotation is its target.

| day | theme | instrument | what a shift does |
| --- | --- | --- | --- |
| Mon | Refactor | `/gigarefine` | drive one repo's worst-scored module to its best form, behaviour preserved |
| Tue | Hunt | `/gigasweep` → `/gigadebug` | sweep a repo for real defects, root-cause the confirmed ones, ship the repro as a regression test |
| Wed | Breathe | `/gigaresearch` | the deep intake: the week's publications against every repo's open problems; file breaths, spike one |
| Thu | Redesign | `/gigaredesign` → `/execute-plan` | rebuild the structure Mon and Tue flagged, always-green staged migration |
| Fri | Ship | `/gigareview` | GO/NO-GO on the week's branches, merge the GOs, patch releases, `stele check` across the grove, CI green |
| Sat | Tend | `/tighten`, dream-pass, ablation | docs, stele claims, prompt density, memory consolidation, home inventory |
| Sun | Reflect | the self-pass | score the week from the trail, rewrite the machine's own prompts and rubrics, plan next week's targets, write the voyage entry |

Every shift, regardless of day, opens with a fifteen-minute breath: the overnight feeds against the repo it is about to touch.

## Repo rotation

Scope is every root under `~/.grove/code/jgeschwendt/` plus the three home repos (`~`, `~/.routines`, `~/.sandman`). `sandman shift pick` orders repos by least-recently-shifted, then weights by live signals: CI red, stale `graph.lock`, open findings, open threads, breaths filed against it. Invariant: every repo is shifted at least once a week; the Sunday score reports coverage and a miss is a finding.

## Standing rails

- A shift works in `shift/<id>` worktrees created through `grove tree add`; trunk never carries in-flight work.
- Merge to trunk only when the repo's test, lint and `stele check && stele emit --check` are green and a `/gigareview` says GO. Patch releases ship on their own; a minor or major opens a decision thread (release policy, unchanged).
- History is never rewritten and nothing is force-pushed; the trail reconciles by revert.
- The orchestrator model is a knob, `SANDMAN_SHIFT_MIND`, default `opus`. Fable takes the Sunday self-pass and the Friday verdicts while quota allows; when the weekly limit is exhausted every spawned agent pins opus or below. Workers are always opus or below.
- Cost: every headless call logs its cost to the trail; a shift stops at `SANDMAN_SHIFT_BUDGET_USD` and hands off early. `claude-breaker` keeps its cap of twenty.
- Three consecutive failed blocks end the shift and open a problem thread. Threads are for what the trail cannot undo: publishing, deleting data, spending money, touching other people.

## Phases

Chiastic: the trail is the core and is built first; each outer layer is rebuilt against what the core required. Opus agents implement; the session model decomposes, gates and reviews.

### 0 · Housekeeping (this branch)

Strip the retired drawing tools: `design/*.tldr`, `docs/arch.tldr`, the `plan` task in `mise.toml`, the `design/` row in `AGENTS.md`, the `.stele/config.toml` comment. `docs/PLAN.md` becomes this file. `stele build`, lock committed.

### 1 · Trail

- `sandman trail append|log|show|revoke` — std-only Rust, one JSONL file per UTC day, written under `.commit.lock`.
- Event: `{id, at, shift, kind, repo, branch, sha, rationale, cost_usd, model, refs}`. Kinds: `shift.start`, `shift.handoff`, `change`, `promote`, `decay`, `breath`, `guard`, `correction`, `revoke`, `reconcile`.
- Trailers: every commit the machine makes carries `Shift: <shift-id>` and `Event: <event-id>`.
- `sandman reconcile <event-id>` — finds commits carrying the event or any event descended from it across every root, reverts them newest-first on a `reconcile/<event-id>` branch per repo, appends a `reconcile` event, and lists the memories whose `source:` names a revoked event for reflect to retire.
- `docs/BANK-FORMAT.md` § trail is the contract. Tests drive the binary against a temp `$SANDMAN_ROOT` with fixture repos.

### 2 · Shift

- `sandman shift brief` composes the shift prompt — the dynamic prompt itself — from: the charter, the weekday theme, the previous handoff, the trail's last day, `shift pick`'s target, the bearing memories for that repo, its `docs/plan.md` "next", open threads, and breaths filed against it. Budgeted; every included item's fingerprint is logged as part of the `shift.start` event so the Sunday score can attribute.
- `sandman shift handoff` writes `~/.sandman/shifts/<id>/handoff.md` and the `shift.handoff` event.
- `sandman shift clock` reads `~/.local/state/claude/window.json` — `resets_at` and `used_percentage` persisted by the harness statusline when a session is live, and by the shift runner when a limit error names the reset — and answers how long the window has left.
- The prompt template lives in the repo at `prompts/shift.md` and is a stele node; the Sunday self-pass edits it.
- `~/.routines/shift.md` is the runner: every five minutes, if no shift holds the lock and the window has room, `spawn claude start` with the brief, the pinned model, `--permission-mode=auto`, cwd at the picked worktree; waits; handoff on exit. The schedule table above is the routine's prose.
- First live shift runs under this phase's acceptance.

### 3 · Mod

- A Claude Code mod — a plugin whose hooks run inside Claude Code (docs: code.claude.com/docs/en/plugins/mods/overview; terminal v2.1.287+; hooks also run under `claude -p`, so every shift session carries it, though only interactive sessions draw) — ships in the harness repo as `plugins/perpetual/`.
- Hooks: `tool.call` guards the rules with mechanical form (`pkill -f`; `git checkout --`, `reset`, `stash`, `clean` as diagnostics; `.yml`; an Agent or Workflow call with no `model` pin) by refusing or holding the call, and appends a `guard` event to the trail; the prompt and turn hooks capture live corrections ("no", "don't", "never", a repeated instruction) as `correction` events — the energy source the distiller and the score feed on, recorded at the moment instead of mined from the archive later.
- Band above the prompt: shift id, weekday theme, target repo, window clock, last trail event. Commands: `/shift` (brief, clock, handoff) and `/trail` (today's events, revoke by id), answered at once without a Claude turn.
- Guards generated from promoted lessons (phase 6, Compile) land here: a lesson violated after two injections proposes its guard; the guard replaces the prose and frees budget.

### 4 · Breathe

- `~/.sandman/breath/sources.toml` — the feeds: arXiv cs.SE and cs.AI listings, Hacker News front page, GitHub trending for Rust, Elixir and TypeScript, and the release feeds of the authors the owner trusts (jdx, oxc, bun, Vercel, Elixir core, Anthropic). Each source carries a budget.
- `sandman breathe file|list|spike|adopt|drop` — the store; ideas are `breath/<date>-<slug>.md` with provenance (URL or arXiv id), the repo it bears on, and state.
- The daily fifteen-minute block and the Wednesday deep pass both run `/gigaresearch` with the store as the claim ledger's home.

### 5 · Score

- `sandman score [--week]` — metrics derived from the trail, the recall traces, threads, `runs.jsonl` and git: correction recurrence per lesson, bearing rate of injected context, promotion precision (promoted items never reverted or corrected), repo coverage, cost per shift and per merged change, shift outcomes.
- Written to `~/.sandman/score/<week>.md`; read by `shift brief` and by the Sunday self-pass.

### 6 · Compile

- `recall` becomes the compiler: candidates from the bank chain *and* the whole grove, ranked by bearing (dowse, merged in from `typesafe-integration`; Kev when no TypeSafe key), tiered budget, attribution logged to the trail.
- Reflect gains titer math: probation → active on two confirming sessions; active → index on thirty unused days; index → `_archive` on ninety; contradiction opens a thread.

### 7 · Recurse

- The machine's own prompts (`prompts/shift.md`, dream's rubric, the score's weights, the judge questions) are stele nodes with claims. The Sunday self-pass scores them, proposes edits on a `self/<week>` branch with `promote` events, merges on green, and writes the voyage entry.

## Not in this change

Cloud routines (ingress stays as it is); bridge pages for the trail and the score (bridge gets them in its own shift); replacing the voyage log (the log stays the human-facing narrative, the trail is the machine-facing ledger).

## Acceptance

Phase 1: a fixture repo receives two machine commits under two events; revoking the first reverts both, the trail shows the reconcile event, and the second event's memory is listed for retirement. Phase 2: one live shift opens with a brief, works a picked repo on a `shift/` branch, writes a handoff, and every commit it made resolves to the trail by trailer.
