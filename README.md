# sandman

Memory engine for Claude sessions — sandman-format banks, session archiving, and the dream/reflect passes. A zero-dependency Rust CLI that plugs into Claude Code's hooks: `recall` on SessionStart, `take` on SessionEnd, three-mind consensus dreaming, a nightly reflect that writes the voyage log and keeps the banks. Design in `docs/DESIGN.md`, on-disk format in `docs/BANK-FORMAT.md`.

## MCP

`sandman mcp` serves the banks as a tool server on stdio — JSON-RPC 2.0, one message
per line, three tools: `recall` (what a session would be primed with), `remember`
(one memory, committed through the same path the CLI uses) and `banks` (the bank
listing, each key marked live when its directory still exists). Any MCP client reaches
the real commit lock, so no consumer re-implements the format.

```sh
claude mcp add --scope user sandman -- "$HOME/.local/bin/sandman" mcp
```

The contract is `docs/MCP.md`.

## License

Copyright Joshua Geschwendt.

Licensed under the [PolyForm Noncommercial License 1.0.0](https://polyformproject.org/licenses/noncommercial/1.0.0) — the full text is in [`LICENSE.md`](LICENSE.md). Commercial use requires a separate license from the author.

External contributions are not accepted.
