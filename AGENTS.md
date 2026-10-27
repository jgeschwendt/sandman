# sandman

```stele
kind: system
purpose: sandman — memory engine for Claude sessions (banks, session archiving, the dream and reflect passes, an MCP tool server); a zero-dependency Rust CLI driven by Claude Code hooks and a nightly tick.
commands:
  lint: cargo clippy --all-targets -- -D warnings && cargo fmt --check
  test: cargo test
invariants:
  - claim: single format authority — every bank write goes through this crate's commit path; every consumer uses the CLI contract and never re-implements slugging, `replaces` archiving, lineage, collision suffixes, or index regen
    anchor: ※ format-authority
  - claim: one config point — the data root is `$SANDMAN_ROOT`, else `~/.sandman`; nothing outside `src/paths.rs` names it
    anchor: README.md#sandman
```

## Map

| where | what |
| --- | --- |
| breath/ | `sources.toml`, the seed feed list `sandman breathe init` copies into the data root — compiled into the binary; the schema is `docs/bank-format.html#breath` |
| docs/ | the design — `index.html`, a ui-styled page and the only design source, the perpetual-shift plan included (GitHub Pages serves this directory; stylesheets linked from https://gigantic.dev/ui/ — `ui.css` + the shared `docs.css` shell; `page.css` holds only what is sandman's); `docs/serve.py` (:7875); `figures/` holds the page's figure PNGs, each with a `-dark` twin, and `figures/project/` the Claude Design canvas source (https://claude.ai/artifact/MKRUSNkEimZQHvWmZ4VASL, `sandman figures`) with the design-system tokens read back under `project/ds/`; `figures/export.sh` (`mise run figures`) renders every board to its PNG, then its `-dark.png` through `darken.py` (light token hexes → dark values; prints any hex that is not a token); the contracts are the pages `bank-format.html` and `mcp.html`, and `routines/` |
| launchd/ | the launchd agents, `{{HOME}}` templates rendered by `sandman plist <job>`: `dev.gigantic.sandman.breathe.plist` runs `sandman breathe tick` hourly at :17 (`docs/bank-format.html#breath-tick`); `dev.gigantic.sandman.install.plist` runs `install` every fifteen minutes, keeping the installed binary, sandbox files and routines at a clean trunk (`docs/bank-format.html#shifts-sandbox`); `dev.gigantic.sandman.report.plist` keeps `sandman report serve` up on 127.0.0.1:1705 (`docs/bank-format.html#report`); `dev.gigantic.sandman.reflect.plist` runs `sandman reflect` nightly at 23:30 local (`docs/bank-format.html#log-rules`) |
| profiles/ | `session.json`, the Claude Code settings shift and breath sessions run under — the strict Bash sandbox and deny rules `sandman shift profile` prints, compiled into the binary; `shift-test` + `shift-test.sb`, the wrapper and Seatbelt profile a shift's tests run under, installed by the install agent; the contract is `docs/bank-format.html#shifts-sandbox` |
| prompts/ | the machine's prompt templates — `shift.md` is the dynamic shift prompt `sandman shift brief` fills, `sabbath.md` the one it fills on Saturday, and `breathe.md` the ruach's pass prompt `sandman breathe pass` fills; their slot names are the contract in `docs/bank-format.html#shifts` and `docs/bank-format.html#breath` |
| src/ | crate — the commit path + bank format, the dispatcher and session-edge verbs, dream's mind runner and 2-of-3 consensus, reflect's voyage entry, sweep and bank upkeep, the MCP tool server (recall · remember · banks over stdio), the trail and reconcile, the shift engine (every landing a pull request), the ruach's store, the week's score |

The design is `docs/index.html`; the on-disk format is `docs/bank-format.html`.

## Working here

- Data root is `$SANDMAN_ROOT`, else `~/.sandman` — one config point (`src/paths.rs`); nothing else names it.
- Never call a model from a test — `$SANDMAN_CLAUDE_BIN` (and the `Options` structs it feeds) is the seam, and the stubs are shell scripts in a temp dir.
- Std only — `[dependencies]` is empty and stays that way until a verb needs otherwise.

<!-- @stele -->

## Hazards (1 active)

- ⚠ `src`: a `SessionEnd` carrying reason `resume` is a beginning, not an ending — Claude Code fires it on the session it is adopting, so `take --hook` declines it. Forcing there moves the transcript out from under the turn about to append to it; Claude Code recreates the file, the next ending takes that live fragment too, and `.recent/<sid>.json` is overwritten to name the stub while the whole conversation sits orphaned in the archive (→ ※ resume-is-not-an-ending)

## Map

| node     | kind      | purpose                                                                                                                                                                                               | unfold                                                 |
| -------- | --------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| breath   | container | the ruach's seed — `sources.toml`, the feed list `sandman breathe init` copies to `<root>/breath/` when absent; its schema is `docs/bank-format.html#breath`                                          | `stele unfold breath` · or read `breath/AGENTS.md`     |
| profiles | container | the Claude Code settings `session.json` that shift and breath sessions run under, and `shift-test` + `shift-test.sb`, the Seatbelt their tests get; contract `docs/bank-format.html#shifts-sandbox`   | `stele unfold profiles` · or read `profiles/AGENTS.md` |
| prompts  | container | the machine's prompts — `shift.md`, `sabbath.md`, `breathe.md`, whose slot names are the contract (docs/bank-format.html#shifts, docs/bank-format.html#breath)                                        | `stele unfold prompts` · or read `prompts/AGENTS.md`   |
| src      | container | crate source — lib + `sandman` bin; `commit.rs` is the format authority (lock, `_archive`, collisions, index regen), landmark in lib.rs; `cli/` dispatches `verbs/`; `mind.rs` is the only model call | `stele unfold src` · or read `src/AGENTS.md`           |
| tests    | container | integration tests — `breathe` `cli` `launchd` `mcp` `report` `score` `shift` `trail` drive the binary on a temp $HOME and fixtures; `real_banks` live banks (`--ignored`), `shift_test` real Seatbelt | `stele unfold tests` · or read `tests/AGENTS.md`       |

## Indexes

All invariants: `.stele/index/invariants.md` · all hazards: `.stele/index/hazards.md`

## Engine

`stele` CLI available → `stele root | unfold <id> | invariants --touching <path> | hazards | nodes --kind <k>`. MCP: `stele serve`.
No engine → everything above is complete; nested AGENTS.md files carry the detail (nearest file wins).
<!-- @end -->
