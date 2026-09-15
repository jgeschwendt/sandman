# `sandman mcp`: the banks as a tool server

Status: **shipped 2026-09-10** — phases 1, 2 and 4 in this repo; phase 3 is registered in Claude Code (user scope) and in the desktop app, which spawns the server at launch (verified 2026-09-10 · `~/Library/Logs/Claude/mcp-server-sandman.log`); the Cowork tool calls are the operator's check. The brief is kept below as written, with the facts verified since marked inline. Read `AGENTS.md`, `src/AGENTS.md`, `docs/DESIGN.md`, and `docs/BANK-FORMAT.md` first; this document adds one verb and changes nothing about the format, the lock, or the passes.

The operator's ask, from the Cowork session that ablated the cloud memory store the same day: "why not a mcp server for sandman that gives you access?" then "can we make sandman a real server? on vercel? then everything can access my memory yeah?" then "maybe later." The decision that session reached: a local, stdio MCP server now; a remote (HTTP) server deferred; the server is what makes the cloud-to-sandman drain a single piece instead of an inbox plus a launchd routine.

## Why

Cloud sessions (Cowork, and the desktop app's linked sessions) reach this Mac through the desktop bridge. The bridge proxies the desktop app's local MCP servers into the cloud session as native tools (`Filesystem`, `Control Chrome`, and any server the operator adds), and it gives the session a shell, but that shell is a Linux VM with `~/.sandman` mounted, not a Mac process: `sandman` is not on its PATH and cannot be. So today a cloud session can read the banks as files and cannot call `remember` without hand-editing a bank, which the format-authority invariant forbids.

A `sandman mcp` server registered with the desktop app closes that gap with zero data movement: the cloud session calls `recall` and `remember` as tools, the binary runs on the Mac under the real commit lock, `MEMORY.md` regenerates through `commit.rs`, and no consumer re-implements slugging or the index. The same server is usable from Claude Code (`claude mcp add`) though Code keeps its `SessionStart` hook for recall; the hook is automatic and the tool is not.

What this does not do: phone and web chat do not route through the desktop bridge, so they still cannot reach sandman. That is the remote server's job, deferred. The cloud memory store keeps its projected `profile.md` and `preferences.md` for those surfaces until then.

## The contract

One new verb, one new module, no new dependency.

```
sandman mcp
    Serve the banks over MCP on stdio: JSON-RPC 2.0, one message per line,
    requests on stdin, responses on stdout, nothing else on stdout ever.
    Journals to <root>/.trace/mcp-<date>.log. Exits 0 on EOF.
```

Transport is stdio only in this phase. The handlers are written as plain functions over `(data_root, home, params) -> Result<Value, McpError>` with the transport as a thin loop around them, so the deferred HTTP transport is a second loop and not a second server.

### Tools

Three tools. Names are the verbs; the surface is deliberately smaller than the CLI because `take`, `dream`, `reflect`, and `forget` are Mac-lifecycle verbs with no cloud caller.

```
recall
    input:  { cwd?: string }            default: $HOME
    output: { text: string, banks: [{key, degraded, memories: [filename]}],
              memories: number, pointers: number, trimmed: {…} }
    Composes exactly what `sandman recall --cwd` composes: verbs::recall::compose.
    `text` is the payload a session would have been primed with; the rest is
    the Recalled shape so a caller can see what the budget cut.

remember
    input:  { body: string, bank?: string, cwd?: string, type?: "user"|"feedback"|"project"|"reference",
              name?: string, description?: string }
    output: { bank: string, file: string, outcome: "created"|"replaced"|"collided", index: string }
    Commits through verbs::remember::remember with the same defaults as the CLI.
    Exactly one of bank/cwd resolves the bank; both given is an invalid-params
    error, and neither given is the home bank.
    session_id comes from $CLAUDE_SESSION_ID when set, else absent, as today.

banks
    input:  {}
    output: { banks: [{ key: string, live: boolean, memories: number }] }
    Every bank under <root>/memories/, each marked live when the directory
    its key decodes to still exists. A cloud caller uses this to pick a home
    instead of guessing the slug encoding; `live: false` is the signal that a
    bank should be retired, not written to.
```

Tool descriptions (the strings a model reads) state the budget rule from `~/.sandman/memories/-Users-jlg/user_memory_is_a_budget.md`: short bodies, no counts, versions, prices, or listings; bank the rule and the probe. The server does not enforce lengths; `commit.rs` is the authority and the description is the nudge.

### Protocol

JSON-RPC 2.0 over stdio, newline-delimited, per the current MCP specification. Verified 2026-09-10: Claude Code 2.1.268 opens the server with the legacy `initialize` handshake and asks for `2025-11-25`. The server answers `2025-11-25`, `2025-06-18`, `2025-03-26` and `2024-11-05`, echoing the client's requested version when it is one of those and `2025-11-25` otherwise (a missing or non-string version included). The spec's newest revision, 2026-07-28, replaces `initialize` with per-request `_meta` and is deliberately not implemented until a client sends it. The methods that must work:

```
initialize                → { protocolVersion, capabilities: { tools: {} }, serverInfo: { name: "sandman", version } }
notifications/initialized → no response
ping                      → {}
tools/list                → { tools: [ {name, description, inputSchema} × 3 ] }
tools/call                → { content: [ { type: "text", text: <JSON of the output shape> } ], isError: false }
                            or { content: [ { type: "text", text: <message> } ], isError: true } for a verb failure
anything else             → JSON-RPC error -32601
malformed line            → JSON-RPC error -32700 with id null, then keep reading
invalid params            → -32602
request before initialize → -32600
```

A verb failure (bad bank, empty body, lock contention) is a tool result with `isError: true`, not a JSON-RPC error: the model should read it and recover. Only protocol-level problems are JSON-RPC errors.

`version` is the same stamp `sandman version` prints (`src/version.rs`), so a journal line can be read against the build that wrote it.

### JSON

`src/json.rs` is `pub(crate)`, hand-rolled, and the reason `[dependencies]` is empty. It parses and renders `Value` with key order kept; that is all this verb needs. Do not add serde. If `json.rs` lacks a helper the server wants (object construction from pairs, a `Number` from `usize`), add it there with the same discipline: typed failures, no panics on hostile input, depth-bounded.

### Journal

Every request journals one line to `<root>/.trace/mcp-<date>.log` through `src/journal.rs`, the same shape every verb uses: stamped, pid, build, then `method= tool= bank= outcome=` and for recall the `Recalled` counts (`chars= banks= degraded= trimmed=`) and never the payload. Errors journal their kind and never a body.

### Concurrency

Stdio is one client, one request at a time; the server handles requests sequentially and does not spawn. The commit lock (`src/lock.rs`) already serializes against reflect, dream, and a concurrent CLI `remember`; a lock failure surfaces as an `isError` tool result naming the holder, and the caller retries. Nothing here reads `.recent/` or the queue for itself (see the `lm:queue-definition` invariant).

### Registration

Desktop app, so the bridge proxies it into cloud sessions:

```
"sandman": { "command": "/Users/jlg/.local/bin/sandman", "args": ["mcp"] }
```

The desktop app reads this from `~/Library/Application Support/Claude/claude_desktop_config.json` under the `mcpServers` key (Settings › Developer › Edit Config opens the same file); a restart of the app picks it up (verified 2026-09-10 · the file on this Mac). Claude Code, optional, for sessions that want the tools alongside the hook:

```
claude mcp add --scope user sandman -- "$HOME/.local/bin/sandman" mcp
```

The binary path is absolute because hooks and spawned processes get a stale PATH snapshot (`~/.sandman/memories/-Users-jlg--claude/project_claude_harness_path_and_classifier.md`).

## Phases

**1. Handlers — shipped.** `src/verbs/mcp.rs`: the three tool functions over `data_root`/`home`, the tool schemas as constants, `McpError`. Unit tests against a temp `$SANDMAN_ROOT` seeded with two banks: `recall` equals `verbs::recall::compose` byte for byte; `remember` produces the file `tests/cli.rs` would expect from the equivalent CLI call and regenerates the index; `banks` decodes keys back to paths and marks a missing directory `live: false`.

**2. Transport — shipped.** The stdio loop in the same module: read a line, parse, dispatch, render, write, flush; `initialize` handshake state (a `tools/call` before `initialize` is an error); EOF exits 0. `src/cli.rs` gains the `mcp` arm and the usage stanza above. Integration test in `tests/mcp.rs` driving the built binary with a scripted stdin (initialize, initialized, tools/list, three tools/call, a malformed line, EOF) and asserting every stdout line parses and matches.

**3. Live check.** Register with the desktop app. From a Cowork session linked to this Mac: `sandman__banks` lists every bank under `memories/` (25, measured 2026-09-10 · the scripted session against a copy of the root); `sandman__recall {cwd: "/Users/jlg"}` returns the same text as `sandman recall --cwd ~` run in a terminal, compared in `.trace/`; `sandman__remember` into a scratch bank under a temp `$SANDMAN_ROOT` (the server inherits the env of its launcher, so the check runs against a copy, not the live root) writes the file and the index. Then the real root.

**4. Docs — shipped.** `README.md` gains the verb; `AGENTS.md` map gains `verbs/mcp.rs`; this document's status line changes to shipped with the date.

## Decisions taken here, open to the operator

- **Three tools, not the CLI.** `take`/`dream`/`reflect`/`forget` stay CLI-only; a cloud session has no transcript to take and no business running a pass. Reversible: a tool is a function and a schema.
- **`recall` returns the rendered text, not the memories.** The tool exists so a cloud session sees what a Code session sees; a structured dump would invite a second renderer downstream. The `Recalled` counts ride along for diagnosis.
- **No `forget` over MCP.** The privacy ending should not be one tool call away from any session that can see the server.
- **stdio before HTTP.** HTTP means a public door with OAuth in front so Anthropic's servers can call it, which cuts against the Tailscale-only rule for personal tooling; that is a separate decision with its own doc when the operator wants phone reach.

## Hard rules

- Std only. `[dependencies]` stays empty.
- Every bank write goes through `commit.rs`. The server holds no format knowledge of its own.
- Nothing but JSON-RPC on stdout. Diagnostics go to stderr or the journal; one stray `println!` breaks the client.
- Never journal a body or a recall payload.
- `$SANDMAN_ROOT` else `~/.sandman`, resolved in `paths.rs` and nowhere else.
- `cargo clippy --all-targets -- -D warnings && cargo fmt --check && cargo test` green before the operator is asked to look.
- No commit, push, or PR without the operator's word, per change.

## After this ships

The cloud-to-sandman drain becomes one scheduled Cowork task, Mac-linked, nightly beside `sandman-reflect`: read the cloud memory store, `sandman__remember` every line not marked `(bank: …)`, then rewrite the store's `profile.md` and `preferences.md` from the `~` bank's `user_*` and `feedback_*` memories. Auto-memory (`~/.claude/projects/*/memory/`) is retired independently of all of this; its tarball from 2026-09-09 is the rollback.

## TODO · from the first live Cowork check (2026-09-11)

The operator's phase-3 check ran from a Cowork session linked to this Mac. `banks` listed every bank; `remember` created `reference_sandman_reachable_from_cowork_cloud.md` in the `~` bank and regenerated the index; `recall` with no cwd answered for `~` and reported `trimmed.sections: ["chronological"]`, no `tools`.

- **`~` recall is over budget, so the voyage log never reaches a session started in `~`.** The home bank's bodies alone fill the 9,000 characters; `compose` drops the tools surface (absent here: no `TOOLS.md` with content) then the chronological one, and the graph never floors, so nothing is reinstated. Reported correctly, but the log the operator asked for is invisible from exactly the cwd he starts most sessions in. Options, unranked: slim the `~` bank (the budget rule says it should be small); reserve a floor for the log tail before the graph gets the remainder; render the log as one index line when it cannot be carried whole. Decision is the operator's.
- **Retire a wrong memory.** The Cowork session banked that recall's payload "omits the voyage log tail and tool index the description promises". It does not: the log was budget-trimmed and said so; the tool index is empty on this box. `sandman forget` it from `~/.sandman/memories/-Users-jlg/reference_sandman_reachable_from_cowork_cloud.md`, or re-`remember` it with the first sentence only (the bridge path and the ToolSearch step are right).
- **The description could name the trim.** The `recall` tool description reads as if all five surfaces always arrive; a clause that `trimmed.sections` names the surfaces the budget cut would have prevented the misread above. One line in `src/verbs/mcp.rs`.
