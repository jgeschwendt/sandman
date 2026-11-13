# sandman

```stele
kind: system
purpose: sandman, rewritten from scratch one part at a time — `mark` appends a session's start or end to the day's ledger file, `take` ends a marked session (archiving its transcript) with one more, `chart` prints what the data root holds as one JSON object, `serve` is the daemon under launchd, whose tick settles held takes and encodes episodes into traces, `status` asks it for its health, `setup` prints its plist. The old sandman stays live at ~/.local/bin/sandman; this build installs as sandman-next.
commands:
  check: mise run check
  cover: mise run cover
  install: mise run install
  test: cargo test
```

## Map

| path                     | what                                                                                       |
| ------------------------ | ------------------------------------------------------------------------------------------ |
| `src/main.rs`            | the whole binary: the root, the machine id, `mark`, `take`, `chart`, `serve` and its tick (settle, encode), `setup`, `status` |
| `src/encode.prompt.md`   | the encoder's prompt, embedded with `include_str!`; the digest follows it on stdin          |
| `tests/sandman.rs`       | end-to-end suite: the built binary over temp roots and Claude homes via `SANDMAN_ROOT`, `SANDMAN_MACHINE`, `CLAUDE_CONFIG_DIR`, a stub shell encoder via `SANDMAN_ENCODER`; a `Daemon` guard kills every `serve` it starts (SIGTERM one whose coverage counts: a SIGKILL writes no profile) |
| `docs/index.html`        | the page: data root, `mark`, `take`, `chart`, `serve`, `encode`, next; two d3 charts and the daemon's lifecycle figure |
| `docs/charts/*.js`       | one ES module per chart over `lib.js`'s chrome; d3 7.9.0 is the page's global              |
| `docs/data/chart.json`   | the timeline's sample: `tests/sandman.rs` `chart` fails unless it is the verb's output     |
| `docs/data/flow.json`    | the data-flow chart's moments, nodes and edges, ①–④ marking the order a session passes them; `next` nodes are not built yet |
| `Cargo.toml`             | the rust base's `lints.toml` verbatim; the one opt-out and `signal-hook` carry their reasons |

The gate is `mise run check` (fmt, clippy `-D warnings`, `cover` at 100 % of lines), run by hk's pre-push (`hk install --mise` once per clone) and by CI. `lints check` from jgeschwendt/rust holds `Cargo.toml` to the base. Tests never touch `~/.sandman`; a child keeps the parent env (llvm-cov's `LLVM_PROFILE_FILE`), with `HOME`, `CLAUDE_*` and `SANDMAN_*` replaced.

## Contracts

Root: `$SANDMAN_ROOT`, else `$HOME/.sandman`. Machine id: `$SANDMAN_MACHINE`, else `<root>/machine` (one line, 32 lowercase hex from `/dev/urandom`), minted on first use by a hard link from a temp file so racing first runs agree. `sandman` with no or another verb: three-line usage on stderr, exit 2.

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

The first that holds: no start mark for the session in the ledger, or `reason` `resume` → nothing written. Else one end line through `mark`'s writer, `outcome`: `skipped` when `$SANDMAN_NO_TAKE` is non-empty or `CLAUDE_MEMORY_PIPELINE=1`; `held` (plus `job`, the dir name) when `reason` is not `clear` and a `<claude>/jobs/*/state.json` names the session by `resumeSessionId`, else `sessionId`; `gone` without `<claude>/projects/*/<session>.jsonl`; else `taken` (plus `archived`). `<claude>` = `$CLAUDE_CONFIG_DIR`, else `$HOME/.claude`. The pointer below stays until `recall` replaces the earlier one.

```
{"archived"?,"at","cwd","job"?,"kind":"end","machine","outcome","reason","session"}
```

The take, byte-compatible with the earlier sandman's: `fs::rename` the transcript to `<root>/.archive/claude/<yyyy>/<mm>/<dd>/<HHMMSS>-<path under <claude>, / → ->` (UTC take time; `<HHMMSS>-<n>-…` on a same-second clash) and `<project>/<session>/`, when present, to that stem + `-dir`; then `<root>/memories/.recent/<session>.json` by temp + rename, compact, newline-terminated, in this key order: `{"archived","cwd","ended","title","highlights"}`. `ended` the transcript's mtime; `cwd` the first line's `cwd` without control characters, else stdin's, else null; `title` the first `type:"user"` line's string content or first `text` block, whitespace collapsed, 80 chars, else the session id; `highlights` the quoted bodies (`"…"` with `\"` `\\` unescaped, or `\"…\"`) after `sandman remember ` then `orrery attend ` in assistant `tool_use.input.command`s and user `<bash-input>` lines, one-lined, 4,000 chars, a repeat moved to the end, the last 20. Rename, pointer, ledger line, in that order. Exit 0 silent when decided; 2 on bad stdin, nothing written; 1 when a rename or write fails (cross-device included), one stderr line, no ledger line.

