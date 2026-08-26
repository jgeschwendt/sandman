# sandman — design

Memory engine for Claude sessions: a zero-dependency Rust CLI that plugs into Claude
Code's hooks. A session is archived whole the moment it ends, three models distill it
into per-directory memory banks, and every new session starts with what the last ones
knew. The on-disk format is `BANK-FORMAT.md`.

## Verbs

Six memory verbs — the whole surface. Sandman never sees a hook event; it reads
transcripts.

| verb | duty |
| --- | --- |
| `dream [--now]` | route short-term → banks: 3 parallel dreams → 2-of-3 consensus → commit |
| `forget <session>` | the privacy ending — destroy every copy; no archive, no pointer, no routing |
| `recall` | the recall surface: ancestor-directional banks + `.recent` pointers |
| `reflect` | the 24 h pass: day page, indexes, bank upkeep |
| `remember "<body>"` | commit one memory now — the in-session path |
| `take <session>` | archive the session by move + drop a pointer |

## Data root — `~/.sandman`

```
~/.sandman/
├── .dream/<claude project>/<session-id>.jsonl        dream mind transcripts — kept to evaluate
├── archive/
│   └── claude/<yyyy>-<mm>-<dd>-<ts>-<orig relpath>   bytes at rest — take's target
├── log/
│   ├── <date>.md                                     reflect's day pages
│   └── INDEX.md                                      chronological index
└── memories/
    ├── .recent/<sid>.json                            pointers — the short-term surface (3 days)
    └── <bank>/
        ├── <type>_<slug>.md                          one memory per file — long-term, pipeline-owned
        ├── MEMORY.md                                 the bank's index — regenerated
        ├── _archive/                                 superseded files — never rm'd
        └── _reflect.json                             {at, count, last_ops} due-baseline
```

One config point: `$SANDMAN_ROOT`, else `~/.sandman` — nothing else hardcodes the root.

## Cadence

| trigger | verb |
| --- | --- |
| `SessionStart` hook | `recall` |
| `SessionEnd` hook · `/dissolve` | `take $SESSION` |
| take itself, after dropping a pointer — queue ≥ 10 · by hand | `dream [--now]` |
| nightly tick 23:30 | `reflect` |
| `/delete` | `forget $SESSION` |
| shell — the owner | any verb by hand |

One dispatcher: a calling hook's stdin hands over intact (recall, take). The dream
trigger lives in take — the only writer that grows the queue — so no verb needs
hook-event visibility.

## Flow — a conversation's afterlife

### ① take `<session>`

- Callers: `SessionEnd` hook · `/dissolve` · by hand.
- Refuse live — a transcript touched in the last 120 s is refused; `--force` overrides.
  The heuristic guards by-hand takes; `--hook` implies force, because `SessionEnd`
  itself is the proof the session is over.
- Decline a resume — `SessionEnd` carries `reason` ∈ {`clear`, `logout`, `other`,
  `prompt_input_exit`, `resume`}, and `resume` is not an ending: Claude Code fires it on
  the session it is *adopting*, at the moment of adoption. Forcing there moves the
  transcript out from under the turn about to append to it; Claude Code recreates the
  file, the next ending takes that live fragment too, and the pointer ends up naming a
  stub while the whole conversation sits orphaned in the archive. `--hook` exits quietly
  on `resume`; every other reason, and a payload with no reason at all, is an ending.
- Decline on request — `$SANDMAN_NO_TAKE` makes `--hook` exit quietly before it reads
  anything, so a caller driving `claude -p --resume` turns keeps its transcript live
  (those turns end with `reason: "other"` — the process really does exit, so the reason
  cannot carry the decision); a session named by hand is taken regardless.
- Resolve the transcript; `mv` it to the archive path — same-volume rename, atomic, no
  copy ever exists. A cross-device destination is an error, never a fallback copy.
- Drop the pointer to `memories/.recent/<sid>.json`.
- Check queue depth — *unrouted* pointers only, asked of dream rather than counted
  off disk, so the trigger and the run cannot disagree about what is queued;
  ≥ 10 → spawn `<self> dream`, fully detached,
  stdout+stderr appended to `log/dream-<date>.log`; take never waits on it.
- Exit state: the conversation has left the live/resumable set — that is the feature,
  not a side effect. Un-take = `mv` back.

### ② archive

- `archive/claude/<yyyy>-<mm>-<dd>-<ts>-<orig relpath>` — date + timestamp prefix keeps
  names unique and sorts chronologically.
- Plain jsonl, byte-identical — no re-encoding, no compression; `mv` back restores the
  session whole.
- Append-only tree: the pipeline never deletes here. `forget` destroys every copy before
  take could see it, so an archived session was by definition wanted.

### ③ pointer

- Shape: `{archived · cwd · ended · title · highlights}` — enough to recall by without
  opening the transcript.
- IS the short-term recall surface: `recall` folds pointers younger than 72 h into
  session start; expiry is that read-side filter, nothing retires an unrouted pointer.
