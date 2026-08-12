# tests

```stele
kind: container
purpose: integration tests — `cli.rs` drives the built binary against a temp $HOME/$SANDMAN_ROOT; `real_banks.rs` round-trips the operator's live banks read-only (ignored by default, `cargo test -- --ignored`)
```

## Working here

- Never touch the real `~/.sandman` or `~/.claude` — fabricate a home, point
  `$HOME` and `$SANDMAN_ROOT` at it, and pass roots explicitly.
- Never run the real `claude` — point `$SANDMAN_CLAUDE_BIN` at a stub script
  under the fabricated home. The binary is env-configurable for exactly this.
- Unit tests live beside their module in `src/`; only cross-process and
  live-machine coverage belongs here.

<!-- stele:begin router -->
<!-- stele:end -->
