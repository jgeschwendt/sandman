# profiles

```stele
kind: container
purpose: the Claude Code settings `session.json` that shift and breath sessions run under, and `shift-test` + `shift-test.sb`, the Seatbelt their tests get; contract `docs/bank-format.html#shifts-sandbox`
```

## Working here

- Compiled into the binary (`include_str!` in `src/verbs/shift.rs`): an edit reaches a
  session only through a rebuilt, installed binary.
- Keys sorted at every depth; `src/verbs/shift.rs`'s unit test holds the file to that.
- `shift-test` and `shift-test.sb` are compiled in too (`src/cli/setup.rs`): they reach a session
  when `sandman setup` reinstalls them, which every upgrade runs
  (`docs/bank-format.html#shifts-sandbox`), and `tests/shift_test.rs` drives the repo's copy.
- A tool missing from `permissions.allow` is denied without asking under `dontAsk`, so a
  shift that needs a new tool needs it listed here first.

<!-- @stele -->
<!-- @end -->
