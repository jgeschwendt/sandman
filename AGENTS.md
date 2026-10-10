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
| breath/ | `sources.toml`, the seed feed list `sandman breathe init` copies into the data root — compiled into the binary; the schema is `docs/BANK-FORMAT.md` § breath |
| docs/ | the design — `DESIGN.md` + a ui-styled page (GitHub Pages serves this directory) (`ui.css` vendored from jgeschwendt/ui @ 4f1d9db — one comment edited, so not byte-identical); `docs/serve.py` (:7875); `plates/` holds the page plates and `plates/perpetual-shift/` the plan plate's Claude Design source |
| prompts/ | the machine's prompt templates — `shift.md` is the dynamic shift prompt `sandman shift brief` fills, `breathe.md` the ruach's pass prompt `sandman breathe pass` fills; their slot names are the contract in `docs/BANK-FORMAT.md` § shifts and § breath |
| src/ | crate — the commit path + bank format, the dispatcher and session-edge verbs, dream's mind runner and 2-of-3 consensus, reflect's voyage entry, sweep and bank upkeep, the MCP tool server (recall · remember · banks over stdio), the trail and reconcile, the shift engine, the ruach's store |

The design is `docs/DESIGN.md`; the on-disk format is `docs/BANK-FORMAT.md`.

## Working here

- Data root is `$SANDMAN_ROOT`, else `~/.sandman` — one config point (`src/paths.rs`); nothing else names it.
- Never call a model from a test — `$SANDMAN_CLAUDE_BIN` (and the `Options` structs it feeds) is the seam, and the stubs are shell scripts in a temp dir.
- Std only — `[dependencies]` is empty and stays that way until a verb needs otherwise.

<!-- @stele -->

## Hazards (1 active)

- ⚠ `src`: a `SessionEnd` carrying reason `resume` is a beginning, not an ending — Claude Code fires it on the session it is adopting, so `take --hook` declines it. Forcing there moves the transcript out from under the turn about to append to it; Claude Code recreates the file, the next ending takes that live fragment too, and `.recent/<sid>.json` is overwritten to name the stub while the whole conversation sits orphaned in the archive (→ ※ resume-is-not-an-ending)

## Map

| node    | kind      | purpose                                                                                                                                                                                                 | unfold                                               |
| ------- | --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------- |
| breath  | container | the ruach's seed — `sources.toml`, the feed list `sandman breathe init` copies to `<root>/breath/` when absent; its schema is `docs/BANK-FORMAT.md` § breath                                            | `stele unfold breath` · or read `breath/AGENTS.md`   |
| prompts | container | the machine's prompt templates — `shift.md` (`sandman shift brief`) and `breathe.md` (`sandman breathe pass`); their slot names are the contract (`docs/BANK-FORMAT.md` § shifts, § breath)             | `stele unfold prompts` · or read `prompts/AGENTS.md` |
| src     | container | crate source — lib + `sandman` bin; `commit.rs` is the format authority (lock, `_archive`, collisions, index regen), landmark in lib.rs; `cli.rs` dispatches `verbs/`; `mind.rs` is the only model call | `stele unfold src` · or read `src/AGENTS.md`         |
| tests   | container | integration tests — `breathe` `cli` `mcp` `shift` `trail` drive the built binary against a temp $HOME/$SANDMAN_ROOT and fixture repos; `real_banks.rs` reads the live banks (`--ignored`)               | `stele unfold tests` · or read `tests/AGENTS.md`     |

## Indexes

All invariants: `.stele/index/invariants.md` · all hazards: `.stele/index/hazards.md`

## Engine

`stele` CLI available → `stele root | unfold <id> | invariants --touching <path> | hazards | nodes --kind <k>`. MCP: `stele serve`.
No engine → everything above is complete; nested AGENTS.md files carry the detail (nearest file wins).
<!-- @end -->
