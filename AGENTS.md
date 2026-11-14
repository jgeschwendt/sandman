# sandman

```stele
kind: system
purpose: sandman, rewritten from scratch one part at a time — `mark` appends a session's start or end to the day's ledger file, `take` ends a marked session (archiving its transcript) with one more, `chart` prints what the data root holds as one JSON object, `recall` seeds a session's start and serves the agent's pulls: a lexical gate, a BM25 prefilter, then Jev, else local Kev, else the lexical rank, `show` prints one, `remember` writes a trace at once, `serve` is the daemon under launchd, whose tick ends sessions that never ended, settles held takes, encodes episodes into traces and consolidates a bank's traces into the bank, unattended, `status` asks it for its health, `setup` prints its plist. The old sandman stays live at ~/.local/bin/sandman; this build installs as sandman-next.
commands:
  check: mise run check
  cover: mise run cover
  install: mise run install
  test: cargo test
```

## Map

| path                     | what                                                                                       |
| ------------------------ | ------------------------------------------------------------------------------------------ |
| `src/main.rs`            | the whole binary: the root, the machine id, `mark`, `take`, `chart`, `recall`, `remember`, `serve` and its tick (reconcile, settle, encode, consolidate), `setup`, `show`, `status` |
| `src/prompts/<job>.md`   | a tick job's default guidance, `<job>.contract.md` its fixed contract; both embedded with `include_str!` |
| `tests/sandman.rs`       | end-to-end suite: the built binary over temp roots and Claude homes via `SANDMAN_ROOT`, `SANDMAN_MACHINE`, `CLAUDE_CONFIG_DIR`, stub shell model commands via `SANDMAN_ENCODER` and `SANDMAN_CONSOLIDATOR` (`exit 9` unless a test names one) and stub System One transports via `SANDMAN_SYSTEMONE` and `SANDMAN_KEV` (`exit 9` unless a test names one; `TYPESAFE_API_KEY` removed; the default transports only against a stub `curl` first on `PATH`); a `Daemon` guard kills every `serve` it starts (SIGTERM one whose coverage counts: a SIGKILL writes no profile) |
| `docs/index.html`        | the page: data root, `mark`, `take`, `chart`, `serve`, `encode`, `consolidate`, next; two d3 charts and the daemon's lifecycle figure |
| `docs/recall.html`       | `recall`'s design page: the problem, the flow (the seed and the agent's pull), one section per decision, the agent's two tools, the pane; the reach chart and the flow figure; what is built marked so |
| `docs/charts/*.js`       | one ES module per chart over `lib.js`'s chrome and `chart.css`, both pages' chart styles; d3 7.9.0 is the page's global |
| `docs/data/chart.json`   | the timeline's sample: `tests/sandman.rs` `chart` fails unless it is the verb's output     |
| `docs/data/recall-reach.json` | the reach chart's counts per bank, banks unnamed, from `sandman-next chart` (2026-11-13) |
| `docs/data/recall-*.txt` | the recall page's samples: `tests/sandman.rs` `recall` fails unless `-seed` is the seed's stdout and `-digest` a pull's |
| `docs/data/flow.json`    | the data-flow chart's moments, nodes and edges, ①–④ marking the order a session passes them; `next` nodes are not built yet |
| `Cargo.toml`             | the rust base's `lints.toml` verbatim; the one opt-out and `signal-hook` carry their reasons |

The gate is `mise run check` (fmt, clippy `-D warnings`, `cover` at 100 % of lines), run by hk's pre-push (`hk install --mise` once per clone) and by CI. `lints check` from jgeschwendt/rust holds `Cargo.toml` to the base. Tests never touch `~/.sandman`; a child keeps the parent env (llvm-cov's `LLVM_PROFILE_FILE`), with `HOME`, `CLAUDE_*`, `SANDMAN_*` and `TYPESAFE_API_KEY` replaced; no test reaches a model API.

## Contracts

Root: `$SANDMAN_ROOT`, else `$HOME/.sandman`. Machine id: `$SANDMAN_MACHINE`, else `<root>/machine` (one line, 32 lowercase hex from `/dev/urandom`), minted on first use by a hard link from a temp file so racing first runs agree. `sandman` with no or another verb: six-line usage on stderr, exit 2. `--session` anywhere must be a session id (as `mark` checks), else one stderr line, exit 2. A command printed for an agent to run (the seed's instructions, the footer, `remember`'s line) names `<me>`, the file name the binary was invoked as, so `sandman-next` prints `sandman-next show …`; usage and error prefixes say `sandman`.

