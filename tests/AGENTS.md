# tests

```stele
kind: container
purpose: integration tests — `breathe` `cli` `mcp` `report` `score` `shift` `trail` drive the built binary against a temp $HOME/$SANDMAN_ROOT and fixtures; `real_banks.rs` reads the live banks (`--ignored`)
```

## Working here

- Never touch the real `~/.sandman` or `~/.claude` — fabricate a home, point
  `$HOME` and `$SANDMAN_ROOT` at it, and pass roots explicitly.
- Never run the real `claude` — point `$SANDMAN_CLAUDE_BIN` at a stub script
  under the fabricated home. The binary is env-configurable for exactly this.
- Fixture git runs with the environment cleared, `GIT_CONFIG_GLOBAL=/dev/null` and a
  fixed identity — for the fixture's own git calls and for the binary's.
- Unit tests live beside their module in `src/`; only cross-process and
  live-machine coverage belongs here.

<!-- @stele -->
<!-- @end -->
