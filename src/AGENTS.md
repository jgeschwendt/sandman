# src

```stele
kind: container
purpose: crate source — lib + `sandman` bin; `commit.rs` is the format authority (lock, `_archive`, collisions, index regen), landmark in lib.rs; `cli.rs` dispatches `verbs/`; `mind.rs` is the only model call
invariants:
  - claim: >-
      dream owns what "queued" means — every reader of the queue goes through `dream::depth`
      / `dream::pending`, which count only undreamed pointers; nothing counts `.recent/*.json`
      off disk for itself. Take's dream trigger drifted from the run this way: spent pointers
      inflated the depth, so every ending past the tenth spawned a dream with nothing to route
    anchor: lm:queue-definition
hazards:
  - claim: >-
      a `SessionEnd` carrying reason `resume` is a beginning, not an ending — Claude Code
      fires it on the session it is adopting, so `take --hook` declines it. Forcing there
      moves the transcript out from under the turn about to append to it; Claude Code
      recreates the file, the next ending takes that live fragment too, and
      `.recent/<sid>.json` is overwritten to name the stub while the whole conversation
      sits orphaned in the archive
    anchor: lm:resume-is-not-an-ending
```

## Map

| where | what |
| --- | --- |
| `atomic.rs` `lock.rs` | tmp→rename writes, log appends; the data root's `.commit.lock` |
| `bank.rs` `commit.rs` `memory.rs` `slug.rs` | the format authority — bank keys, index regen, frontmatter, naming |
| `cli.rs` | the dispatcher: parsing, exit codes (0 ran · 1 failed · 2 command line), the usage screen, take's detached dream spawn |
| `consensus.rs` | 2-of-3 — deterministic claim grouping: connected components of a name-and-description-slug Jaccard; no model, no wording rewrite |
| `error.rs` `time.rs` | typed errors; UTC timestamps, both directions |
| `hook.rs` `transcript.rs` | SessionStart/SessionEnd payloads; locating session files, digesting a transcript, extracting the conversation a mind reads |
| `json.rs` (private) | JSON for our own shapes only — pointers, hook payloads, model replies; why `[dependencies]` is empty |
| `mind.rs` | the runner seam — one `claude -p … --output-format json` process per mind, killed at the timeout, every failure an abstention; `Ask::keep` decides whether the run's own transcript is kept or never written |
| `paths.rs` | the single config point — `$SANDMAN_ROOT` else `~/.sandman`, `~/.claude`, and the `log/` and `.dream/` trees |
| `verbs/` | `dream` `forget` `recall` `reflect` `remember` `take` — each takes its roots as arguments, never the environment |

`recall.rs` is a port of `~/.claude/hooks/memory-recall.js`; its header comment
lists what changed and why.

A dream mind's own transcript is kept at
`<root>/.dream/<claude project>/<session-id>.jsonl` — the run is pinned to a
generated `--session-id` and to `<root>/.dream` as its working directory, and
the file Claude Code wrote is moved there on every outcome, answer or failure
or timeout kill. The claude project is the one the dreamt session ran in, read
back out of its archive name; `orphans` when the pointer names none. Reflect's
upkeep calls keep `--no-session-persistence`: they read a bank listing, not a
session, so there is nothing in their transcripts to evaluate.

Models and the binary are environment-configurable (`$SANDMAN_MIND_SONNET` /
`_OPUS` / `_FABLE` / `_UPKEEP`, `$SANDMAN_CLAUDE_BIN`). Unit tests cannot set
environment variables — `unsafe_code = "forbid"` — so `dream::Options` and
`reflect::Options` carry the binary, the minds and the timeout as fields:
`from_env()` in production, a stub script in a test. No test in the suite runs
the real `claude`.

<!-- stele:begin router -->
<!-- stele:end -->