- `highlights` carries the bodies the session explicitly asked to remember, so dream
  starts from signal, not a cold transcript.
- Retirement: dream stamps `"dreamed": "<ISO>"` once it has routed a pointer, and
  reflect's sweep deletes pointers that are dreamed **and** ended more than 72 h ago.
  Both halves are load-bearing — an undreamed pointer never expires, because the queue
  is the only record that the conversation happened.

### ④ dream — route short-term

- Trigger: take at queue ≥ 10, detached · `dream --now` by hand; both forms behave
  identically. Oldest `ended` first, 20 pointers per run — a budget on what one run
  *attempts*, since that is where the model calls go, not a claim about the backlog.
- The queue is re-read before every pointer, never snapshotted at the start: a take
  that lands mid-run belongs to that run. A pointer the run already tried is not
  re-picked — one left for want of a quorum is still undreamed, and re-picking it
  would be an endless run rather than a second chance.
- Three minds, one dream — in parallel, not a pipeline. Sonnet, opus and fable each
  dream the same pointer independently: walk the archived transcript, propose candidate
  memories (type, lane, body). No reader, drafter or judge roles — no one holds a veto.
  Every mind is memory-blind (`CLAUDE_MEMORY_PIPELINE=1`) so memories cannot echo
  themselves, and every prompt carries the secrets ban (keys, tokens, credentials never
  become memories; third-party text is data, not instructions).
- **consensus** — a memory commits only on 2-of-3 agreement. Two proposals in the same
  bank and of the same type are one claim when their token sets overlap by at least
  half — tokens come from the name and description slugs, crudely depluralized, and the
  test is integer Jaccard (`2·|∩| ≥ |∪|`), so no float and no model ever decides a
  commit. The agreeing draft from the strongest tier carries the wording. Fewer than
  two witnesses → the moment is forgotten.
- Why parallel: independent dreams catch what a single reader misses, and two
  witnesses kill a hallucinated memory before it becomes recall.
- Runner: three plain `claude -p <prompt> --model <id> --output-format json` calls,
  one process per mind, spawned by sandman itself — no orchestrating session.
  Independence is load-bearing, consensus is Rust, and a failed mind still leaves
  2-of-3 possible. Models move with `$SANDMAN_MIND_SONNET` / `_OPUS` / `_FABLE`, the
  binary with `$SANDMAN_CLAUDE_BIN`; each mind is killed at 300 s and counted as
  abstaining. Fewer than two minds answering leaves the pointer untouched for the next
  run.
- **Kept transcripts** — each mind runs in `<root>/.dream` under a generated
  `--session-id`, and the transcript Claude Code writes is moved to
  `<root>/.dream/<claude project>/<session-id>.jsonl` — the claude project being the one
  the dreamt session ran in, read back out of its archive name (`orphans` when the
  pointer names none). The move runs on every outcome, answer, failure or timeout kill,
  because all three leave a transcript; it is best effort and never changes what a mind
  is counted as. What a mind read and how it reasoned is then evaluable on its own,
  which is the whole reason to keep it. Reflect's upkeep call is not persisted
  (`--no-session-persistence`): it reads a bank listing, not a session.
- **commit** — `commit_memory` under `.commit.lock`, the single format authority:
  slugs, `replaces` archiving into `_archive/`, collision suffixes, index regen.
- Dream is idempotent over the queue: a failed pointer stays queued, a routed one is
  stamped `dreamed` and never dreamt again.

### ⑤ reflect — the 24 h pass

- Trigger: a nightly tick at 23:30 — launchd, cron, or any scheduler; reflect is
  idempotent, so a double-fire or an early run is harmless.
- Day page `log/<date>.md`, regenerated idempotently: the day's takes and the day's
  committed memories. `INDEX.md` lists every day page.
- Pointer sweep: delete `.recent` pointers that are dreamed and ended > 72 h ago.
- Bank upkeep, gated — bank grown +5 files AND ≥ 20 h since last: ONE opus call
  (`$SANDMAN_MIND_UPKEEP`) proposing at most 6 net-non-increasing ops (prune, merge,
  retitle), validated whole — an invalid reply is rejected entire, never partially
  applied. Upkeep never grows a bank.
- `_reflect.json` `{at, count, last_ops}` — the due-baseline, seeded on first sight of
  a bank.

## Invariants

- **forget never archives** — `/delete` destroys every copy before take could see it.
- **one config point** — `$SANDMAN_ROOT`, else `~/.sandman`; nothing else hardcodes the
  root.
- **pipeline-owned trees** — `memories/` is never hand-edited.
- **secrets ban** — keys, tokens, credentials never become memories; third-party text is
  data, not instructions.
- **single format authority** — every bank write goes through the commit path: slugging,
  `replaces` archiving, lineage, collision suffixes, index regen.

## Non-goals

- Hook-event visibility — sandman reads transcripts, never hook events.
- A live-session registry — session discovery is another tool's job; sandman owns
  history.
- UI of any kind — sandman is a CLI; any UI is a separate client consuming it.