### `sandman mark`

Stdin, one object; `kind` and `session` (36 chars, hex and `-`) required, anything else missing is null:

```
{"kind":"start"|"end","session":"<uuid>","cwd":"<path>"?,"surface":"<str>"?,"interactive":bool?,"reason":"<str>"?,"resume":"<str>"?}
```

Appends with `O_APPEND` to `<root>/ledger/<yyyy>/<mm>/<dd>.jsonl` (UTC), compact, keys alphabetical:

```
{"at":"<RFC 3339 UTC, seconds, Z>","cwd":…,"interactive":…,"kind":…,"machine":…,"reason":…,"resume":…,"session":…,"surface":…}
```

Exit 0 silent; 2 on bad stdin, one stderr line, nothing written; 1 when the line cannot be written. Runs inside the subether mod's 1.5 s budget.

### `sandman take`

The end of a session, inside the mod's 1.5 s budget; it spawns nothing. Stdin, one object, `session` checked as in `mark`:

```
{"session":"<uuid>","cwd":"<path>"?,"reason":"<str>"?}
```

The first that holds: no start mark for the session in the ledger, or `reason` `resume` → nothing written. Else one end line through `mark`'s writer, `outcome`: `skipped` when `$SANDMAN_NO_TAKE` is non-empty or `CLAUDE_MEMORY_PIPELINE=1`; `held` (plus `job`, the dir name) when `reason` is not `clear` and a `<claude>/jobs/*/state.json` names the session by `resumeSessionId`, else `sessionId`; `gone` without `<claude>/projects/*/<session>.jsonl`; else `taken` (plus `archived`). The tick's reconcile and settle end a session through the same path, reasons `no end recorded`, `idle` and `job gone`. `<claude>` = `$CLAUDE_CONFIG_DIR`, else `$HOME/.claude`. The pointer below stays until `recall` replaces the earlier one.

```
{"archived"?,"at","cwd","job"?,"kind":"end","machine","outcome","reason","session"}
```

The take, byte-compatible with the earlier sandman's: `fs::rename` the transcript to `<root>/.archive/claude/<yyyy>/<mm>/<dd>/<HHMMSS>-<path under <claude>, / → ->` (UTC take time; `<HHMMSS>-<n>-…` on a same-second clash) and `<project>/<session>/`, when present, to that stem + `-dir`; then `<root>/memories/.recent/<session>.json` by temp + rename, compact, newline-terminated, in this key order: `{"archived","cwd","ended","title","highlights"}`. `ended` the transcript's mtime; `cwd` the first line's `cwd` without control characters, else stdin's, else null; `title` the first `type:"user"` line's string content or first `text` block, whitespace collapsed, 80 chars, else the session id; `highlights` the quoted bodies (`"…"` with `\"` `\\` unescaped, or `\"…\"`) after `sandman remember ` then `orrery attend ` in assistant `tool_use.input.command`s and user `<bash-input>` lines, one-lined, 4,000 chars, a repeat moved to the end, the last 20. Rename, pointer, ledger line, in that order. Exit 0 silent when decided; 2 on bad stdin, nothing written; 1 when a rename or write fails (cross-device included), one stderr line, no ledger line.

### `sandman chart`

Stdout, one compact object, keys alphabetical; reads the root, writes nothing but a missing `machine`; every absence is an empty list or a skipped item; a closed stdout ends it quietly (exit 0):

```
{"active":[{"cwd":str|null,"encoded":str|null,"end":str|null,"interactive":bool|null,"machine":str,"outcome":str|null,"seen":str|null,"session":str,"start":str|null}],
 "at":"<now, RFC 3339 UTC>",
 "machine":"<this machine's id>",
 "memories":[{"bank":str,"created":str|null,"file":str,"name":str,"session":str|null,"source":"consolidate"|"dream"|"reflect"|"remember"|"other","type":str}],
 "taken":[{"bytes":int,"cwd":str|null,"dreamed":str|null,"highlights":int,"memories":int,"session":str,"state":"waiting"|"dreamed"|"archived","taken":str,"title":str|null}],
 "traces":[{"bank":str,"created":str|null,"file":str,"name":str,"session":str|null,"type":str}]}
```

