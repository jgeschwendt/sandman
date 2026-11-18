# sandman

```stele
kind: system
purpose: sandman, rewritten from scratch one part at a time — `mark` appends a session's start or end, or a denied tool call (a guard), to the day's ledger file, `take` ends a marked session (archiving its transcript) with one more, `chart` prints what the data root holds as one JSON object, `check` holds every note to the memory cycle's invariants (`--fix` repairs what is mechanical), `recall` seeds a session's start and serves the agent's pulls over the memories (episodic, by repository) and the knowledge store (knowledge pages and playbooks, by topic): a lexical gate, BM25 and cached embeddings fused, a local decision model on the top 10, fused again, `show` prints one, `remember` writes a trace at once, `rethink` records the agent's verdict on a recalled note (and the recall evaluation's example), `defer` keeps work the person puts off as an intention (prospective memory, due a date or after an event) that the seed surfaces when due, `due` lists them, `done` and `drop` close one, `mcp` serves those to an agent as MCP tools, `serve` is the daemon under launchd, whose tick ends sessions that never ended, fills recall's embedding cache, settles held takes, encodes episodes into traces, a memory's by repository and a knowledge or playbook's by topic, consolidates a bank's traces into the bank and a topic's into its pages and, weekly, reflects: rewrites those two jobs' guidance from evidence, adopted after a judged replay, unattended, `status` asks it for its health, `setup` installs the release through hotswap (its layout, the `~/.local/bin/sandman` link, the upgrade agent), the daemon under launchd on that link, and retires the old sandman's agents, `upgrade` is hotswap's swap onto a newer release, `forget` erases one session from every store for good, `version` prints the release and build. Releases come from `.github/workflows/release.yaml`, one per merge to main; `mise run install` builds a checkout as `sandman-next`.
commands:
  check: mise run check
  cover: mise run cover
  install: mise run install
  test: cargo test
```

## Map

