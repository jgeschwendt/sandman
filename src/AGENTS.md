# src

```stele
kind: container
purpose: crate source — lib + `sandman` bin; `commit.rs` is the format authority (lock, `_archive`, collisions, index regen), landmark in lib.rs; `cli.rs` dispatches `verbs/`; `mind.rs` is the only model call
```

## Map

| where | what |
| --- | --- |
| `atomic.rs` `lock.rs` | tmp→rename writes, log appends; the data root's `.commit.lock` |
| `bank.rs` `commit.rs` `memory.rs` `slug.rs` | the format authority — bank keys, index regen, frontmatter, naming |
| `cli.rs` | the dispatcher: parsing, exit codes (0 ran · 1 failed · 2 command line), the usage screen, take's detached dream spawn |
| `consensus.rs` | 2-of-3 — deterministic claim grouping by name-slug Jaccard; no model, no wording rewrite |
| `error.rs` `time.rs` | typed errors; UTC timestamps, both directions |
| `hook.rs` `transcript.rs` | SessionStart/SessionEnd payloads; locating session files, digesting a transcript, extracting the conversation a mind reads |
| `json.rs` (private) | JSON for our own shapes only — pointers, hook payloads, model replies; why `[dependencies]` is empty |
| `mind.rs` | the runner seam — one `claude -p … --output-format json` process per mind, killed at the timeout, every failure an abstention |
| `paths.rs` | the single config point — `$SANDMAN_ROOT` else `~/.sandman`, `~/.claude`, and the `log/` tree |
| `verbs/` | `dream` `forget` `recall` `reflect` `remember` `take` — each takes its roots as arguments, never the environment |

`recall.rs` is a port of `~/.claude/hooks/memory-recall.js`; its header comment
lists what changed and why.

Models and the binary are environment-configurable (`$SANDMAN_MIND_SONNET` /
`_OPUS` / `_FABLE` / `_UPKEEP`, `$SANDMAN_CLAUDE_BIN`). Unit tests cannot set
environment variables — `unsafe_code = "forbid"` — so `dream::Options` and
`reflect::Options` carry the binary, the minds and the timeout as fields:
`from_env()` in production, a stub script in a test. No test in the suite runs
the real `claude`.

<!-- stele:begin router -->
<!-- stele:end -->
