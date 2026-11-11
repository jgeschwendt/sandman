# sandman

```stele
kind: system
purpose: sandman, rewritten from scratch one part at a time — `mark` appends a session's start or end to the day's marks file, `chart` prints what the data root holds as one JSON object. The old sandman stays live at ~/.local/bin/sandman; this build installs as sandman-next.
commands:
  check: mise run check
  cover: mise run cover
  install: mise run install
  test: cargo test
```

## Map

| path                     | what                                                                                       |
| ------------------------ | ------------------------------------------------------------------------------------------ |
| `src/main.rs`            | the whole binary: the root, the machine id, `mark`, `chart`                                |
| `tests/sandman.rs`       | end-to-end suite: the built binary over temp roots via `SANDMAN_ROOT`, `SANDMAN_MACHINE`   |
| `docs/index.html`        | the page: data root, `mark`, `chart`, next; two d3 charts              |
| `docs/charts/*.js`       | one ES module per chart over `lib.js`'s chrome; d3 7.9.0 is the page's global              |
| `docs/data/chart.json`   | the timeline's sample: `tests/sandman.rs` `chart` fails unless it is the verb's output     |
| `docs/data/flow.json`    | the data-flow chart's nodes and edges, ①–⑤ marking the order a session passes them        |
| `Cargo.toml`             | the rust base's `lints.toml` verbatim; the one opt-out carries its reason                   |

The gate is `mise run check` (fmt, clippy `-D warnings`, `cover` at 100 % of lines), run by hk's pre-push (`hk install --mise` once per clone) and by CI. `lints check` from jgeschwendt/rust holds `Cargo.toml` to the base. Tests never touch `~/.sandman`; a child keeps the parent env (llvm-cov's `LLVM_PROFILE_FILE`), with `HOME` and `SANDMAN_*` replaced.

## Contracts

Root: `$SANDMAN_ROOT`, else `$HOME/.sandman`. Machine id: `$SANDMAN_MACHINE`, else `<root>/machine` (one line, 32 lowercase hex from `/dev/urandom`), minted on first use by a hard link from a temp file so racing first runs agree. `sandman` with no or another verb: two-line usage on stderr, exit 2.

### `sandman mark`

Stdin, one object; `kind` and `session` (36 chars, hex and `-`) required, anything else missing is null:

```
{"kind":"start"|"end","session":"<uuid>","cwd":"<path>"?,"surface":"<str>"?,"interactive":bool?,"reason":"<str>"?,"resume":"<str>"?}
```

Appends with `O_APPEND` to `<root>/marks/<yyyy>/<mm>/<dd>.jsonl` (UTC), compact, keys alphabetical:

```
{"at":"<RFC 3339 UTC, seconds, Z>","cwd":…,"interactive":…,"kind":…,"machine":…,"reason":…,"resume":…,"session":…,"surface":…}
```

Exit 0 silent; 2 on bad stdin, one stderr line, nothing written; 1 when the line cannot be written. Runs inside a 1.5 s hook budget.

### `sandman chart`

Stdout, one compact object, keys alphabetical; reads the root, writes nothing but a missing `machine`; every absence is an empty list or a skipped item; a closed stdout ends it quietly (exit 0):

```
{"active":[{"cwd":str|null,"end":str|null,"machine":str,"session":str,"start":str|null}],
 "at":"<now, RFC 3339 UTC>",
 "machine":"<this machine's id>",
 "memories":[{"bank":str,"created":str|null,"file":str,"name":str,"session":str|null,"source":"dream"|"reflect"|"remember"|"other","type":str}],
 "taken":[{"bytes":int,"cwd":str|null,"dreamed":str|null,"highlights":int,"memories":int,"session":str,"state":"waiting"|"dreamed"|"archived","taken":str,"title":str|null}]}
```

- `active` — per session in `marks/*/*/*.jsonl`: `start` the `at` of its earliest start mark (a reload re-fires it), `end` of its latest end mark; `cwd`, `machine` from that start mark, else the end mark. By `start` (null first), then session.
- `taken` — per file `<root>/.archive/claude/<yyyy>/<mm>/<dd>/<HHMMSS>-…-<uuid>.jsonl` directly in a day dir; the latest per session wins. `taken` = that date and time as UTC, `bytes` = size; `title`, `cwd`, `highlights` (count, 0 without), `dreamed` from `<root>/memories/.recent/<session>.json` when it parses as an object. `state`: `waiting` pointer without `dreamed`, `dreamed` with it, `archived` no pointer. `memories` = memory entries naming the session. By `taken`, then session.
- `memories` — per `<root>/memories/<bank>/<file>.md`, banks and files starting `.` or `_` and `MEMORY.md` skipped. Frontmatter: `key: value` lines between `---` fences; `name` (else the stem), `type` (else ""), `created`, `source` → `dream`/`reflect`/`remember` by its first word, else `other`; a dream's `session` is the uuid in `dream <ISO> · <uuid> · minds=…`. By bank, then file.