| path                     | what                                                                                       |
| ------------------------ | ------------------------------------------------------------------------------------------ |
| `src/main.rs`            | the whole binary: the root, the machine id, `mark`, `take`, `chart`, `check`, `defer`, `done`, `drop`, `due`, `forget`, `mcp`, `recall`, `remember`, `rethink`, `serve` and its tick (reconcile, embed, settle, encode, consolidate, reflect), `setup`, `show`, `status`, `upgrade` (hotswap's `manifest`), `version`; one file, recall's pipeline a closure in its arm (clippy's `single_call_fn` would fire on a module's entry), what recall and the tick share top-level |
| `build.rs`               | the build identity: `SANDMAN_RELEASE`, the release workflow's `SANDMAN_VERSION` else the crate version; `SANDMAN_BUILD`, the short commit, `-dirty` over uncommitted work, `unknown` outside git |
| `src/prompts/<job>.md`   | a model job's default guidance (`encode`, `consolidate`, `reflect`, `judge`), `<job>.contract.md` its fixed contract; both embedded with `include_str!` |
| `tests/sandman.rs`       | end-to-end suite: the built binary over temp roots and Claude homes via `SANDMAN_ROOT`, `SANDMAN_MACHINE`, `CLAUDE_CONFIG_DIR`, a fixed local day via `SANDMAN_TODAY`, stub shell model commands via `SANDMAN_ENCODER`, `SANDMAN_CONSOLIDATOR` and `SANDMAN_REFLECTOR` (`exit 9` unless a test names one, and no review unless a test lowers `SANDMAN_REFLECT_MIN` from the harness's 1000); `launchctl` a stub a test plants on `PATH`, `HOME` a temp dir (so `~/Library/LaunchAgents` is one too); every transport is curl, and the first `curl` on `PATH` is a stub: one that refuses unless a test plants `stub`'s, which logs each request by its URL and answers embeddings `[clock, deploy, beta, 1]` from the text (`SANDMAN_OLLAMA` `http://ollama.test`, `TYPESAFE_API_KEY` removed); a `Daemon` guard kills every `serve` it starts (SIGTERM one whose coverage counts: a SIGKILL writes no profile) |
| `docs/index.html`        | the page, linking the recall, knowledge and intentions pages: data root, `mark`, `take`, `chart`, `forget`, `serve` (with `setup`, `status`, `version`), `encode`, `consolidate`, `reflect`, the memory cycle (its design and staged plan; built: the note's `id`/`from`/`origin`, consolidate's rewrite lines, its merge of notes, `# judged` (revise), retitle and the order measure, ids on recall, show, remember and encode lines, `rethink` in session and at encode, `eval/recall.jsonl`, `check`'s invariants 1–5 and `--fix`, chart's `id`/`from`/`origin`/`state`, `ended` and `lineage`; strength on chart's rows; not built: decay, graduation, release, `check`'s 6–7 and `--release`, chart's `held`, `released`), next; two d3 charts, the daemon's lifecycle, forget's order and the cycle's four figures |
| `docs/knowledge.html`    | the knowledge store's design page: the problem, the three memory systems and what maps to what, the store, how a page is written and revised (the split figure), the playbook's shape, recall, next, open questions |
| `docs/intentions.html`   | prospective memory's design page: the problem, time- and event-based, the verbs, the store, what the seed shows (the timeline figure, the pinned due block), surfaced never acted on, next, open questions |
| `docs/recall.html`       | `recall`'s design page: the problem, the flow (the seed and the agent's pull), one section per decision, the agent's tools (commands and MCP), the pane; the reach chart and the flow figure; what is built marked so |
| `docs/charts/*.js`       | one ES module per chart over `lib.js`'s chrome and `chart.css`, both pages' chart styles; d3 7.9.0 is the page's global |
| `docs/data/chart.json`   | the timeline's sample: `tests/sandman.rs` `chart` fails unless it is the verb's output     |
| `docs/data/recall-reach.json` | the reach chart's counts per bank, banks unnamed, from `sandman-next chart` (2026-11-13) |
| `docs/data/recall-*.txt` | the recall page's samples: `tests/sandman.rs` `recall` fails unless `-seed` is the seed's stdout and `-digest` a pull's |
| `docs/data/flow.json`    | the data-flow chart's moments, nodes and edges, ①–④ marking the order a session passes them; `next` nodes are not built yet |
| `Cargo.toml`             | the rust base's `lints.toml` verbatim; the one opt-out and `signal-hook` carry their reasons |

The gate is `mise run check` (fmt, clippy `-D warnings`, `cover` at 100 % of lines), run by hk's pre-push (`hk install --mise` once per clone) and by CI. `lints check` from jgeschwendt/rust holds `Cargo.toml` to the base. Tests never touch `~/.sandman` or a real server; a child keeps the parent env (llvm-cov's `LLVM_PROFILE_FILE`), with `HOME`, `CLAUDE_*`, `PATH`, `SANDMAN_*` and `TYPESAFE_API_KEY` replaced.

## Contracts

Root: `$SANDMAN_ROOT`, else `$HOME/.sandman`. Machine id: `$SANDMAN_MACHINE`, else `<root>/machine` (one line, 32 lowercase hex from `/dev/urandom`), minted on first use by a hard link from a temp file so racing first runs agree. `sandman` with no or another verb: twelve-line usage on stderr, exit 2. `--session` anywhere must be a session id (as `mark` checks), else one stderr line, exit 2. A command printed for an agent to run (the seed's instructions, the footer, `remember`'s line) names `<me>`, the file name the binary was invoked as, so `sandman-next` prints `sandman-next show …`; usage and error prefixes say `sandman`.

### `sandman mark`

Stdin, one object; `kind` and `session` (36 chars, hex and `-`) required, anything else missing is null; a `guard` (a tool call the subether mod's guard denied) requires a string `rationale`:

```
{"kind":"start"|"end","session":"<uuid>","cwd":"<path>"?,"surface":"<str>"?,"interactive":bool?,"reason":"<str>"?,"resume":"<str>"?}
{"kind":"guard","session":"<uuid>","rationale":"<str>","tool":"<str>"?}
```

Appends to `<root>/ledger/<yyyy>/<mm>/<dd>.jsonl` (UTC), compact, keys alphabetical, in one `O_APPEND` write under a shared `flock`, opened again when the file it waited on was replaced (`forget`); every ledger, log and embedding-cache append is this writer:

```
{"at":"<RFC 3339 UTC, seconds, Z>","cwd":…,"interactive":…,"kind":…,"machine":…,"reason":…,"resume":…,"session":…,"surface":…}
{"at","kind":"guard","machine","rationale":<one-lined, 1,000>,"session","tool":<str>|null}
```

Every reader of sessions (`sessions()`: chart's `active`, take, reconcile, settle, encode) reads only `start`, `end` and `encode` lines, so a guard never opens, ends or encodes one; reflect's evidence reads its own kinds.

Exit 0 silent; 2 on bad stdin, one stderr line, nothing written; 1 when the line cannot be written. Runs inside the subether mod's 1.5 s budget.

### `sandman take`

The end of a session, inside the mod's 1.5 s budget; it spawns nothing. Stdin, one object, `session` checked as in `mark`:

```
{"session":"<uuid>","cwd":"<path>"?,"reason":"<str>"?}
```

The first that holds: no start mark for the session in the ledger, or `reason` `resume` → nothing written. Else one end line through `mark`'s writer, `outcome`: `skipped` when `$SANDMAN_NO_TAKE` is non-empty or `CLAUDE_MEMORY_PIPELINE=1`; `held` (plus `job`, the dir name) when `reason` is not `clear` and a `<claude>/jobs/*/state.json` names the session by `resumeSessionId`, else `sessionId`, its `state` neither `done` nor `stopped` (Claude Code keeps a finished job's file: such a job names nothing, here or in the tick); `gone` without `<claude>/projects/*/<session>.jsonl`; else `taken` (plus `archived`). The tick's reconcile and settle end a session through the same path, reasons `no end recorded`, `idle` and `job gone`. `<claude>` = `$CLAUDE_CONFIG_DIR`, else `$HOME/.claude`. The pointer below stays until `recall` replaces the earlier one.

```
{"archived"?,"at","cwd","job"?,"kind":"end","machine","outcome","reason","session"}
```

The take, byte-compatible with the earlier sandman's: `fs::rename` the transcript to `<root>/.archive/claude/<yyyy>/<mm>/<dd>/<HHMMSS>-<path under <claude>, / → ->` (UTC take time; `<HHMMSS>-<n>-…` on a same-second clash) and `<project>/<session>/`, when present, to that stem + `-dir`; then `<root>/memories/.recent/<session>.json` by temp + rename, compact, newline-terminated, in this key order: `{"archived","cwd","ended","title","highlights"}`. `ended` the transcript's mtime; `cwd` the first line's `cwd` without control characters, else stdin's, else null; `title` the first `type:"user"` line's string content or first `text` block, whitespace collapsed, 80 chars, else the session id; `highlights` the quoted bodies (`"…"` with `\"` `\\` unescaped, or `\"…\"`) after `sandman remember ` then `orrery attend ` in assistant `tool_use.input.command`s and user `<bash-input>` lines, one-lined, 4,000 chars, a repeat moved to the end, the last 20. Rename, pointer, ledger line, in that order. Exit 0 silent when decided; 2 on bad stdin, nothing written; 1 when a rename or write fails (cross-device included), one stderr line, no ledger line.

### `sandman chart`

Stdout, one compact object, keys alphabetical; reads the root, writes nothing but a missing `machine`; every absence is an empty list or a skipped item; a closed stdout ends it quietly (exit 0):

```
{"active":[{"cwd":str|null,"encoded":str|null,"end":str|null,"interactive":bool|null,"machine":str,"outcome":str|null,"seen":str|null,"session":str,"start":str|null}],
 "at":"<now, RFC 3339 UTC>",
 "ended":[{"at":str,"id":str,"into":str|null,"op":str,"path":str,"sessions":[str],"state":"ended"}],
 "guards":[{"at":str,"rationale":str,"session":str,"tool":str|null}],
 "intentions":[{"about":str|null,"after":str|null,"closed":str|null,"created":str|null,"due":str|null,"id":str,"session":str|null,"state":str|null,"what":str}],
 "knowledge":[{"created":str|null,"file":str,"from":[str],"id":str|null,"name":str,"origin":str|null,"session":str|null,"sessions":[str],"source":…,"state":"active","strength":number,"topic":str,"type":str}],
 "lineage":[{"at":str,"from":str,"op":str,"to":str}],
 "machine":"<this machine's id>",
 "memories":[{"bank":str,"created":str|null,"file":str,"from":[str],"id":str|null,"name":str,"origin":str|null,"session":str|null,"sessions":[str],"source":"consolidate"|"dream"|"reflect"|"remember"|"other","state":"active","strength":number,"type":str}],
 "taken":[{"bytes":int,"cwd":str|null,"dreamed":str|null,"highlights":int,"memories":int,"session":str,"state":"waiting"|"dreamed"|"archived","taken":str,"title":str|null}],
 "traces":[{"bank":str,"created":str|null,"file":str,"from":[str],"id":str|null,"name":str,"origin":str|null,"session":str|null,"sessions":[str],"state":"active","strength":number,"type":str}]}
```

- `active` — per session in `ledger/*/*/*.jsonl`: `start` the `at` of its earliest start mark (a reload re-fires it), `end` and `outcome` of its latest end mark (null on an end line without one), both null when a start mark is later (resumed: running again); `cwd`, `machine` from that start mark, else the end mark; `encoded` the `outcome` of its latest encode line; `interactive` true when a start mark says true, else false when one says false, else null; `seen` the mtime of `<claude>/projects/*/<session>.jsonl` (`take`'s finder), null without one. By `start` (null first), then session.
- `ended` — per `from` entry of a `rewrite` line (`### consolidate`) with an id other than the line's `to` id (a keep continues its note): `at`, `op` the line's, `id`, `path`, `sessions` the entry's, `into` the `to` id (null for a prune). By `at`, in ledger order.
- `lineage` — the same entries whose line has a `to` id: an edge `from` the entry's id `to` it. By `at`.
- `guards` — per `guard` line in the ledger, by `at`: what bifrost's portals page reads in place of the old `trail/`.
- `taken` — per file `<root>/.archive/claude/<yyyy>/<mm>/<dd>/<HHMMSS>-…-<uuid>.jsonl` directly in a day dir; the latest per session wins. `taken` = that date and time as UTC, `bytes` = size; `title`, `cwd`, `highlights` (count, 0 without), `dreamed` from `<root>/memories/.recent/<session>.json` when it parses as an object. `state`: `waiting` pointer without `dreamed`, `dreamed` with it, `archived` no pointer. `memories` = memory and knowledge entries whose `sessions` holds the session. By `taken`, then session.
- `memories` — per `<root>/memories/<bank>/<file>.md`, banks and files starting `.` or `_` and `MEMORY.md` skipped. Frontmatter: `key: value` lines between `---` fences; `name` (else the stem), `type` (else ""), `created`, `source` → `consolidate`/`dream`/`reflect`/`remember` by its first word, else `other`; `session` the uuid in a consolidate's or a dream's (`dream <ISO> · <uuid> · minds=…`); `sessions` the note's provenance, the archived sessions it descends from, sorted and once each: the uuids on its `sessions:` line (space-separated, directly before `source:`), its `session:` and that `session`; `id` and `origin` the frontmatter's, `from` its `from:` ids, sorted and once each (docs/index.html#cycle-note); `state` `active`, every note on disk; `strength` (docs/index.html#cycle-strength) `(1 + $SANDMAN_USED_WEIGHT (3) · used + $SANDMAN_SHOWN_WEIGHT (1) · shown − $SANDMAN_JUDGED_WEIGHT (4) · judged) × 2^(−idle ÷ $SANDMAN_HALF_LIFE_DAYS (30))` over the ledger lines whose `file` is the note's path or whose `id` is its id: `used` its `rethink` lines with verdict `used`, `shown` its `show` lines, `judged` its `stale` and `wrong` verdicts with `at` after its `updated`, else `created` (all without either), `idle` whole days from the latest of its `created`, `updated`, used verdicts and shows to the local today (`$SANDMAN_TODAY`, as `defer`'s), 0 without any, rounded to 3 places. By bank, then file.
- `intentions` — per `<root>/intentions/<id>.md` (`### sandman defer`), open or closed: its frontmatter's fields, `what` its body. By `created`, then id.
- `knowledge` — per `<root>/knowledge/<topic>/<file>.md`, as `memories`, `topic` in place of `bank`. By topic, then file.
- `traces` — per `<root>/traces/<bank>/<file>.md`, the same reader and skips, a topic's bank `@<topic>`; `session` from the frontmatter, `sessions` as a memory's. By bank, then file.

### `sandman check`

`sandman check [--fix]`: every note the `notes` reader accepts (memories, knowledge pages, traces, reader order) and then every `rewrite` line in the ledger held to the memory cycle's invariants 1–5 (docs/index.html#cycle-check); a violation is one stdout line `<path>: <n>: <detail>`, `<path>` what `show` takes or `ledger <the line's at>`, in that order, then exit 1; none: `check: <n> notes: clean`, exit 0. Plain `check` writes nothing (no ledger line, no machine id).

1. `no id`; `id <id> is not 12 lowercase hex`; `id <id> is also <path>'s` (the first in reader order keeps it);
2. `from`, `origin` or `sessions` `is ""` or `is "-"`; `origin <value> is not hand, import or routine`; `no source`: chart's `sessions` empty, no `from:` id and no valid `origin`;
3. `from: <word> is not an id`, `sessions: <word> is not a session id`, `<key> is not sorted once`; `strength is counted, never stored`;
4. `from <id> was never ended` (no rewrite line ends it, chart's `ended`); `id <id> was ended`;
5. per rewrite line: `not a rewrite line` (a `from` array of `{"digest":str,"id":str|null,"path":str,"sessions":[str]}`, an `op` of the nine, `model` and `reason` string or null, `to` null, `{"id","path"}` or `{"artifact"}`); `id <id> ended again`; `to <id> is neither active nor ended later`.

`--fix` first rewrites, by temp + rename, each note it changes: an empty or `-` `from:` or `sessions:` dropped, else its words (`-` dropped) sorted and once; an `origin:` that is none of `hand`, `import`, `routine` dropped; a minted id first (12 hex from `/dev/urandom`, the old line dropped) when it has none valid of its own or an earlier note holds it; then, with no source left, `origin:` before `sessions:` or `source:` (else last) by its stamp's first word: `remember` → `hand`, `consolidate`, `dream`, `reconsolidate`, `reflect` → `routine`, else `import`; a file without frontmatter gains it. Stdout `check: fixed <n>`, then `{"at","fixed":n,"kind":"check","machine"}` through `mark`'s writer, then the check over what is left. A failed write: `sandman check: <error>`, exit 1.

### `sandman recall`

Two entries, one pass (docs/recall.html#tool). The **pull**, the agent's, any time: `sandman recall --cue "<text>" [--about <path>] [--session <uuid>] [--budget <chars>]`. The **seed**, the mod's at a session's first prompt: `sandman recall` with no `--cue` reads one object on stdin, `session` checked as in `mark` (else one stderr line, exit 2, nothing written):

```
{"session":"<uuid>","cwd":"<path>"?,"surface":"<str>"?,"interactive":bool?,"prompt":"<the first prompt's text>"?}
```

The pass, in order, every constant `reference.py`'s, the evaluation's choice (docs/recall.html#chosen), parity on a synthetic corpus to the last explain line:

- **set** — every note the `notes` reader accepts under `<root>/memories/`, then `<root>/knowledge/` (path `knowledge/<topic>/<file>`, bank `@<topic>`), then `<root>/traces/` (a trace is a candidate the moment it is written; its path, below, is `traces/<bank>/<file>`). Frontmatter `recall: mute`: nothing. `type: user` or `recall: pin`, outside a `@<topic>` bank: the always-set, unscored, never sent, printed by the seed only. The rest are the candidates, in reader order (bank, then file); `considered` counts them all.
- **repository** — a bank's label: its key equal to `key(<home>)` → `home (~)`, to `key(<grove>)` → `grove root (every repository)`; `@<topic>` → `knowledge · <topic>` (no repository's, so no bonus); equal to or extending (`-` after) `key(<grove>/<owner>/<repo>)`, the longest first → `<owner>/<repo>` (a worktree's bank folds into its repository's); equal to `key(<grove>/<owner>)` → `<owner> (owner: all its repositories)`; extending an owner's → `<owner>/<the rest>` (a repository gone from disk); extending `key(<claude>/jobs)` → `scratch dir`; else the key read back as a path, `--` as `/.` and `-` as `/`, `~` for the home prefix. `key` is the encode's bank key, so it is lossy: a label is resolved by listing `<grove>`'s owner and repository dirs (hidden ones skipped) and comparing keys, and a bank matching none reads as the path guess. `<grove>` = `$SANDMAN_GROVE`, else `$HOME/.grove/code`.
- **about** — given: `--about`'s path, else the seed's `cwd` when it lies at least two levels under `<grove>`, as the label of `key(project(it))`; given, it leads the state (`repository: <given>\n<cue>`, else the cue) and joins the BM25 query. Else the first repository label, longest key first (ties by path), whose `<repo>` lowercased is one of the cue's lowercase `[a-z0-9.]` runs; inferred, it earns its bank's bonus only.
- **tokens** — lowercase ASCII alphanumeric runs, a one-character run or a `STOPWORDS` word dropped, a final `s` dropped past four characters.
- **gate** — no token, or every token in `STEER`: no ask, no model, no ranked line.
- **head** — `<label> · <name>\n<description one-lined to 400>`.
- **bm25** — k1 1.2, b 0.75, idf ln(1 + (n − df + ½) ÷ (df + ½)), of each candidate's `<head>\n<body> <file without .md>` tokens for the distinct tokens of `<cue> <given>`; plus 4 in the about's bank, else 1 in home's or the grove root's, none when its `<name> <description>` lowercased holds a `CLAIM` phrase or `every`, `any` or `all` ` request` ending a word (an `s` allowed). Descending, a tie in reader order.
- **cosine** — the query `EMBED[1]` + the state embedded, against each candidate's `<head>\n<body's first 1,500 characters>` from the cache; a candidate it lacks: at most `$SANDMAN_RECALL_EMBED` (8), the best by BM25, embedded and appended, the rest in the BM25 rank only. Sums are Python 3.12's (Neumaier). Descending, a tie in reader order.
- **embedding** — `$SANDMAN_OLLAMA` (`http://127.0.0.1:11434`) `/api/embed`, `{"input":[…16 at most],"model":"qwen3-embedding:0.6b","options":{"num_ctx":1024},"truncate":true}`, each vector L2-normalised. The cache `<root>/embeddings.jsonl`, one `{"h","path","v"}` a line, `h` the SHA-256 hex of `qwen3-embedding:0.6b\n1024\n<text>` (the reference's key: its cache copies in), the last line per `h` winning. A failure is `embed: <cause>`: the query's drops the cosine rank, an inline one leaves those candidates lexical.
- **fuse** — reciprocal rank: each rank r from 0 adds 1 ÷ (61 + r), BM25's then the cosine's; descending, a tie in BM25's order. No cosine rank: BM25's order.
- **decide** — the fused top 10 in one request, `{"model","questions":{"<rank>":{"instructions":"Memory (<label>): <name> — <description one-lined to 160>. QUESTION","type":"noul"}},"state"}`, to each of `$SANDMAN_SCORERS` (`pplx,kev,jev`; an unknown name skipped, empty none) until one answers a noul per question: `pplx` `pplx-decider-flash:4b` at `$SANDMAN_OLLAMA/v1/systemone`, `kev` `kev-latest` at `http://127.0.0.1:1703/v1/systemone`, `jev` `jev-latest` at `https://api.typesafe.ai/v1/systemone`, skipped while `<root>/jev-down.json`'s `until` (ms since the epoch) is ahead. A failure is `<scorer>: <cause>`; Jev's writes `{"error","until"}` to `jev-down.json`, `until` now + `$SANDMAN_JEV_HELD_MS` (21,600,000) when its stdout or stderr holds `billing_error`, `error: 401`, `error: 402` or `error: 403`, else + `$SANDMAN_JEV_DOWN_MS` (60,000); no key is `no API key: set TYPESAFE_API_KEY or write it to ~/.config/typesafe/api_key`.
- **transport** — curl through `sh -c`, own process group, the request on stdin, the reply on stdout read beside the wait; `curl -q -sS --fail-with-body --max-time <bound> -H 'Content-Type: application/json' --data-binary @- <url>`; Jev's key, `$TYPESAFE_API_KEY` else `~/.config/typesafe/api_key` (trimmed), given to `sh` as `SANDMAN_SYSTEMONE_KEY`: a builtin `printf` pipes the `Authorization` header to curl as config on its stdin (`-K -`), the request goes on fd 3 (`@/dev/fd/3`), and curl runs with that variable emptied, so the key is never in an argv. The group is killed when the pass's bound runs out: `$SANDMAN_RECALL_MS` (10,000) for a pull, `$SANDMAN_SEED_MS` (4,000) for the seed, from the pass's start, every embedding and scorer included. Failures: `<status>: <stderr>`, `timed out after <ms> ms`, `output: <parse error>`, `reply: no noul for question <rank>`, `reply: not one embedding per text`.
- **cut** — a scorer answered: its order (descending noul, a tie in fused order) and the fused order within the 10 fused again, 1 ÷ (61 + its rank) + 1 ÷ (61 + the fused rank); descending, a tie newer `updated`, else `created`, then the scorer's order; kept at 2/66 or more, at most `$SANDMAN_RECALL_CAP` (6). None, with a cosine rank: the fused order, kept at 0.028438886647841874 or more and within 0.006272091157751175 of the top, at most 8. Neither: BM25's, descending, a tie newer first, at most 5. A line's score: a fused one × 61 ÷ 2 (the best possible 2/61 reads 1), BM25's ÷ the top one (÷ 1 when that is 0). Then a kept candidate sent by an earlier recall of the session is skipped; a line is cut, and every one after it, past the at-most or the budget.
- **pull's stdout** — within `--budget` characters (4,000): `recall · <cue, 60> · <sent> of <considered> · <path>[: <failures' stage names joined by " and "> failed]`, `<path>` the scorer, `hybrid, no decision model` or `lexical` (absent when the gate closed or there is no candidate); the ranked lines `.NN <mark><name> (<type> · <path>) — <description, 160>`, `.NN` the score to two places without a leading 0, `<mark>` `~` for a trace (provisional until consolidated), else nothing, `<path>` what `show` takes; `read one in full: <me> show <path>`; no line: `nothing recalled` in place of the lines and the footer, `nothing recalled: no specific ask` when the gate closed. Exit 0 whatever the models did.
- **seed** — first, when stdin's `interactive` is true, the start mark through `mark`'s line (`kind` `start`, `reason` and `resume` null), whatever follows; a headless session (`interactive` false, null or absent: a routine's `claude -p`) gets the same text and ledger line but no mark, so `take`, reconcile and encode never see it. `CLAUDE_MEMORY_PIPELINE=1`: nothing else, exit 0. Else stdout within `$SANDMAN_SEED_BUDGET` (9,000) characters, the fixed parts and the always-set counted first: the instructions (`SEED`, `<id>` the session), a blank line, the due block (`### sandman due`, at most 10 rows, then `  … <n> more: <me> due`; absent when nothing is due), `always · about the person, or pinned`, the always-set `  · <mark><name> (<type> · <path>) — <description, 160>`; after a pass, the pull's header and ranked lines; the footer. The same pass in its own bound, so a seed never fails the prompt.
- **stats** — stderr: `sandman recall: <error>` when a stage failed, then `recall: scorer=<"…"|null> embedded=<bool> ask=<bool> about=<"…"|null> ms=<wall> tokens=<usage.input_tokens> considered=<n> sent=<n> cut=<n>`.
- **ledger** — through `mark`'s writer, a failed write ignored: `{"about"?:"<label>","always"?:["<path>",…],"ask":bool,"at","considered","cue":<one-lined, 200>,"cut","embedded":bool,"error"?:"<stage>: <cause>[; …]","kind":"recall","machine","ms","scorer":"pplx"|"kev"|"jev"|null,"seed"?:true,"sent":[{"file":"<path>","id":<the note's id>|null,"score":<2 places>}],"session":<uuid>|null,"tokens"}`; `always` and `seed` on a seed's line, the always-set it printed; `about` given or inferred; `embedded` whether the cosine rank took part; `scorer` the model that decided, null for the hybrid and lexical cuts.

### `sandman remember`

The agent's write: `sandman remember "<body>" [--type user|feedback|project|reference|knowledge|playbook] [--topic <topic>] [--name <name>] [--description <line>] [--about <path>] [--session <uuid>]`, the body first. `--type` defaults to `feedback`, to `knowledge` when `--topic` is given; a memory type ignores `--topic`. An empty body, another type, or `knowledge`/`playbook` without a topic slug (lowercase ASCII letters, digits and `-`, a letter or digit first): `sandman remember: takes a non-empty "<body>" first and --type user|feedback|project|reference, or knowledge|playbook with --topic <slug>`, exit 2, nothing written. One trace at once, `<root>/traces/<bank>/<type>_<slug>.md` by temp + rename, encode's slug, clash and writer, `about` and `session` written when given:

```
---
id: <12 hex, minted>
name: <--name, else the body's first eight words; one-lined, 200>
description: <--description, else the body's first line one-lined to 160; one-lined, 400>
type: <--type, else feedback>
created: <now>
about: <--about, canonical when it is a directory>?
topic: <--topic>?                (knowledge and playbook only)
session: <uuid>?
origin: hand                     (without --session)
source: remember <now> · <uuid, else ->
---

<body, trimmed>
```

`<bank>` is `@<topic>` for a knowledge type, else `key(project(dir))`, `dir` `--about` when it is a directory (a relative one from the cwd), else the cwd, else `$HOME`: without `--about`, a session started at the root lands in the root's bank. Stdout `<me> show traces/<bank>/<file>`; then `{"at","file":"traces/<bank>/<file>","id","kind":"remember","machine","session":<uuid>|null,"type"}` through `mark`'s writer (a failed write ignored). A failed trace write: `sandman remember: <error>`, exit 1, no ledger line.

### `sandman show`

`sandman show <path> [--session <uuid>]`, `<path>` `<bank>/<file>.md` for a memory, `knowledge/<topic>/<file>.md` for a knowledge page, `traces/<bank>/<file>.md` for a trace: the note the `notes` reader finds at exactly that path (so no `..`, absolute path, hidden or `_` name, or `MEMORY.md` resolves) as `<name> (<type>[ · trace]) · <updated, else created, else undated>` and its body, frontmatter stripped and trimmed; then `{"at","file","id":<its id>|null,"kind":"show","machine","session"}` through `mark`'s writer (a failed write ignored), the signal that a recalled note was read. No such note: `sandman show: no memory <arg> (want [traces/]<bank>/<file>.md or knowledge/<topic>/<file>.md under <root>)`, exit 1.

### `sandman rethink`

`sandman rethink <path> --irrelevant|--used|--wrong|--stale [--why "<text>"] [--session <uuid>]` (docs/index.html#cycle-rethink): the agent's verdict on a note, `<path>` as `show` takes it. Not exactly one verdict flag: `sandman rethink: takes <path> first and one of --irrelevant, --used, --wrong, --stale`, exit 2; no note there: `show`'s line under `sandman rethink:`, exit 1; nothing written either way. Else, through `mark`'s writer, `cue` the latest recall line of the same `session` (null matching null) whose `sent` holds the note by `file`, else by `id`:

```
{"at","by":"agent","cue":<that send's>|null,"file":"<path>","id":<the note's>|null,"kind":"rethink","machine","session":<uuid>|null,"verdict","why":<--why>|null}
```

and, with a send, an example appended to `<root>/eval/recall.jsonl` by the same writer (`eval/` created), `label` 0 for `irrelevant`, else 1, `about` and `cue` the send's, `digest` the SHA-256 hex of the note's file: `{"about","at","cue","digest","id","label","path","session","verdict"}`. Stdout `rethink: <verdict> <path>`. A failed write: `sandman rethink: <error>`, exit 1. The seed's instructions name it in one sentence; encode's verdicts are the same lines, `by` `encode` (`### encode`).

### `sandman defer`

Prospective memory (docs/intentions.html): work the person puts off, kept as an intention, surfaced and never acted on; no tick job reads it. `sandman defer "<what>" --when <when> [--about <path>] [--session <uuid>]`, the what first; `sandman defer --id <id> --when <when>` pushes one. `<when>`, trimmed and lowercased, against the local today (`$SANDMAN_TODAY`, the suite's fixed day, else `date +%F` in the machine's zone): `today`, `tomorrow`, a weekday name (the next one after today), `+<n>d`, `+<n>w` (`n` to 65,535), or a real `YYYY-MM-DD` from 1970 → `due` that date; any other non-blank text → `after` the text as given, one-lined to 400 (an event, a malformed date included). A blank or missing `--when`, or no what and no `--id`: `sandman defer: takes "<what>" first, or --id <id>, and --when today|tomorrow|<weekday>|YYYY-MM-DD|+<n>d|+<n>w|<event>`, exit 2, nothing written.

The store, `<root>/intentions/<id>.md` by temp + rename, `<id>` 8 hex from `/dev/urandom`, the fields in this order, an empty one left out, the body the what trimmed:

```
---
due: <YYYY-MM-DD> | after: <event>
created: <now>
about: <--about, canonical when it is a directory>?
session: <uuid>?
state: open|done|dropped
closed: <now>?                   (done, drop)
note: <--note>?
---

<what>
```

A push rewrites the file in that order: `due` or `after` set, the other gone, `state` `open`, `closed` gone, `note` `--note`'s or gone. Stdout `<id> <date>` or `<id> waiting: <event>`. Then `{"after"|"due","at","id","kind":"defer"|"push","machine","note"?,"session":<uuid>|null}` through `mark`'s writer (a failed write ignored). A failed file write: `sandman defer: <error>`, exit 1, no ledger line. Nothing expires or closes on its own.

### `sandman done`, `sandman drop`

`sandman done|drop <id> [--note "<text>"] [--session <uuid>]`: closes one, `state` `done` or `dropped`, `closed` now, `note` `--note`'s or gone, the rest kept; stdout `<id> done|dropped`; a ledger line `{"at","id","kind":"done"|"drop","machine","note"?,"session"}`. An id (here and in `defer --id`) matches every intention whose id starts with it, open or closed; none or several (a blank id matches none): `sandman <verb>: <id> is not one intention's id (<n> match)`, exit 1, nothing written.

### `sandman due`

`sandman due [--all]` (`--all false`, as MCP passes it, is not all): the due block, else `nothing due`. A header, `due · deferred work: raise it with the person, never start it unasked`, then a row per open intention, `  · <id> <due | waiting: <after>> — <body one-lined to 160>[ (<label>)]`, `<label>` recall's repository label of `key(project(about))` when `about` is set: those with `due` at or before the local today by `due` (most overdue first), then every one with no `due` (an event, always listed), then with `--all` the rest by `due`; ties by `created`. Writes nothing.

### `sandman forget`

`sandman forget <session>`, the session id as `mark` checks it (else `sandman forget: takes a session id`, exit 2): erases the session so that nothing of it is kept, for real, unlike everything else here (no `.pruned/` copy), and records nothing naming it (no ledger line, no log line). The `/delete` skill and bridge's forget button run it twice, in-session and from a watcher once the CLI has exited, because the exit flush regrows the transcript. In order:

1. the ledger: every day file, today's too, rewritten without each line holding the id, but a `rewrite` line, kept with the id off each `from` entry's `sessions` (dropped too if it still names it), so invariants 4–5 hold through it; under an exclusive `flock` on the file (the writer's shared one, `### sandman mark`, so an append in flight finishes first and one waiting reopens the replacement), by temp + rename; first, so a `take` or a tick racing it finds no start mark and archives nothing;
2. removed: `<claude>/projects/*/<session>.jsonl` and `<session>/` in every project; every `.archive/claude/**` entry whose name holds the id (the copy and its `-dir`); `memories/.recent/<session>.json`; every memory, knowledge page and trace whose `session` is the id (an encoded or remembered one, a consolidate's keep); every `.pruned/**` file whose frontmatter `session` is it; `run/<job>.in|out|err` holding the id, and `run/encode.*` when, of every encode line and this session's end lines, the latest names it (the last or next encode was this episode's);
3. rewritten by temp + rename: a pruned reply `<HHMMSS>-<bank>-ops.json` without the ops on a trace whose copy `<HHMMSS>-<bank>-<trace>` went; every other memory, knowledge page, trace, pruned file and intention holding the id, its `session: <id>` line dropped, the id taken off a `sessions:` line (the line dropped when it empties) and else `-` (a merge's `source` stamp; an intention stays, the person's, its session link gone); `embeddings.jsonl` without the removed notes' lines, `eval/recall.jsonl`, `run/serve.log` and `serve.log.1` without lines holding it, each under the lock as the ledger is.

What a consolidate merged into another memory stays in that memory's body: a merge cannot be unpicked, so only what `session:` attributes is removed. Claude Code's own stores (`history.jsonl`, `file-history/<session>/`, `session-env/<session>/`) are not sandman's. An encode in flight when it runs may still write its traces and line; the second run takes them. Stdout `forgot: <n> removed, <n> rewritten`, else `nothing to forget`, exit 0 (idempotent); a failed removal or write `sandman forget: <error>`, exit 1, the rest left for the next run.

### `sandman mcp`

An MCP server over stdio for the agent's verbs, newline-delimited JSON-RPC 2.0, one compact reply line per request, keys alphabetical; the legacy era (an `initialize` handshake), which Claude Code falls back to when its `server/discover` probe gets an error. `initialize` answers `{"capabilities":{"tools":{}},"protocolVersion","serverInfo":{"name":"sandman","version":<crate version>}}`, the client's `protocolVersion` when it is `2025-11-25`, `2025-06-18`, `2025-03-26` or `2024-11-05`, else `2025-11-25`; `ping` `{}`; `tools/list` `{"tools":[…]}`, alpha-sorted, each description the seed's when-to-pull guidance:

```
defer    {"what":string, "when":string, "about"?:string, "id"?:string, "session"?:string}
done     {"id":string, "note"?:string, "session"?:string}
drop     {"id":string, "note"?:string, "session"?:string}
due      {"all"?:boolean, "session"?:string}
recall   {"cue":string, "about"?:string, "budget"?:number, "session"?:string}
remember {"body":string, "about"?:string, "session"?:string, "topic"?:string, "type"?:string}
rethink  {"path":string, "verdict":string, "session"?:string, "why"?:string}
show     {"path":string, "session"?:string}
```

`tools/call` runs the verb as `<this executable> <tool> …`, stdin closed: the first required argument positional (recall's as `--cue`, `""` when missing or none is required), `verdict` as `--<verdict>`, every other given one `--<name> <value>` (a number as its digits), `--session` always: so the text, the ledger lines and the session's de-duplication are the verb's own. Result `{"content":[{"text","type":"text"}],"isError"}`: exit 0 → the verb's stdout, `isError` false; else its stderr (the verb's error line), `isError` true. The session: the call's `session`, else `$CLAUDE_CODE_SESSION_ID` when it is a session id (Claude Code sets it in a stdio server's env, measured 2026-11-13 · 2.1.325), else one minted from `/dev/urandom` for the process. Errors, `{"error":{"code","message"},"id","jsonrpc":"2.0"}`: a line that is not a JSON object `-32700` `Parse error` (`id` null); an unknown tool `-32602` `Unknown tool: <name>`; any other method `-32601` `Method not found`. A line without `id` is a notification, unanswered. EOF on stdin exits 0. Register (the owner's step): `claude mcp add --scope user sandman-next -- ~/.local/bin/sandman-next mcp`.

### `sandman serve`

The daemon, a foreground process under launchd. Start: a socket path over 103 bytes, a `run/` not this user's or reachable by group or other, or a held lock is one stderr line and exit 1. Otherwise it creates `run/` 0700, holds an exclusive `flock` on `run/serve.lock` for life, binds `run/serve.new` and renames it over `run/serve.sock` (the lock proves a stale socket dead), writes `run/serve.pid`, logs its start and prints its health line on stdout: ready.

One newline-terminated JSON request and one reply per connection, compact, keys alphabetical; a connection silent for 1 s is dropped unanswered:

```
{"op":"health"}   → {"machine":str,"ok":true,"pid":int,"since":"<RFC 3339 start>","state":"booting"|"ready"|"stopping","version":"<release>-<build>"}
{"op":"shutdown"} → {"ok":true}, written after the listener is dropped
anything else     → {"code":"bad_request"|"unknown_op","ok":false}
```

`state` is `ready` whenever it answers today: state is derived before the socket is bound, and a stopping daemon no longer accepts. SIGTERM, SIGINT and SIGHUP take the shutdown path; SIGPIPE is ignored (Rust's default). Exit 0 after removing `serve.pid` and `serve.sock` while the pid file still holds its pid. The drain budget is 2 s (`ExitTimeOut`): one connection in hand, at most its 1 s read, and the tick's job in hand (a running model command's group killed); the tick starts no other.

The tick, a thread: every `$SANDMAN_TICK_MS` (30 000) it re-reads the ledger and runs six jobs, in order; the drain sets the stop flag, wakes the tick and joins it, so the action in hand finishes, each job checks the flag before its next, and the tick leaves its loop before the exit (an exited loop is also what llvm-cov's counts of the loop body assume). **Reconcile**, per session whose earliest start mark has this machine's id, whose latest start mark is later than its latest end (or it has none) and older than `$SANDMAN_GRACE_MS` (600 000; a transcript may not exist yet), and that no job names: no transcript → `take`'s end with reason `no end recorded` (`gone`: a crash, a kill, or a session before `take`), at most 20 a tick; a transcript unmodified for longer than `$SANDMAN_IDLE_MS` (604 800 000, a week) → the same end with reason `idle` (`taken`: an abandoned session's memory is kept), at most one a tick; else it is left, maybe alive. A session resumed later gets a new start mark and is open again. **Embed**: `embeddings.jsonl` rewritten (temp + rename) to the live lines, one per hash of a current candidate's embedding text, once it is past twice their bytes; then the candidates whose text it lacks, in reader order, at most `$SANDMAN_EMBED_TICK` (128), embedded as recall embeds (16 a request, a 60 s bound, a stop kills it) and appended, logged as `{"embedded":<n>,"event":"embed"}` or `{"event":"embed","reason"}`. **Settle**, per session whose latest end has outcome `held` and this machine's id and no start mark is later (lines of other kinds never hide it), skips one a job still names or whose transcript was modified within `$SANDMAN_QUIET_MS` (120 000) and otherwise runs `take`'s end with reason `job gone` (`taken` or `gone`). Nothing is held in memory; a failed take stays held and is retried next tick. Then one of, a tick: a **reflect** rollback when due, else **encode** when an episode waits, else **consolidate**, else **reflect** when due (below). `run/serve.log` is the daemon's: one compact object per line, written whole in one `write`, keys alphabetical, `at` on each — `{"event":"start","pid","version"}`, `{"event":"exit","reason":"signal TERM"|"shutdown request"|<accept error>}`, `{"event":"embed",…}` (above), `{"event":"reconcile"|"take"|"encode"|"consolidate"|"reflect",…the ledger line}` or `{"event":…,"reason":<write error>,"session"|"bank"|"kind"}` per tick job, `{"event":"panic","reason"}` from the panic hook. A line that finds the log at `$SANDMAN_LOG_CAP` bytes (default 1 MiB) first renames it to `serve.log.1`.

### encode

A job of `serve`'s tick, not a verb. Per tick, after settling: of the sessions whose latest end has outcome `taken` and this machine's id, with no start mark or encode line later (a `recall` or `show` naming it never hides it), the one whose end is earliest: an encoded episode is never picked again. One line through `mark`'s writer, logged to `serve.log` as `event` `encode`:

```
{"at","error"?,"ids"?:[<each trace's id, in reply order>],"kind":"encode","machine","outcome":"encoded"|"skipped"|"gone"|"failed","session","token"?,"traces":<count>}
```

`skipped`: no start mark of the session has `interactive` true. `gone`: no file at the end line's `archived`. Else the digest: `# episode` (`date:` the end's `at`, `cwd:` as the pointer's, else `unknown`), `# already remembered` (`- <name> — <description>` per memory, then trace, whose `session` is the episode's and whose `source` begins `remember`; absent without), `# highlights` (`- <body>` per take highlight whose body differs from every such note's body one-lined to 4,000; absent without), `# recalled` (`- <path> · <name> — <description, 160> · shown|not shown` per note on disk, reader order, that a `recall` line of the session sent, by `file` or else `id`, and no `rethink` line of the session names the same way; `shown` when a `show` line of the session does; absent without), `# transcript` (per user or assistant message part: `user: <text>`, `assistant: <text>`, `tool: <name> <command|file_path>` one-lined to 200; tool results and other lines dropped); over `$SANDMAN_DIGEST_BYTES` (300 000) the head and tail halves, cut back to whole lines, around `… <n> bytes omitted …`.

The model run, shared with consolidate and reflect (its wait and its reply parse shared with `recall`): `$SANDMAN_ENCODER` else `claude -p --model opus --output-format text --safe-mode --tools "" --no-session-persistence`, as `sh -c`, own process group, env `CLAUDE_MEMORY_PIPELINE=1` `SANDMAN_NO_TAKE=1` and, from the token file, `CLAUDE_CODE_OAUTH_TOKEN`, stdin `run/<job>.in` = `<guidance>\n<contract>\n<data>`, stdout and stderr to `run/<job>.out`, `.err`. The token file, `$SANDMAN_CLAUDE_TOKEN_FILE` else `$HOME/Codes/claude-oauth-token`, is read at each run (a replaced file needs no restart): opened, it must be a regular file with no group or other bits, its trimmed text the token, set for the command alone (`--safe-mode` leaves auth alone, `--bare` would ignore it; verified 2026-11-14 · `claude --help`, code.claude.com/docs/en/authentication). The job's line notes it as `token`, never its value: absent without the file (the inherited env and the stored sign-in, as before); the path when used; `refused: <path>: mode <octal>, want a regular file of mode 600` (or the open error) and `ignored: <path> is empty`, the run going ahead without it. The owner's step, once a year (the token's life): `claude setup-token` in a terminal prints the token and stores nothing; then `install -m 600 /dev/stdin ~/Codes/claude-oauth-token`, paste, Ctrl-D. The guidance is `<root>/prompts/<job>.md` when it holds any non-blank text, else the embedded `src/prompts/<job>.md`; the contract, `src/prompts/<job>.contract.md`, is embedded and always sent; only reflect writes `prompts/` (below). A stop signal fails the run, the group killed. A replacement keeps: exit 0 and stdout holding one object, read from its first `{` to its last `}`; encode's is `{"traces":[{"type":"user"|"feedback"|"project"|"reference"|"knowledge"|"playbook","name":str,"description":str,"body":str,"about":str|null?,"topic":str|null?}],"verdicts"?:[{"file":str,"verdict":"irrelevant"|"stale"|"used"|"wrong","why":str|null?}]}`, `topic` a slug as `remember` takes it, required for `knowledge` and `playbook` and ignored for the rest; each verdict's `file` one `# recalled` lists, at most once. `failed` (`error` one line, 400 chars, no file written): `exit status: <n>: <stderr>`, `timed out after <ms> ms` (`$SANDMAN_ENCODE_MS`, 300 000; the group killed), `output: <parse error>`, `output: not a trace: <it>`, `output: not a verdict: <it>` (`verdicts` neither absent, null nor a list, or a verdict off that shape). Else `encoded`, one file per trace: `<root>/traces/<bank>/<type>_<slug>.md` by temp + rename, `<bank>` `@<topic>` for a knowledge type (the `@` no bank key holds; docs/knowledge.html#store), else the earlier sandman's `Bank::key_for(project::root(dir))` (`project`, `common`), `dir` the trace's `about` when it is an absolute path to an existing directory, else the episode's cwd, else `$HOME`; `<slug>` its `slug::slug` (lowercase, non-alphanumeric runs `_`, trimmed, 60 chars), `_2`… on a clash. `about:` and `topic:` are written when given:

```
---
id: <12 hex, minted>
name: <one-lined, 200>
description: <one-lined, 400>
type: <type>
created: <now>
about: <one-lined, 400>?
topic: <slug>?
session: <uuid>
archived: <path>
source: encode <now> · <uuid> · model=<the word after --model in the command, else its first word>
---

<body, trimmed>
```

Then, per verdict in reply order, `rethink`'s line and example (`### sandman rethink`), `by` `encode`, `session` the episode's, `why` the verdict's; then the encode line. A stop signal or shutdown mid-run kills the group and writes no line: the episode runs again after the next start. `failed` is final for the tick; removing its line retries it.

### consolidate

A job of `serve`'s tick, run only when no episode waits (never in the tick of an encode), at most one bank a tick, everything derived from disk. A bank `@<topic>` is a topic's: everything below runs alike with `knowledge/<topic>/` in place of `memories/<bank>/` (its memories are the topic's pages), and its keeps take `knowledge` or `playbook`, a bank's the four memory types. A memory or page is **judged** while a `rethink` line naming it (by `file`, else `id`) has verdict `wrong` or `stale` and `at` after its `updated`, else `created` (chart's `judged` term). Candidates: each bank holding a trace file under `<root>/traces/<bank>/` (the `notes` reader) or a judged memory or page, whose newest trace `created` and judged verdict `at` are at or before now − `$SANDMAN_CONSOLIDATE_QUIET_MS` (600 000), with a trace newer than the bank's latest `failed` consolidate line or a judged verdict newer than its latest consolidate line of either outcome. Of those, the bank whose earliest such time is earliest. Data (after the guidance and the contract, as above, job `consolidate`, `$SANDMAN_CONSOLIDATOR`, `$SANDMAN_CONSOLIDATE_MS` 600 000):

```
# bank
<bank>

# traces
<trace file="<file>">
<the whole file, trailing whitespace trimmed>
</trace>
…
# memories                                   (absent without any)
<memory file="<file>">…</memory>             (memories/<bank>/ by file, while their bytes fit $SANDMAN_CONSOLIDATE_BYTES, 300 000)

# listed                                     (absent when all fit)
- <file> · <name> · <description>            (the rest)

# judged                                     (absent without any)
- <file> · wrong|stale · <why one-lined to 400, else ->   (per judged verdict, memories by file)
```

Reply `{"ops":[…]}`, each op `{"op":"keep","trace","type","name","description","body"}`, `{"op":"merge","trace","memory","description","body"}`, `{"op":"merge","memories":[<two or more files>],"type","name","description","body"}`, `{"op":"revise","memory","description","body"}`, `{"op":"retitle","memory","name","description"}`, `{"op":"prune","trace","reason"}` or `{"op":"prune","memory","reason"}` (string values; of `trace`, `memory` and `memories` exactly those shown). Rejected whole, `failed`, nothing written: `output: not an op: <it>` when `ops` is not an array, an op is none of these (a keep or a merge of memories with a type its bank does not take, a prune naming both or neither, a revise of a memory not judged), a file is not in the input, a listed memory is named, or a trace or memory (a parent included) is named twice; `output: trace not named: <file>`; and the model run's errors. Applied, in this order: every write staged as `.<file>.<pid>` beside its target (a failure removes them all and fails); then each named trace and each named memory renamed to `<root>/.pruned/<yyyy>/<mm>/<dd>/<HHMMSS>-<bank>-<file>` (UTC; `<HHMMSS>-<n>-…` on a clash); then the staged writes renamed in. The writes: `.pruned/…/<HHMMSS>-<bank>-ops.json`, the reply, compact; per keep `memories/<bank>/<type>_<slug>.md` (encode's slug and clash) with `id` (minted when it has none), `created`, `about`, `topic`, `session`, `archived` from the trace and `name`, `description`, `type` from the op, in encode's order, `source: consolidate <now> · <the trace's session> · model=<model>`; per merge of a trace, and per revise, the memory rewritten: its frontmatter kept in order, its `id` first (minted when it has none), `description` and `source` (that stamp, `-` for the session without a trace) replaced and `updated: <now>` set (appended when absent), the op's body; per retitle the same without the stamp or `updated`, `name` and `description` replaced, its body kept, at keep's slug for the new name. Each is the note continuing (docs/index.html#cycle-note): before `source:`, `from:` the union of the inputs' `from:` ids, `origin:` the memory's, else the trace's, only when there is neither `from:` nor `sessions:`, and `sessions:` the union of the inputs' `sessions` (chart's: so a merge keeps the stamp it replaces), each absent when empty. Per merge of memories a new note at keep's slug: `id` minted, `name`, `description`, `type`, `created: <now>`, `topic` (a topic's), `from:` the parents' ids sorted (one minted for a parent without), the stamp; no `sessions`, `session` or `archived`. Once applied, one `rewrite` line per op through `mark`'s writer (a failed write ignored), then the consolidate line. `from` each input as it stood, the trace, then the memory or the parents in order: `digest` the SHA-256 hex of its file, `id` (null without one), `path` what `show` takes, `sessions` chart's; `to` where it went, `{"id","path"}` of the note written, null for a prune; `reason` the prune's, else null; `model` the consolidator's. A path in an older line (recall's `sent`, `show`) resolves through these lines' paths.

```
{"at","from":[{"digest","id","path","sessions"}],"kind":"rewrite","machine","model","op":"keep"|"merge"|"prune"|"retitle"|"revise","reason":str|null,"to":{"id","path"}|null}
{"at","bank","error"?,"kind":"consolidate","kept":n,"machine","merged":n,"order"?:{"after":{"chars","notes"},"before":{"chars","notes"},"merges":n},"outcome":"consolidated"|"failed","pruned":n,"token"?}
```

`merged` counts both merges. `order`, on a consolidated line (docs/index.html#cycle-rewrite): the bank's active notes, its traces and its memories (a topic's pages), before the model run and after the pass, `notes` their count and `chars` their files' characters; `merges` the merges of memories. No pass is reverted for it yet: every order op ends more notes than it writes, so only the recall evaluation's score (stage 10) can fail one.

A stop mid-run kills the group and writes no line. `MEMORY.md` is left alone: the earlier sandman's recall reads memory files, never the index, and its commit path regenerates the index whole.

### reflect

A job of `serve`'s tick, the weekly review, everything derived from disk. With `L` this machine's latest `reflect` line and `after` the ledger lines following it, a **model run** is a line of this machine with `kind` `consolidate`, or `encode` with outcome `encoded` or `failed`.

- **rollback** — first of the tick's last jobs: when `L` has outcome `adopted` and, for a job in its `history`, the first two model runs of that job in `after` both have outcome `failed`, the file `L.history[job]` is put in force as an adoption is (below), outcome `reverted`, `reason` `its next two runs failed`. Any later reflect line ends the watch.
- **due** — only in a tick with no rollback, no episode waiting and no quiet bank: `L` absent or its `at` at or before now − `$SANDMAN_REFLECT_MS` (604 800 000), and at least `$SANDMAN_REFLECT_MIN` (5) model runs in `after`. Else nothing, no line.
- **input** — the model run (`job` `reflect`, `$SANDMAN_REFLECTOR`, `$SANDMAN_REFLECTOR_MS` 600 000), data `<guidance job="encode">\n<in force, trimmed>\n</guidance>\n<guidance job="consolidate">…</guidance>\n\n# since\n<L's at, else the start>\n\n`, then encode's sections (`# <title>`, `- <item>` lines, a blank line, absent when empty), cut as encode's digest past `$SANDMAN_REFLECT_BYTES` (300 000): `runs` each model run's line, compact; `ops` `<bank>: <op without body>` per op of each `.pruned/**/<HHMMSS>-<bank>-ops.json` whose mtime is after `L`'s `at`; `merged again` `<bank>/<memory> · merged <n> times` for a memory those ops merge more than once; `remembered by hand` `<name> — <description> (<file>)` per `remember` line of this machine in `after`, the trace read at its `file`, else the `.pruned/` file ending `-<bank>-<file>`; `sent, never read` `<file> · sent <n> times, never shown` per file sent by this machine's `recall` lines in `after` at least 3 times, none of them in a session with a `show` line of that file in `after`.
- **reply** — `{"encode":str|null,"consolidate":str|null,"reason":str}`. `failed`: the model run's errors, `output: not a proposal: <it>` for another shape, `output: <job> guidance over <n> bytes` past `$SANDMAN_GUIDANCE_BYTES` (8 000).
- **replay** — per job with a string, `consolidate` then `encode`, the latest `$SANDMAN_REFLECT_REPLAY` (3) inputs. Encode's: per `encode` line of this machine with outcome `encoded`, newest first, whose session's latest end has a file at `archived`, encode's digest of that end, rebuilt. Consolidate's: per `.pruned/**/<HHMMSS>-<bank>-ops.json`, newest path first, consolidate's input for `<bank>` (memories within `$SANDMAN_CONSOLIDATE_BYTES`) over the traces its ops name and the memories they name by `memory` (a merge's parents are not rebuilt), read beside it as `<HHMMSS>-<bank>-<file>`, then `memories/<bank>/`'s (or the topic's `knowledge/<topic>/`) other memories, less any whose `created` and `session` equal a consumed trace's; by file. Each input runs through the job's model run twice, guidance in force then the new one (`run/<job>.*`, nothing else written), then the model run `judge` (`$SANDMAN_REFLECTOR`, `$SANDMAN_REFLECTOR_MS`) on `# job\n<job>\n\n# input\n<input>\n# a\n<reply>\n\n# b\n<reply>\n`, the two replies' order from `/dev/urandom`; its reply `{"better":"a"|"b"|"same"}`, else `failed` `output: not a verdict: <it>`. Adopted when an input was replayed and the guidance in force won none.
- **adopt** — per adopted job: the guidance in force (the embedded default too) written to `dated(<root>, "prompts/.history", "<job>.md")`, then the new one to `<root>/prompts/<job>.md`, both by temp + rename. reflect never writes `prompts/reflect.md`, `prompts/judge.md` or a contract.

One line through `mark`'s writer, logged to `serve.log` as `event` `reflect`; `history` maps each job put in force to the root-relative copy of the version it replaced; `outcome` `adopted` when any was, else `kept`; `replay` per proposed job, all 0 when nothing was replayed. A stop mid-run kills the group and writes no line. A review costs 1 model run plus 3 per input replayed.

```
{"at","error"?,"history":{"<job>":"prompts/.history/<yyyy>/<mm>/<dd>/<HHMMSS>-<job>.md"},"kind":"reflect","machine","outcome":"adopted"|"kept"|"failed"|"reverted","reason"?,"replay"?:{"<job>":{"new":n,"old":n,"same":n}},"token"?}
```

### `sandman setup`

The plist for `dev.gigantic.sandman`: `ProgramArguments` `~/.local/bin/sandman` (hotswap's link, never this executable: a checkout's setup wires the installed release) and `serve`, `EnvironmentVariables` the caller's `PATH` (launchd's finds no `claude`) and the root it ran with, `KeepAlive`, `RunAtLoad`, `ThrottleInterval` 10, `ExitTimeOut` 2, `StandardErrorPath` `<root>/serve.err` for failures before the log (outside `run/`, which launchd would otherwise create 0744). `setup --print` prints it and writes nothing. `setup` installs it, `launchctl` resolved from `PATH` (none, as on Linux: `sandman setup: launchctl: <error> (setup installs a launchd agent, macOS only; --print prints the plist)`, exit 1, before anything is written), `gui/<id -u>` the domain, each launchctl call that changes something printed as `launchctl <args>`:

1. the old sandman's agents, `dev.gigantic.sandman.breathe`, `.reflect`, `.report` (retired 2026-11-14): each loaded one booted out, then each plist removed (`removed <path>`); none is ever written;
2. `hotswap::ensure("sandman", manifest)` (gigantic.dev/hotswap/): a mise install of a newer version than the active one moved into `~/.local/share/hotswap/sandman/<version>/`, else, with none active, the latest release installed by packslip under signer `ps1_nensfiggtc7n6uctbniwf7q4am`; `~/.local/bin/sandman` linked through its `current` (`linked <link> → <its target>`); and the upgrade agent `dev.gigantic.sandman.upgrade` (`~/.local/bin/sandman upgrade` every 900 s), hotswap's alone: setup never writes, loads or boots it out. Its failure: `sandman setup: <hotswap's line>`, exit 1;
3. `~/Library/LaunchAgents/dev.gigantic.sandman.plist` by temp + rename (`wrote <path>`);
4. the same plist already loaded: `kickstart -k`, the swap; else booted out when loaded, then `bootstrap`.

A bootout returns before launchd lets go of the job, and a bootstrap before then fails, so after each bootout `launchctl print` is polled every 100 ms until it fails, at most `$SANDMAN_BOOTOUT_MS` (30 000, past a default `ExitTimeOut` of 20 s): `<label> still loaded <ms> ms after its bootout`, exit 1. A failed call: `sandman setup: launchctl <args>: <its stderr>`, exit 1. `setup --remove` first has `hotswap::forget` drop the upgrade agent, the layout and the record (refused while a swap runs), then boots `dev.gigantic.sandman` out and removes its plist and `~/.local/bin/sandman` (`removed <path>`), leaving the root alone. Idempotent. Before the rewrite's first release, `releases/latest` is the old sandman's: do not run setup on a host until it ships.

### `sandman upgrade`

`sandman upgrade [--check | --back | --retry]`: `hotswap::upgrade("sandman", manifest, args)`, its code the exit code (0 green or settled, 1 red, 2 unable to try; another argument `usage: sandman upgrade [--check | --back | --retry]`, 2). The manifest: repo `jgeschwendt/sandman`, signer `ps1_nensfiggtc7n6uctbniwf7q4am` (packslip's fingerprint of the repository id, verified 2026-11-17 against v2026.11.1's bundle with `packslip pin`), bin `sandman`, setup `sandman setup`, agents `dev.gigantic.sandman` (one `kickstart -k`, whose SIGTERM is the daemon's 2 s drain, so no drain step), smoke `sandman version` then `/bin/sh -c` polling `sandman status` once a second, 30 tries, until its health's `version` is `sandman version`'s; hotswap's default deadlines. Releases: `.github/workflows/release.yaml`, one per merge to main, `YYYY.M.N` (`SANDMAN_VERSION`, which `build.rs` stamps), `aarch64-apple-darwin` and `x86_64-unknown-linux-gnu`, packslip-signed. `mise run install` still builds a checkout to `~/.local/bin/sandman-next`, for its verbs; setup never wires it.

### `sandman status`

Sends `{"op":"health"}` and prints the daemon's reply on stdout, exit 0; when its `version` is not this client's, one stderr line names both and the fix, `<me> setup`. No socket, a refused connect, or no `"ok":true` reply within 1 s: `{"code":"down","ok":false}`, exit 1. No retries.

### `sandman version`

Stdout `<release>-<build>` (`build.rs`: the release workflow's `SANDMAN_VERSION`, else the crate version; then the short commit, `-dirty` over uncommitted work, else `unknown`), the daemon health's `version`; exit 0. An installer's smoke check.
