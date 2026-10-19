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
    anchor: ※ queue-definition
hazards:
  - claim: >-
      a `SessionEnd` carrying reason `resume` is a beginning, not an ending — Claude Code
      fires it on the session it is adopting, so `take --hook` declines it. Forcing there
      moves the transcript out from under the turn about to append to it; Claude Code
      recreates the file, the next ending takes that live fragment too, and
      `.recent/<sid>.json` is overwritten to name the stub while the whole conversation
      sits orphaned in the archive
    anchor: ※ resume-is-not-an-ending
```

## Map

| where | what |
| --- | --- |
| `atomic.rs` `lock.rs` | tmp→rename writes, log appends; the data root's `.commit.lock` |
| `bank.rs` `commit.rs` `memory.rs` `slug.rs` | the format authority — bank keys, index regen, frontmatter, naming |
| `cli.rs` | the dispatcher: parsing, exit codes (0 ran · 1 failed · 2 command line · 3 a runner's fault: `shift clock`'s window file, `breathe pass`'s template or `sources.toml`), the usage screen, take's detached dream spawn |
| `consensus.rs` | 2-of-3 — deterministic claim grouping: connected components of a name-and-description-slug Jaccard; no model, no wording rewrite |
| `error.rs` `time.rs` | typed errors; UTC timestamps, both directions |
| `grove.rs` | the repos the machine commits into — grove checkouts (manifest roots plus the tree) and the extras; scrubbed-env git calls; `Event:` and `Shift:` trailer scans, one pass per repository |
| `hook.rs` `transcript.rs` | SessionStart/SessionEnd payloads; locating session files, digesting a transcript, extracting the conversation a mind reads |
| `json.rs` (private) | JSON for our own shapes only — pointers, hook payloads, model replies; why `[dependencies]` is empty |
| `log.rs` | the voyage log's format — the day's sources and their fingerprint, entry parse/render, the prompt, `began:` and the index; who calls the mind and when is reflect's |
| `mind.rs` | the runner seam — one `claude -p … --output-format json` process per mind, killed at the timeout, every failure an abstention; `Ask::keep` decides whether the run's own transcript is kept or never written |
| `paths.rs` | the single config point — `$SANDMAN_ROOT` else `~/.sandman`, `~/.claude`, the window file, the cloud routine's clones (`$SANDMAN_GROVE`), the prompt templates, `~/.docket`, the routines' run state and report's read cursor `.reports-read`, and the tier split the leading dot marks: raw and gitignored (`.archive/` `.dream/` `.trace/`) against versioned content (`breath/` `log/` `memories/` `pending-takes/` `score/` `shifts/` `trail/`) |
| `toml.rs` | flat TOML for `breath/sources.toml` and `shifts/rotation.toml` only — root keys and `[table]`s of strings, integers, floats and one-line string arrays, everything else refused by line; renders back canonically, comments dropped |
| `verbs/` | `breathe` `dream` `forget` `mcp` `recall` `reconcile` `reflect` `remember` `report` `score` `shift` `take` `trail` — each takes its roots as arguments, never the environment; `mcp` is the tool server (the three handlers plus the stdio loop, journalling to `.trace/mcp-<date>.log`); `trail` is the event log's format and verbs, `reconcile` reverts a revoked event's commits on `reconcile/<id>` branches and refuses a repo whose trunk carries a trailered merge, since the machine's pull requests merge by rebase; `shift` is the shift engine — the creation-week calendar (New York time, the DST rule computed), the night (`Night`: the hours shifts run in, and the anchor day whose theme a night carries — the one keying the report page too), the window clock (a stale window file read as unknown), the rotation (sandman's trunk on the Sabbath; the owner's off list, `shifts/rotation.toml`), the brief composer (`sabbath.md` on Saturday; reusing recall's bank-chain renderer as `recall::bearing`) the handoff, which refuses without the shift's `report.md`, and the landing — `land` records a rebased, trailered branch as `meta.json`'s `landing` (superseding the unmerged landings its trailers name), `pr` the runner's record of its pull request, which the brief's `prs.open` reads back — over a `Context` `cli` resolves — and the cloud: `open` (pull the state clone, the committed `current.json` as the lock, the rotation over `Cloud::clones`, the brief, commit and push) and `sync`, which `handoff --cloud` calls too; `report` is the owner's digest — the shifts' reports since the read cursor, the `proposal:` threads awaiting him, the SessionStart hook that shows the unread reports, never moves the cursor (the page's `GET /` of the newest night does) and skips the machine's own sessions — and calls no model, network or git; it also serves: `report/serve.rs` is `report serve`, loopback HTTP/1.1 one request at a time, Host- and Origin-checked, whose `POST /answer` replies through the `docket` binary (`Server::thread`, `$SANDMAN_DOCKET_BIN` resolved in `cli`) and appends a `decision` event, whose `POST /rotation` rewrites the off list and appends a `guard`, and `report/html.rs` the night page and the markdown renderer reports and proposal bodies need; `breathe` is the ruach's store — sources, breaths with URL and title-Jaccard dedupe, passes and their prompt (sharing shift's slot filler), reweight — and calls no model or network; `score` is the week's numbers — the ISO `Week` in New York days, metrics from the trail, shifts, breaths, threads and routine runs, the score page with its worst three — and calls no model, network or git |

`mcp`'s handlers are plain functions over a `Context` — data root, home, session id —
and read the environment no more than any other verb does: the stdio loop resolves
`$CLAUDE_SESSION_ID` once and hands it over.

`recall.rs` is a port of `~/.claude/hooks/memory-recall.js`; its header comment
lists what changed and why.

A dream mind's own transcript is kept at
`<root>/.dream/<claude project>/<session-id>/<mind session-id>.jsonl` — the run is pinned to a
generated `--session-id` and to `<root>/.dream` as its working directory, and
the file Claude Code wrote is moved there on every outcome, answer or failure
or timeout kill. The claude project is the one the dreamt session ran in, read
back out of its archive name; `orphans` when the pointer names none. Reflect's
upkeep calls keep `--no-session-persistence`: they read a bank listing, not a
session, so there is nothing in their transcripts to evaluate.

Models and the binary are environment-configurable (`$SANDMAN_MIND_SONNET` /
`_OPUS` / `_FABLE` / `_LOG` / `_UPKEEP`, `$SANDMAN_CLAUDE_BIN`). Unit tests cannot set
environment variables — `unsafe_code = "forbid"` — so `dream::Options` and
`reflect::Options` carry the binary, the minds and the timeout as fields:
`from_env()` in production, a stub script in a test. No test in the suite runs
the real `claude`.

<!-- @stele -->
<!-- @end -->
