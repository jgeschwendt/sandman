# profiles

```stele
kind: container
purpose: the Claude Code settings in `session.json` that shift and breath sessions run under (`sandman shift profile`); the contract is `docs/BANK-FORMAT.md` § shifts, The sandbox
```

## Working here

- Compiled into the binary (`include_str!` in `src/verbs/shift.rs`): an edit reaches a
  session only through a rebuilt, installed binary.
- Keys sorted at every depth; `src/verbs/shift.rs`'s unit test holds the file to that.
- A tool missing from `permissions.allow` is denied without asking under `dontAsk`, so a
  shift that needs a new tool needs it listed here first.

<!-- @stele -->
<!-- @end -->