### `sandman chart`

Stdout, one compact object, keys alphabetical; reads the root, writes nothing but a missing `machine`; every absence is an empty list or a skipped item; a closed stdout ends it quietly (exit 0):

```
{"active":[{"cwd":str|null,"encoded":str|null,"end":str|null,"machine":str,"outcome":str|null,"session":str,"start":str|null}],
 "at":"<now, RFC 3339 UTC>",
 "machine":"<this machine's id>",
 "memories":[{"bank":str,"created":str|null,"file":str,"name":str,"session":str|null,"source":"dream"|"reflect"|"remember"|"other","type":str}],
 "taken":[{"bytes":int,"cwd":str|null,"dreamed":str|null,"highlights":int,"memories":int,"session":str,"state":"waiting"|"dreamed"|"archived","taken":str,"title":str|null}],
 "traces":[{"bank":str,"created":str|null,"file":str,"name":str,"session":str|null,"type":str}]}
```

- `active` — per session in `ledger/*/*/*.jsonl`: `start` the `at` of its earliest start mark (a reload re-fires it), `end` and `outcome` of its latest end mark (null on an end line without one); `cwd`, `machine` from that start mark, else the end mark; `encoded` the `outcome` of its latest encode line. By `start` (null first), then session.
- `taken` — per file `<root>/.archive/claude/<yyyy>/<mm>/<dd>/<HHMMSS>-…-<uuid>.jsonl` directly in a day dir; the latest per session wins. `taken` = that date and time as UTC, `bytes` = size; `title`, `cwd`, `highlights` (count, 0 without), `dreamed` from `<root>/memories/.recent/<session>.json` when it parses as an object. `state`: `waiting` pointer without `dreamed`, `dreamed` with it, `archived` no pointer. `memories` = memory entries naming the session. By `taken`, then session.
- `memories` — per `<root>/memories/<bank>/<file>.md`, banks and files starting `.` or `_` and `MEMORY.md` skipped. Frontmatter: `key: value` lines between `---` fences; `name` (else the stem), `type` (else ""), `created`, `source` → `dream`/`reflect`/`remember` by its first word, else `other`; a dream's `session` is the uuid in `dream <ISO> · <uuid> · minds=…`. By bank, then file.
- `traces` — per `<root>/traces/<bank>/<file>.md`, the same reader and skips; `session` from the frontmatter. By bank, then file.

### `sandman serve`

The daemon, a foreground process under launchd. Start: a socket path over 103 bytes, a `run/` not this user's or reachable by group or other, or a held lock is one stderr line and exit 1. Otherwise it creates `run/` 0700, holds an exclusive `flock` on `run/serve.lock` for life, binds `run/serve.new` and renames it over `run/serve.sock` (the lock proves a stale socket dead), writes `run/serve.pid`, logs its start and prints its health line on stdout: ready.

One newline-terminated JSON request and one reply per connection, compact, keys alphabetical; a connection silent for 1 s is dropped unanswered:

```
{"op":"health"}   → {"machine":str,"ok":true,"pid":int,"since":"<RFC 3339 start>","state":"booting"|"ready"|"stopping","version":"<crate version>"}
{"op":"shutdown"} → {"ok":true}, written after the listener is dropped
anything else     → {"code":"bad_request"|"unknown_op","ok":false}
```

