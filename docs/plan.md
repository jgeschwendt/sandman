# The perpetual shift — plan

Status: **live since 2026-10-10 14:19Z.** Phases 0–5 and a minimal 7 built (trail, reconcile, shift, breathe, score, the Sabbath brief), trunk c1aecb3 installed, the runner and both breath runners installed, sessions in `dontAsk` inside the strict sandbox. First live tick (02:25Z) failed on workspace trust; first Sabbath s-20261010T1345Z (13:45–14:08Z) handed off cleanly — dream-pass 263→245 memories, W42 targets for 20 repos — and its four findings (sandbox must cover `~/.grove`, api.anthropic.com egress, tools through mise, one Sabbath a day) landed the same day. Deferred to the machine: phase 6 (dowse ranking, titer promote/decay), the mod's tool-call guards, the ruach's own Sabbath, the ablation prompt. Sunday opens the first Hunt. Every shift now ends with a `report.md` for the owner (`sandman report`, and a SessionStart hook that shows each once) and turns work larger than itself into a `proposal:` thread Monday executes once approved.

The plate is `docs/plates/perpetual-shift/` (Claude Design source); `mise run plates` renders it to `perpetual-shift.png` beside it, with the docs page's six.

## Premise, pressure-tested

A prompt that rewrites itself has no free energy. Without an external fitness signal it converges on bloat or noise. The machine therefore has three non-negotiable parts: an **energy source** (session transcripts, the owner's corrections, outcomes such as green CI and resolved threads, and ideas published outside), a **sink** (hard token budgets per context tier, a cost cap per shift and per day of research), and a **trail** (every autonomous decision is an event the owner can revoke and the machine can reconcile). Everything else is plumbing around those three.

Sandman grows into the machine. It already owned the loop's skeleton — take, dream, recall, reflect — and `~/.sandman` is the git-synced home the trail lives in. Stele stays the per-repo knowledge store; routines stay the clock; threads stay the human valve; spawn stays the session launcher.

## Vocabulary

- **Shift** — one autonomous session bounded by the five-hour usage window. Shifts run back to back; the machine never stops. A shift opens with a *brief*, closes with a *handoff*.
- **Trail** — the append-only event log at `~/.sandman/trail/<yyyy>/<mm>/<dd>.jsonl`. Every machine commit in every repo carries `Shift:` and `Event:` trailers, so `git log` anywhere joins back to the trail. Contract: `docs/BANK-FORMAT.md` § trail.
- **Reconcile** — the owner revokes an event; the machine reverts every commit carrying it or its descendants, in a scratch worktree on a `reconcile/<id>` branch per repo (new commits, never history rewrites), and lists the memories whose `source:` names a revoked event for reflect to retire.
- **Ruach** — the research engine: the breath over the waters, continuous, outside the days. It files **breaths** (ideas with provenance, a repo they bear on, a state: new, spiked, adopted, dropped) under `~/.sandman/breath/`.
- **Titer** — the evidence count behind a lesson. Probation needs two sessions of evidence to become active; an active lesson that is heeded decays toward an index line; one that keeps being violated escalates to a mechanical guard.
- **Sabbath** — the seventh day. The machine does not stop; it changes mode from making to remembering. **Shemitah** — every seventh week the Sabbath runs the ablation pass (rewrite the memory corpus from scratch) instead of the ordinary self-pass.

## The spine — the creation week

Hebrew reckoning: Sunday is Day 1, Saturday the Sabbath. Local time, America/New_York. The owner works by day and the machine by night: a night of weekday D runs D 21:00 → D+1 07:00 (`$SANDMAN_SHIFT_NIGHT`), and carries D's theme in every shift it holds. The day's theme is the shift's instrument, the repo rotation its target. The order is the point: find, restructure, polish, measure, multiply, finish, rest.

| day | creation | theme | instrument | what a shift does |
| --- | --- | --- | --- | --- |
| Sun | Day 1 · Light divided from darkness | **Hunt** | `/gigasweep` → `/gigadebug` | sweep a repo for real defects, root-cause the confirmed ones, ship the repro as a regression test |
| Mon | Day 2 · Firmament; waters divided | **Redesign** | `/gigaredesign` → `/execute-plan` | execute the approved proposals first, then rebuild the structure Sunday's findings and standing proposals indict, always-green staged migration |
| Tue | Day 3 · Land and seed | **Refactor** | `/gigarefine` | drive a repo's worst-scored module — tests as much as source — to its best form, behaviour preserved: the fewest statements, functions, branches and lines that still fulfil the feature set |
| Wed | Day 4 · Lights to mark seasons | **Measure** | `sandman score`, benchmarks | compute the score, run the perf and coverage passes, count statements, functions, branches and lines in source and tests, write the numbers where the Sabbath will read them |
| Thu | Day 5 · Swarms; be fruitful and multiply | **Harden** | `/security-review`, coverage, dependencies | bring the repo to 100 % coverage first (table stakes, end-to-end style), then close security findings and land dependency updates with their tests |
| Fri | Day 6 · Image; the work finished, very good | **Ship** | `/gigareview` | GO/NO-GO on the week's branches, merge the GOs, patch releases, `stele check` across the grove, CI green before sundown |
| Sat | Day 7 · Sabbath | **Rest** | the self-pass | no code commits: score the week from the trail, dream-pass, `/tighten` the machine's own prompts, rubrics and tests to their fewest lines, plan next week's targets, write the voyage entry; every seventh week, the ablation |

## The ruach — research as its own engine

Not a block inside shifts: a second loop with its own trail kinds, score and Sabbath, coupled to the shift engine through the breath store.

```
Sources ──▶ Read ──▶ Judge ──▶ File ──▶ Spike ──▶ Reweight ─┐
   ▲ (weighted)  (opus,     (Kev:    (breath,  (a shift   (adoption │
   │              budget)   bearing, provenance tries one)  raises a │
   │                        novelty)  state)                source)  │
   └────────────────────────────────────────────────────────────────┘
        ▲ open problems: plan next, findings, threads   │ breaths, filed against a repo
        └──────────────── the shift engine ◀────────────┘
```

- **Sources** — `~/.sandman/breath/sources.toml`: arXiv cs.SE and cs.AI listings, Hacker News, GitHub trending for Rust, Elixir and TypeScript, and the release feeds of the authors the owner trusts (jdx, oxc, bun, Vercel, Elixir core, Anthropic). Each carries a weight and a budget.
- **Read** — opus, under `SANDMAN_BREATH_BUDGET_USD` per day, against the **problems file** `~/.sandman/breath/problems.md` that the shift engine keeps current from every repo's `docs/plan.md` "next", open findings and open threads.
- **Judge** — Kev (free, local, `io.jlg.kev` on 1703): per candidate a Noul "does it bear on a repo?" and "is it new to us?" (against existing breaths and memories). TypeSafe when a key is present; same fallback rules as dowse.
- **File** — `sandman breathe file --url … --repo … --title …` writes `breath/<date>-<slug>.md` with provenance, the judge's scores and state `new`; a `breath` trail event.
- **Spike** — the shift brief carries the breaths filed against its target; a shift spikes at most one and marks it `adopted` or `dropped` with the reason.
- **Reweight** — adoption raises the source and the query that found it; a month of silence lowers them. The ruach's Sabbath rewrites `sources.toml` and its own prompt from its score.
- **Runners** — `~/.routines/breathe-skim.md` (hourly at :17, a cheap read-and-judge pass over the due feeds) and `~/.routines/breathe-deep.md` (daily at 10:00 UTC, `/gigaresearch` over every feed), both `--bg` sessions with cwd `~/.sandman/breath`, both exiting on PAUSE and when `breathe pass` says nothing is due or the budget is spent. Their text lives in `docs/routines/` until installed.

## Repo rotation

Scope is every root under `~/.grove/code/jgeschwendt/`. The three home repos (`~`, `~/.routines`, `~/.sandman`) are out of the rotation: home-sync commits them without trailers every fifteen minutes, and a shift there could edit the trail or the live runner; reconcile still scans them. `sandman shift pick` orders repos by least-recently-shifted, then weights by live signals: CI red, stale `graph.lock`, open findings, open threads, breaths filed against it. Only trunks Claude Code trusts are picked — `claude --bg` refuses any other cwd — and `sandman shift pick --untrusted` lists the rest for the owner (BANK-FORMAT § shifts, Trust). Invariant: every repo is shifted at least once a week; Wednesday's score reports coverage and a miss is a finding.

## Standing rails

- A shift works in `shift/<id>` worktrees created through `grove tree add`; trunk never carries in-flight work.
- A shift lands by fast-forward only — it rebases its own unpushed `shift/<id>` onto a moved trunk, then `git merge --ff-only`, never a merge commit — so reconcile reverts every landed commit on its own and refuses a trailered merge.
- Land on trunk only when the repo's test, lint and `stele check && stele emit --check` are green, coverage has not dropped — 100 % is table stakes, end-to-end style like jlg.io, and test code is source code (owner 2026-10-09: "code golf should be in the spirit of the refactor/tighten phases") — and a `/gigareview` says GO. Patch releases ship on their own; a minor or major opens a decision thread (release policy, unchanged).
- History is never rewritten and nothing is force-pushed; the trail reconciles by revert.
- Shift and breath sessions run `dontAsk` inside Claude Code's strict Bash sandbox under `sandman shift profile` (BANK-FORMAT § shifts, The sandbox) — not `bypassPermissions`, which auto-approves the sandbox's ask for an unlisted domain (probed 2026-10-09): git's network commands run outside it with the host's auth, nothing writes `~/.local/bin`, and the deny rules and the mod's guards hold in every mode.
- The orchestrator model is a knob, `SANDMAN_SHIFT_MIND`, default `opus`. Fable takes the Sabbath self-pass and Friday's verdicts while quota allows; when the weekly limit is exhausted every spawned agent pins opus or below. Workers are always opus or below.
- Cost: every headless call logs its cost to the trail; a shift stops at `SANDMAN_SHIFT_BUDGET_USD` and hands off early; the ruach stops at `SANDMAN_BREATH_BUDGET_USD` per day. `claude-breaker` keeps its cap of twenty.
- The Sabbath commits no code. Three consecutive failed blocks end a shift and open a problem thread. Threads are for what the trail cannot undo: publishing, deleting data, spending money, touching other people.
- Work larger than one shift — an architecture change, a redesign, a refactor spanning modules — is a `proposal:<owner/repo>:<slug>` decision thread with a comparison matrix, never started unapproved; Monday's Redesign executes the approved ones first (BANK-FORMAT § report).
- Every shift, the Sabbath too, writes `report.md` for the owner before its handoff — Worth your eye, Landed, Proposals, Decisions (every choice with two live options and the one rejected), Numbers — and `shift handoff` refuses without it; `sandman report`, its SessionStart hook and `sandman report serve` — a loopback page of each night, its proposals answered by form into their threads — are how he reads them.

## Phases

Chiastic: the trail is the core and was built first; each outer layer is rebuilt against what the core required. Opus agents implement; the session model decomposes, gates and reviews.

### 0 · Housekeeping — done

Retired drawing tools stripped (`design/*.tldr`, `docs/arch.tldr`, the `plan` task, the lefthook `tldraft check` gate); `docs/PLAN.md` became this file; lock rebuilt.

### 1 · Trail — done (d33d0aa, e9e906b, e604573)

`sandman trail append|log|show|revoke|today|trailers`, `sandman reconcile <event-id> [--dry-run]`, `src/grove.rs` (checkout discovery, trailer reading, env-scrubbed git), BANK-FORMAT § trail. 16 integration tests in `tests/trail.rs` including the plan's acceptance case. Reconcile has run only against fixtures; its first live run is a Sunday finding to watch. An adversarial review hardened it the same day: the reconcile branch starts from the repo's trunk and hits trunk does not reach are reported `off trunk`, merges are never reverted — the machine lands by fast-forward only, so a trailered merge refuses its repo for the operator (a third review broke two merge-side heuristics) — a repo with no trunk to name, or a manifest `trunk` that does not resolve, fails rather than falling back, reverts skip the repo's commit hooks, the scratch worktree is force-removed on every path, and a conflict or failure in any repo exits 1 after all are reported. `revoke` and `reconcile` take a shift id too — every event of the shift and its descendants, every commit whose `Shift:` names it — and the brief names its own `shift.start` as `{{shift.start_event}}`, the `--parent` of every append the shift makes.

### 2 · Shift — built

`sandman shift attach|brief|clock|handoff|pick|profile|status`; the contract is BANK-FORMAT § shifts. The rotation covers `$SANDMAN_SHIFT_OWNERS` (default `jgeschwendt`) rather than every grove owner — the grove also holds client repos, and touching other people is a thread, not a shift — never the home repos (§ Repo rotation), and weights ties by the cheap signals only (a failing `stele check`, open threads, breaths); CI red and open findings join when `score` (phase 5) records them. `attach` exists because `spawn` mints the session id only after `brief` has written `current.json`. The adversarial review's brief fixes: every Context slot is fenced as data under a source label and the template is filled in one pass, so nothing a handoff or thread says is read as an instruction or a slot; over the cap the slots give in a fixed order down to their floors (the previous handoff to 1,500 characters) and the template's own text is never cut — a brief that still will not fit is refused; `shift handoff` caps a handoff at 2,000 characters (`--truncate` to cut); `shift clock` exits 3 on an unreadable window file; an open thread counts for a repo only when it names the slug as a whole token. The runner stays paused until the operator removes `PAUSE` and installs a binary carrying `shift`.


- `sandman shift brief` composes the shift prompt — the dynamic prompt itself — from `prompts/shift.md` (a stele node the Sabbath edits): the charter, the day's theme and creation note, the previous handoff, the trail's last day, `shift pick`'s target, the bearing memories for that repo, its `docs/plan.md` "next", open threads, breaths filed against it, this week's score. Budgeted; every included item's fingerprint is logged in the `shift.start` event so Wednesday can attribute.
- `sandman shift handoff` writes `~/.sandman/shifts/<id>/handoff.md` and the `shift.handoff` event; `--abandoned` records a shift the runner found dead.
- `sandman shift clock` reads `~/.local/state/claude/window.json` — `resets_at` and `used_percentage` persisted by the harness statusline (claude 5a52667) and by the runner when a limit error names the reset — and answers how long the window has left; `--check` exits 1 when under `SANDMAN_SHIFT_MIN_WINDOW_MIN` (default 60) minutes remain — handoff is fifteen before the reset, so a shift always has 45 minutes of work — and 3 when the file is not a readable window.
- `sandman shift pick [--path]` applies the rotation.
- `~/.routines/shift.md` is the runner: every five minutes, close a finished shift (handoff, release the lock) and, if the window has room and `~/.sandman/shifts/PAUSE` is absent, open one through `spawn run` (`{"verb":"claude","action":"start",…,"model":$SANDMAN_SHIFT_MIND,"permission_mode":"dontAsk","settings":<shift profile>}`) with cwd at the picked worktree. The spine table is the routine's prose.
- The problems file for the ruach is written here, by `shift brief`, from what it already gathers.

### 3 · Mod

- A Claude Code mod — a plugin whose hooks run inside Claude Code (docs: code.claude.com/docs/en/plugins/mods; terminal 2.1.287+; hooks also run under `claude -p`, so every shift carries it, though only interactive sessions draw) — ships in the harness repo as `plugins/perpetual/`.
- Hooks: `tool.call` guards the rules with mechanical form (`pkill -f`; `git checkout --`, `reset`, `stash`, `clean` as diagnostics; force pushes; `.yml`; an Agent or Workflow call with no `model` pin) by denying the call with the rule quoted, failing closed, and appends a `guard` event; `prompt.submit` captures live corrections ("no,", "don't", "never", "stop", "i said", a repeated instruction) as `correction` events — the energy source recorded at the moment instead of mined from the archive later.
- Band above the prompt: shift id, day and theme, target repo, window clock, last trail event, read from `~/.sandman/shifts/current.json` and today's trail file on a timer. Commands `/shift` and `/trail`, answered at once without a Claude turn, output to the display log only.
- Guards generated from promoted lessons (phase 6) land here: a lesson violated after two injections proposes its guard; the guard replaces the prose and frees budget.
- The permission classifier refused to delegate this phase twice (the guard half reads as an agent that overrides tool permissions). It is built by the operator's hand or with his permission rule; recorded here so the Sabbath does not retry blindly.

### 4 · Ruach — verbs built

`sandman breathe init|sources|file|list|spike|adopt|drop|pass|attach|close|reweight` and `prompts/breathe.md`; the contract is BANK-FORMAT § breath, the seed feed list `breath/sources.toml`. The verb is a store and a prompt composer — the reading, judging and filing happen in the session the runner starts, which calls `breathe file` with the judge's scores. A pass record is `passes/<id>/{pass.json,prompt.md}`, beside its prompt as a shift's brief sits beside its meta; `pass` refuses when no source is due or the day's budget is spent, so a runner tick starts nothing then. `reweight` remembers its last run as the root key `reweighted` and counts adoptions after it; silence is measured from the first read or the last adoption and restarts after each penalty, so a source loses 0.1 per silent month. The shift brief carries `spiked` breaths beside `new` ones. The runners and the ruach's score lines and Sabbath are next.

As described above: `sources.toml`, `problems.md`, `sandman breathe file|list|spike|adopt|drop|reweight`, the hourly and daily runner, the ruach's own score lines (breaths per source, adoption rate, idea-to-merge latency) and its Sabbath. `/gigaresearch` keeps its claim ledger in `~/.sandman/breath/ledger/`.

### 5 · Score — built, minimal

`sandman score [--week yyyy-Www] [--json]`; the contract is BANK-FORMAT § score. It reads only what exists — the trail, `shifts/*/meta.json`, the breath store, `~/.threads`, the `shift` and `breathe-*` `runs.jsonl` — and writes `~/.sandman/score/<week>.md`, the worst three first so a weekday brief's 20-line `score.week` carries them; the Sabbath brief reads the whole page. Built: repo coverage against the spine's every-repo-once-a-week, shift outcomes and mean length, events by kind, repo and model, cost summed and the events carrying none, correction recurrence (title-token Jaccard against earlier corrections this week or last), context freshness (a Context slot's fingerprint seen in an earlier shift this week is stale context), breath movement and per-source counts, open threads and those shifts opened, routine ticks and failures.

Deferred: promotion precision (nothing emits `promote` or `decay` yet), recurrence per lesson rather than per correction (needs recall's attribution, phase 6), bearing measured by outcome rather than by repetition, cost per merged change and anything else from git, CI red and open findings as rotation signals, and a Wednesday runner calling `score` — today a Measure shift or the Sabbath runs it by hand.

### 6 · Compile

- `recall` becomes the compiler: candidates from the bank chain *and* the whole grove, ranked by bearing (dowse, merged in from `typesafe-integration`; Kev when no TypeSafe key), tiered budget, attribution logged to the trail.
- Reflect gains titer math: probation → active on two confirming sessions; active → index on thirty unused days; index → `_archive` on ninety; contradiction opens a thread.

### 7 · Recurse — the Sabbath built, minimal

Built: on a New York Saturday `shift brief` fills `prompts/sabbath.md` instead of `shift.md`, and `shift pick` returns sandman's trunk — the machine's own repo, where `sabbath/<week>` lives — rotation aside, while the rotation's last-shifted bookkeeping and the score's coverage ignore Saturday shifts. The Sabbath brief carries the week whole: the score page, the week's trail and corrections, breath movement, every shift's outcome, and the targets last Sabbath set (`sabbath.previous`); on the Shemitah (ISO week divisible by seven) it says to run `~/.sandman/ablation.prompt.md` instead of the self-pass. Its `shift.start` carries `theme:Rest` (every start carries its theme). The session writes `score/<next week>-targets.md`, a `## <owner/repo>` line each, and a weekday brief falls back to the target's line when its own `docs/plan.md` says nothing, the fence label naming which source it used. The contract is BANK-FORMAT § shifts.

Deferred: `promote` and `decay` are trail kinds nothing emits yet — the Sabbath prompt proposes them by hand; the prompts are not yet stele nodes with claims the Sabbath scores; the ablation prompt itself; the ruach's own Sabbath (rewriting `sources.toml` and `breathe.md` from its score); a runner that lands `sabbath/<week>` on green without a session's judgement.

## Not in this change

Cloud routines (ingress stays as it is); bridge pages for the trail, the breaths and the score (bridge gets them in its own shift); replacing the voyage log (the log stays the human-facing narrative, the trail the machine-facing ledger); the Jubilee.

## Acceptance

Phase 1 (met): a fixture repo receives two machine commits under two events; revoking the first reverts both, the trail shows the reconcile event, and the second event's memory is listed for retirement. Phase 2: one live shift opens with a brief, works a picked repo on a `shift/` branch, writes a handoff, and every commit it made resolves to the trail by trailer. Phase 4: a breath filed from a real feed reaches a shift brief for the repo it bears on.
