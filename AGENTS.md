# sandman

```stele
kind: system
purpose: sandman — memory engine for Claude sessions (banks, session archiving, the dream and reflect passes); a zero-dependency Rust CLI driven by Claude Code hooks and a nightly tick.
commands:
  lint: cargo clippy --all-targets -- -D warnings && cargo fmt --check
  test: cargo test
invariants:
  - claim: single format authority — every bank write goes through this crate's commit path; every consumer uses the CLI contract and never re-implements slugging, `replaces` archiving, lineage, collision suffixes, or index regen
    anchor: lm:format-authority
  - claim: one config point — the data root is `$SANDMAN_ROOT`, else `~/.sandman`; nothing outside `src/paths.rs` names it
    anchor: README.md#sandman
```

## Map

| where | what |
| --- | --- |
| design/ | `.tldr` scenes — `sandman-v1` is the design wireframe (`sandman-v0` kept as its predecessor); `mise run plan` (:7873) draws them |
| docs/ | the design — `DESIGN.md` + a ui-styled page (GitHub Pages serves this directory) (`ui.css` vendored from jgeschwendt/ui @ 4f1d9db — one comment edited, so not byte-identical); `docs/serve.py` (:7875); `arch.tldr` is the tldraft drawing the page's plates are cut from, into `plates/` — one frame per page section |
| src/ | crate — the commit path + bank format, the dispatcher and session-edge verbs, dream's mind runner and 2-of-3 consensus, reflect's day page, sweep and bank upkeep |

The design is `docs/DESIGN.md`; the on-disk format is `docs/BANK-FORMAT.md`.

## Working here

- Data root is `$SANDMAN_ROOT`, else `~/.sandman` — one config point (`src/paths.rs`); nothing else names it.
- Never call a model from a test — `$SANDMAN_CLAUDE_BIN` (and the `Options` structs it feeds) is the seam, and the stubs are shell scripts in a temp dir.
- Std only — `[dependencies]` is empty and stays that way until a verb needs otherwise.

<!-- stele:begin router -->

## Hazards (1 active)

- ⚠ `src`: a `SessionEnd` carrying reason `resume` is a beginning, not an ending — Claude Code fires it on the session it is adopting, so `take --hook` declines it. Forcing there moves the transcript out from under the turn about to append to it; Claude Code recreates the file, the next ending takes that live fragment too, and `.recent/<sid>.json` is overwritten to name the stub while the whole conversation sits orphaned in the archive (→ lm:resume-is-not-an-ending)

## Map

| node  | kind      | purpose                                                                                                                                                                                                  | unfold                                           |
| ----- | --------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| src   | container | crate source — lib + `sandman` bin; `commit.rs` is the format authority (lock, `_archive`, collisions, index regen), landmark in lib.rs; `cli.rs` dispatches `verbs/`; `mind.rs` is the only model call  | `stele unfold src` · or read `src/AGENTS.md`     |
| tests | container | integration tests — `cli.rs` drives the built binary against a temp $HOME/$SANDMAN_ROOT; `real_banks.rs` round-trips the operator's live banks read-only (ignored by default, `cargo test -- --ignored`) | `stele unfold tests` · or read `tests/AGENTS.md` |

## Indexes

All invariants: `.stele/index/invariants.md` · all hazards: `.stele/index/hazards.md`

## Engine

`stele` CLI available → `stele root | unfold <id> | invariants --touching <path> | hazards | nodes --kind <k>`. MCP: `stele serve`.
No engine → everything above is complete; nested AGENTS.md files carry the detail (nearest file wins).
<!-- stele:end -->