- `active` — per session in `ledger/*/*/*.jsonl`: `start` the `at` of its earliest start mark (a reload re-fires it), `end` and `outcome` of its latest end mark (null on an end line without one), both null when a start mark is later (resumed: running again); `cwd`, `machine` from that start mark, else the end mark; `encoded` the `outcome` of its latest encode line; `interactive` true when a start mark says true, else false when one says false, else null; `seen` the mtime of `<claude>/projects/*/<session>.jsonl` (`take`'s finder), null without one. By `start` (null first), then session.
- `taken` — per file `<root>/.archive/claude/<yyyy>/<mm>/<dd>/<HHMMSS>-…-<uuid>.jsonl` directly in a day dir; the latest per session wins. `taken` = that date and time as UTC, `bytes` = size; `title`, `cwd`, `highlights` (count, 0 without), `dreamed` from `<root>/memories/.recent/<session>.json` when it parses as an object. `state`: `waiting` pointer without `dreamed`, `dreamed` with it, `archived` no pointer. `memories` = memory entries naming the session. By `taken`, then session.
- `memories` — per `<root>/memories/<bank>/<file>.md`, banks and files starting `.` or `_` and `MEMORY.md` skipped. Frontmatter: `key: value` lines between `---` fences; `name` (else the stem), `type` (else ""), `created`, `source` → `consolidate`/`dream`/`reflect`/`remember` by its first word, else `other`; `session` the uuid in a consolidate's or a dream's (`dream <ISO> · <uuid> · minds=…`). By bank, then file.
- `traces` — per `<root>/traces/<bank>/<file>.md`, the same reader and skips; `session` from the frontmatter. By bank, then file.

### `sandman recall`

Two entries, one pass (docs/recall.html#tool). The **pull**, the agent's, any time: `sandman recall --cue "<text>" [--about <path>] [--session <uuid>] [--budget <chars>]`. The **seed**, the mod's at a session's first prompt: `sandman recall` with no `--cue` reads one object on stdin, `session` checked as in `mark` (else one stderr line, exit 2, nothing written):

```
{"session":"<uuid>","cwd":"<path>"?,"surface":"<str>"?,"interactive":bool?,"prompt":"<the first prompt's text>"?}
```

The pass, in order, every constant the evaluation's choice (docs/recall.html#chosen):

- **set** — every note the `notes` reader accepts under `<root>/memories/`, then under `<root>/traces/` (a trace is a candidate the moment it is written; its path, below, is `traces/<bank>/<file>`). Frontmatter `recall: mute`: nothing. `type: user` or `recall: pin`: the always-set, unscored, never sent, printed by the seed only. The rest are the candidates, in reader order (bank, then file); `considered` counts them all.
- **repository** — a bank's label: its key equal to `key(<home>)` → `home (~)`, to `key(<grove>)` → `grove root (every repository)`; equal to or extending (`-` after) `key(<grove>/<owner>/<repo>)`, the longest first → `<owner>/<repo>` (a worktree's bank folds into its repository's); equal to `key(<grove>/<owner>)` → `<owner> (owner: all its repositories)`; extending an owner's → `<owner>/<the rest>` (a repository gone from disk); extending `key(<claude>/jobs)` → `scratch dir`; else the key read back as a path, `--` as `/.` and `-` as `/`, `~` for the home prefix. `key` is the encode's bank key, so it is lossy: a label is resolved by listing `<grove>`'s owner and repository dirs (hidden ones skipped) and comparing keys, and a bank matching none reads as the path guess. `<grove>` = `$SANDMAN_GROVE`, else `$HOME/.grove/code`.
- **about** — given: `--about`'s path, else the seed's `cwd` when it lies at least two levels under `<grove>`, as the label of `key(project(it))`; given, it leads the state. Else the first repository label, longest key first (ties by path), whose `<repo>` lowercased is one of the cue's lowercase `[a-z0-9.]` runs; inferred, it feeds the prefilter only.
- **tokens** — lowercase ASCII alphanumeric runs, a one-character run or a `STOPWORDS` word dropped, a final `s` dropped past four characters.
- **gate** — no token, or every token in `STEER`: no ask, no model, no ranked line.
- **prefilter** — BM25 (k1 1.2, b 0.75; idf ln(1 + (n − df + ½) ÷ (df + ½))) of each candidate's `<name> <description> <label> <file without .md> <body>` tokens for the distinct tokens of `<cue> <about>`, plus 4 in the about's bank, else 1 in home's or the grove root's; descending, a tie in reader order; the top `$SANDMAN_RECALL_N` (39), the seed's `$SANDMAN_SEED_N` (20), kept. A kept candidate's lexical score is its score ÷ the top one (÷ 1 when that is 0).
- **scorers** — tried in `SCORERS` order, the first to answer wins: `jev`, then `kev`; while `<root>/jev-down.json`'s `until` (ms since the epoch) is ahead, `kev` alone; `$SANDMAN_SCORER` `jev`, `kev` or `none` forces one or neither. One request each, `{"model":"<scorer>-latest","questions","state"}`, `state` `repository: <given about>\n<cue>`, else the cue; a question per kept candidate keyed by its rank: `{"instructions":{"memory":RECORD(memory),"question":QUESTION,"repository":<its label>},"type":"noul"}`, Jev's with `"criteria":{"false":CRITERIA[1],"true":CRITERIA[0]}`. `RECORD` is `<name> — <description one-lined to 160>`. A failure is `<scorer>: <cause>`; Jev's writes `{"error","until"}` to `jev-down.json`, `until` now + `$SANDMAN_JEV_HELD_MS` (21,600,000) when its stdout or stderr holds `billing_error`, `error: 401`, `error: 402` or `error: 403`, else + `$SANDMAN_JEV_DOWN_MS` (60,000). Neither answering: the lexical rank.
- **transport** — Jev `$SANDMAN_SYSTEMONE`, Kev `$SANDMAN_KEV`, each through `sh -c`, own process group, request JSON on stdin, response JSON on stdout, exit 0; killed when the pass's bound runs out: `$SANDMAN_RECALL_MS` (10,000) for a pull, `$SANDMAN_SEED_MS` (4,000) for the seed, from the pass's start, gate and prefilter included. Unset, `curl -q -sS --fail-with-body --max-time <bound> -H 'Content-Type: application/json'`: Kev's `--data-binary @- http://127.0.0.1:1703/v1/systemone`, no key; Jev's `--data-binary @/dev/fd/3 https://api.typesafe.ai/v1/systemone` with the key `$TYPESAFE_API_KEY` else `~/.config/typesafe/api_key` (trimmed), given to `sh` as `SANDMAN_SYSTEMONE_KEY`: a builtin `printf` pipes the `Authorization` header to curl as config on its stdin (`-K -`), the request goes on fd 3, and curl runs with that variable emptied, so the key is never in an argv; no key is the failure `no API key: set TYPESAFE_API_KEY or write it to ~/.config/typesafe/api_key`. Other failures: `<status>: <stderr>`, `timed out after <ms> ms`, `output: <parse error>`, `reply: no noul for question <rank>`.
- **cut** — per the winning scorer, `(w, floor, max)` from `SCORERS`, each overridable as `$SANDMAN_<JEV|KEV|LEXICAL>_<W|FLOOR|MAX>`: Jev (0.7, 0.3, 8), Kev (0.6, 0.4, 6), the lexical rank (0, 0, 5, the noul its lexical score). final = w · noul + (1 − w) · lexical; descending, a tie newer `updated`, else `created`, first; kept candidates at or above the floor not sent by an earlier recall of the session; a line is cut, and every one after it, past `max` lines or the budget.
- **pull's stdout** — within `--budget` characters (4,000): `recall · <cue, 60> · <sent> of <considered> · <jev|kev|lexical, unscored[: <failed scorers joined by " and "> failed]>` (the scorer part absent when nothing was kept), the ranked lines `.NN <mark><name> (<type> · <path>) — <description, 160>`, `.NN` the noul to two places without a leading 0, `<mark>` `~` for a trace (provisional until consolidated), else nothing, `<path>` what `show` takes; `read one in full: <me> show <path>`; no line: `nothing recalled` in place of the lines and the footer, `nothing recalled: no specific ask` when the gate closed. Exit 0 whatever the scorers did.
- **seed** — first, the start mark through `mark`'s line (`kind` `start`, `reason` and `resume` null), whatever follows. `CLAUDE_MEMORY_PIPELINE=1`: nothing else, exit 0. Else stdout within `$SANDMAN_SEED_BUDGET` (9,000) characters, the fixed parts and the always-set counted first: the instructions (`SEED`, `<id>` the session), a blank line, `always · about the person, or pinned`, the always-set `  · <mark><name> (<type> · <path>) — <description, 160>`; with candidates kept, the pull's header and ranked lines; the footer. Past its bound it prints the lexical rank, so a seed never fails the prompt.
- **stats** — stderr: `sandman recall: <error>` when a scorer failed, then `recall: scorer=<"…"|null> ask=<bool> about=<"…"|null> ms=<wall> tokens=<usage.input_tokens> considered=<n> prefilter=<n> sent=<n> cut=<n>`.
- **ledger** — through `mark`'s writer, a failed write ignored: `{"about"?:"<label>","always"?:["<path>",…],"ask":bool,"at","considered","cue":<one-lined, 200>,"cut","error"?:"<scorer>: <cause>[; …]","kind":"recall","machine","ms","prefilter":<kept>,"scorer":"jev"|"kev"|"lexical"|null,"seed"?:true,"sent":[{"file":"<path>","noul":<2 places>}],"session":<uuid>|null,"tokens"}`; `always` and `seed` on a seed's line, the always-set it printed; `about` given or inferred; `scorer` null when nothing was kept; `error` beside a non-empty `sent` when a later scorer answered.

### `sandman remember`

The agent's write: `sandman remember "<body>" [--type user|feedback|project|reference] [--name <name>] [--description <line>] [--about <path>] [--session <uuid>]`, the body first. An empty body or another type: `sandman remember: takes a non-empty "<body>" first and --type user|feedback|project|reference`, exit 2, nothing written. One trace at once, `<root>/traces/<bank>/<type>_<slug>.md` by temp + rename, encode's slug, clash and writer, `about` and `session` written when given:

```
---
name: <--name, else the body's first eight words; one-lined, 200>
description: <--description, else the body's first line one-lined to 160; one-lined, 400>
type: <--type, else feedback>
created: <now>
about: <--about, canonical when it is a directory>?
session: <uuid>?
source: remember <now> · <uuid, else ->
---

<body, trimmed>
```

`<bank>` is `key(project(dir))`, `dir` `--about` when it is a directory (a relative one from the cwd), else the cwd, else `$HOME`: without `--about`, a session started at the root lands in the root's bank. Stdout `<me> show traces/<bank>/<file>`; then `{"at","file":"traces/<bank>/<file>","kind":"remember","machine","session":<uuid>|null,"type"}` through `mark`'s writer (a failed write ignored). A failed trace write: `sandman remember: <error>`, exit 1, no ledger line.

### `sandman show`

`sandman show <path> [--session <uuid>]`, `<path>` `<bank>/<file>.md` for a memory, `traces/<bank>/<file>.md` for a trace: the note the `notes` reader finds at exactly that path (so no `..`, absolute path, hidden or `_` name, or `MEMORY.md` resolves) as `<name> (<type>[ · trace]) · <updated, else created, else undated>` and its body, frontmatter stripped and trimmed; then `{"at","file","kind":"show","machine","session"}` through `mark`'s writer (a failed write ignored), the signal that a recalled note was read. No such note: `sandman show: no memory <arg> (want [traces/]<bank>/<file>.md under <root>)`, exit 1.

### `sandman serve`

The daemon, a foreground process under launchd. Start: a socket path over 103 bytes, a `run/` not this user's or reachable by group or other, or a held lock is one stderr line and exit 1. Otherwise it creates `run/` 0700, holds an exclusive `flock` on `run/serve.lock` for life, binds `run/serve.new` and renames it over `run/serve.sock` (the lock proves a stale socket dead), writes `run/serve.pid`, logs its start and prints its health line on stdout: ready.

One newline-terminated JSON request and one reply per connection, compact, keys alphabetical; a connection silent for 1 s is dropped unanswered:

```
{"op":"health"}   → {"machine":str,"ok":true,"pid":int,"since":"<RFC 3339 start>","state":"booting"|"ready"|"stopping","version":"<crate version>"}
{"op":"shutdown"} → {"ok":true}, written after the listener is dropped
anything else     → {"code":"bad_request"|"unknown_op","ok":false}
```

`state` is `ready` whenever it answers today: state is derived before the socket is bound, and a stopping daemon no longer accepts. SIGTERM, SIGINT and SIGHUP take the shutdown path; SIGPIPE is ignored (Rust's default). Exit 0 after removing `serve.pid` and `serve.sock` while the pid file still holds its pid. The drain budget is 2 s (`ExitTimeOut`): one connection in hand, at most its 1 s read, and the tick's job in hand (a running model command's group killed); the tick starts no other.

The tick, a thread: every `$SANDMAN_TICK_MS` (30 000) it re-reads the ledger and runs four jobs, in order, each action under a lock the drain takes after setting the stop flag (the action in hand finishes, no other starts). **Reconcile**, per session whose earliest start mark has this machine's id, whose latest start mark is later than its latest end (or it has none) and older than `$SANDMAN_GRACE_MS` (600 000; a transcript may not exist yet), and that no job names: no transcript → `take`'s end with reason `no end recorded` (`gone`: a crash, a kill, or a session before `take`), at most 20 a tick; a transcript unmodified for longer than `$SANDMAN_IDLE_MS` (604 800 000, a week) → the same end with reason `idle` (`taken`: an abandoned session's memory is kept), at most one a tick; else it is left, maybe alive. A session resumed later gets a new start mark and is open again. **Settle**, per session whose latest end has outcome `held` and this machine's id and no start mark is later (lines of other kinds never hide it), skips one a job still names or whose transcript was modified within `$SANDMAN_QUIET_MS` (120 000) and otherwise runs `take`'s end with reason `job gone` (`taken` or `gone`). Nothing is held in memory; a failed take stays held and is retried next tick. Then **encode** when an episode waits, else **consolidate** (below), one a tick. `run/serve.log` is the daemon's: one compact object per line, written whole in one `write`, keys alphabetical, `at` on each — `{"event":"start","pid","version"}`, `{"event":"exit","reason":"signal TERM"|"shutdown request"|<accept error>}`, `{"event":"reconcile"|"take"|"encode"|"consolidate",…the ledger line}` or `{"event":…,"reason":<write error>,"session"|"bank"}` per tick job, `{"event":"panic","reason"}` from the panic hook. A line that finds the log at `$SANDMAN_LOG_CAP` bytes (default 1 MiB) first renames it to `serve.log.1`.

### encode

A job of `serve`'s tick, not a verb. Per tick, under the tick's lock, after settling: of the sessions whose latest end has outcome `taken` and this machine's id, with no start mark or encode line later (a `recall` or `show` naming it never hides it), the one whose end is earliest: an encoded episode is never picked again. One line through `mark`'s writer, logged to `serve.log` as `event` `encode`:

```
{"at","error"?,"kind":"encode","machine","outcome":"encoded"|"skipped"|"gone"|"failed","session","traces":<count>}
```

`skipped`: no start mark of the session has `interactive` true. `gone`: no file at the end line's `archived`. Else the digest: `# episode` (`date:` the end's `at`, `cwd:` as the pointer's, else `unknown`), `# already remembered` (`- <name> — <description>` per memory, then trace, whose `session` is the episode's and whose `source` begins `remember`; absent without), `# highlights` (`- <body>` per take highlight whose body differs from every such note's body one-lined to 4,000; absent without), `# transcript` (per user or assistant message part: `user: <text>`, `assistant: <text>`, `tool: <name> <command|file_path>` one-lined to 200; tool results and other lines dropped); over `$SANDMAN_DIGEST_BYTES` (300 000) the head and tail halves, cut back to whole lines, around `… <n> bytes omitted …`.

The model run, shared with consolidate (its wait and its reply parse shared with `recall`): `$SANDMAN_ENCODER` else `claude -p --model opus --output-format text --safe-mode --tools "" --no-session-persistence`, as `sh -c`, own process group, env `CLAUDE_MEMORY_PIPELINE=1` `SANDMAN_NO_TAKE=1`, stdin `run/<job>.in` = `<guidance>\n<contract>\n<data>`, stdout and stderr to `run/<job>.out`, `.err`. The guidance is `<root>/prompts/<job>.md` when it holds any non-blank text, else the embedded `src/prompts/<job>.md`; the contract, `src/prompts/<job>.contract.md`, is embedded and always sent; the daemon never writes `prompts/`. A replacement keeps: exit 0 and stdout holding one object, read from its first `{` to its last `}`; encode's is `{"traces":[{"type":"user"|"feedback"|"project"|"reference","name":str,"description":str,"body":str,"about":str|null?}]}`. `failed` (`error` one line, 400 chars, no file written): `exit status: <n>: <stderr>`, `timed out after <ms> ms` (`$SANDMAN_ENCODE_MS`, 300 000; the group killed), `output: <parse error>`, `output: not a trace: <it>`. Else `encoded`, one file per trace: `<root>/traces/<bank>/<type>_<slug>.md` by temp + rename, `<bank>` the earlier sandman's `Bank::key_for(project::root(dir))` (`project`, `common`), `dir` the trace's `about` when it is an absolute path to an existing directory, else the episode's cwd, else `$HOME`; `<slug>` its `slug::slug` (lowercase, non-alphanumeric runs `_`, trimmed, 60 chars), `_2`… on a clash. `about:` is written when given:

```
---
name: <one-lined, 200>
description: <one-lined, 400>
type: <type>
created: <now>
about: <one-lined, 400>?
session: <uuid>
archived: <path>
source: encode <now> · <uuid> · model=<the word after --model in the command, else its first word>
---

<body, trimmed>
```

A stop signal or shutdown mid-run kills the group and writes no line: the episode runs again after the next start. `failed` is final for the tick; removing its line retries it.

### consolidate

A job of `serve`'s tick, run only when no episode waits (never in the tick of an encode), at most one bank a tick, everything derived from disk. Candidates: each `<root>/traces/<bank>/` holding a trace file (the `notes` reader) whose newest `created` is at or before now − `$SANDMAN_CONSOLIDATE_QUIET_MS` (600 000), and, when the bank's latest `failed` consolidate line has `at` F, whose newest `created` is after F. Of those, the bank with the earliest `created`. Data (after the guidance and the contract, as above, job `consolidate`, `$SANDMAN_CONSOLIDATOR`, `$SANDMAN_CONSOLIDATE_MS` 600 000):

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
```

Reply `{"ops":[…]}`, each op `{"op":"keep","trace","type","name","description","body"}`, `{"op":"merge","trace","memory","description","body"}`, `{"op":"prune","trace","reason"}` or `{"op":"prune","memory","reason"}` (string values). Rejected whole, `failed`, nothing written: `output: not an op: <it>` when `ops` is not an array, an op is none of these (a keep naming a `memory`, a prune naming both or neither), a file is not in the input, a listed memory is named, or a trace or memory is named twice; `output: trace not named: <file>`; and the model run's errors. Applied, in this order: every write staged as `.<file>.<pid>` beside its target (a failure removes them all and fails); then each named trace and each merged or pruned memory renamed to `<root>/.pruned/<yyyy>/<mm>/<dd>/<HHMMSS>-<bank>-<file>` (UTC; `<HHMMSS>-<n>-…` on a clash); then the staged writes renamed in. The writes: `.pruned/…/<HHMMSS>-<bank>-ops.json`, the reply, compact; per keep `memories/<bank>/<type>_<slug>.md` (encode's slug and clash) with `name`, `description`, `type` from the op and `created`, `about`, `session`, `archived` from the trace, in encode's order, `source: consolidate <now> · <the trace's session> · model=<model>`; per merge the memory rewritten: its frontmatter kept in order with `description` and `source` (that stamp) replaced and `updated: <now>` set (appended when absent), the op's body. One line through `mark`'s writer, logged as `event` `consolidate`:

```
{"at","bank","error"?,"kind":"consolidate","kept":n,"machine","merged":n,"outcome":"consolidated"|"failed","pruned":n}
```

A stop mid-run kills the group and writes no line. `MEMORY.md` is left alone: the earlier sandman's recall reads memory files, never the index, and its commit path regenerates the index whole.

### `sandman setup`

Prints the launchd plist for `dev.gigantic.sandman` on stdout and writes nothing: `ProgramArguments` this executable and `serve`, `EnvironmentVariables` the caller's `PATH` (launchd's finds no `claude`) and the root it ran with, `KeepAlive`, `RunAtLoad`, `ThrottleInterval` 10, `ExitTimeOut` 2, `StandardErrorPath` `<root>/serve.err` for failures before the log (outside `run/`, which launchd would otherwise create 0744). Install, then swap after each `mise run install`:

```
sandman-next setup > ~/Library/LaunchAgents/dev.gigantic.sandman.plist
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/dev.gigantic.sandman.plist
launchctl kickstart -k gui/$(id -u)/dev.gigantic.sandman
```

### `sandman status`

Sends `{"op":"health"}` and prints the daemon's reply on stdout, exit 0; when its `version` is not this client's, one stderr line names both and the `launchctl kickstart -k` fix. No socket, a refused connect, or no `"ok":true` reply within 1 s: `{"code":"down","ok":false}`, exit 1. No retries.