`state` is `ready` whenever it answers today: state is derived before the socket is bound, and a stopping daemon no longer accepts. SIGTERM, SIGINT and SIGHUP take the shutdown path; SIGPIPE is ignored (Rust's default). Exit 0 after removing `serve.pid` and `serve.sock` while the pid file still holds its pid. The drain budget is 2 s (`ExitTimeOut`): one connection in hand, at most its 1 s read, and the tick's take or encode in hand (a running encoder's group killed); the tick starts no other.

The tick, a thread: every `$SANDMAN_TICK_MS` (30 000) it re-reads the ledger and, per session whose latest line is an end with outcome `held` and this machine's id, under a lock the drain takes after setting the stop flag, skips one a job still names or whose transcript was modified within `$SANDMAN_QUIET_MS` (120 000) and otherwise runs `take`'s end with reason `job gone` (`taken` or `gone`). Nothing is held in memory; a failed take stays held and is retried next tick. Then it runs one encode (below). `run/serve.log` is the daemon's: one compact object per line, keys alphabetical, `at` on each — `{"event":"start","pid","version"}`, `{"event":"exit","reason":"signal TERM"|"shutdown request"|<accept error>}`, `{"event":"take"|"encode",…the ledger line}` or `{"event":…,"reason":<write error>,"session"}` per tick take or encode, `{"event":"panic","reason"}` from the panic hook. A line that finds the log at `$SANDMAN_LOG_CAP` bytes (default 1 MiB) first renames it to `serve.log.1`.

### encode

A job of `serve`'s tick, not a verb. Per tick, under the tick's lock, after settling: of the sessions whose latest ledger line is an end with outcome `taken` and this machine's id, the one with the earliest `at` (so an encoded episode, whose latest line is its encode, is never picked again). One line through `mark`'s writer, logged to `serve.log` as `event` `encode`:

```
{"at","error"?,"kind":"encode","machine","outcome":"encoded"|"skipped"|"gone"|"failed","session","traces":<count>}
```

`skipped`: no start mark of the session has `interactive` true. `gone`: no file at the end line's `archived`. Else the digest, to `run/encode.in`: `# episode` (`date:` the end's `at`, `cwd:` as the pointer's, else `unknown`), `# highlights` (`- <body>` per take highlight, absent without), `# transcript` (per user or assistant message part: `user: <text>`, `assistant: <text>`, `tool: <name> <command|file_path>` one-lined to 200; tool results and other lines dropped); over `$SANDMAN_DIGEST_BYTES` (300 000) the head and tail halves, cut back to whole lines, around `… <n> bytes omitted …`. The encoder, `$SANDMAN_ENCODER` else `claude -p --model opus --output-format text --safe-mode --tools "" --no-session-persistence`, runs as `sh -c`, own process group, env `CLAUDE_MEMORY_PIPELINE=1` `SANDMAN_NO_TAKE=1`, stdin `<prompt>\n<digest>`, stdout and stderr to `run/encode.out`, `run/encode.err`. The contract a replacement keeps: exit 0 and stdout holding one object, `{"traces":[{"type":"user"|"feedback"|"project"|"reference","name":str,"description":str,"body":str}]}`, read from its first `{` to its last `}`. `failed` (`error` one line, 400 chars, no file written): `exit status: <n>: <stderr>`, `timed out after <ms> ms` (`$SANDMAN_ENCODE_MS`, 300 000; the group killed), `output: <parse error>`, `output: not a trace: <it>`. Else `encoded`, one file per trace: `<root>/traces/<bank>/<type>_<slug>.md` by temp + rename, `<bank>` the earlier sandman's `Bank::key_for(project::root(cwd))` (`project`, `common`; no cwd: `$HOME`), `<slug>` its `slug::slug` (lowercase, non-alphanumeric runs `_`, trimmed, 60 chars), `_2`… on a clash:

```
---
name: <one-lined, 200>
description: <one-lined, 400>
type: <type>
created: <now>
session: <uuid>
archived: <path>
source: encode <now> · <uuid> · model=<the word after --model in the encoder, else its first word>
---

<body, trimmed>
```

A stop signal or shutdown mid-run kills the group and writes no line: the episode runs again after the next start. `failed` is final for the tick; removing its line retries it.

### `sandman setup`

Prints the launchd plist for `dev.gigantic.sandman` on stdout and writes nothing: `ProgramArguments` this executable and `serve`, `EnvironmentVariables` the caller's `PATH` (launchd's finds no `claude`) and the root it ran with, `KeepAlive`, `RunAtLoad`, `ThrottleInterval` 10, `ExitTimeOut` 2, `StandardErrorPath` `<root>/serve.err` for failures before the log (outside `run/`, which launchd would otherwise create 0744). Install, then swap after each `mise run install`:

```
sandman-next setup > ~/Library/LaunchAgents/dev.gigantic.sandman.plist
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/dev.gigantic.sandman.plist
launchctl kickstart -k gui/$(id -u)/dev.gigantic.sandman
```

### `sandman status`

Sends `{"op":"health"}` and prints the daemon's reply on stdout, exit 0; when its `version` is not this client's, one stderr line names both and the `launchctl kickstart -k` fix. No socket, a refused connect, or no `"ok":true` reply within 1 s: `{"code":"down","ok":false}`, exit 1. No retries.
