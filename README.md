# sandman

Memory engine for Claude sessions — sandman-format banks, session archiving, and the dream/reflect passes. A zero-dependency Rust CLI that plugs into Claude Code's hooks: `recall` on SessionStart, `take` on SessionEnd, three-mind consensus dreaming, a nightly reflect that writes the voyage log and keeps the banks. Design in `docs/DESIGN.md`, on-disk format in `docs/BANK-FORMAT.md`.

## Trail

`sandman trail` is the append-only event log every autonomous decision lands in — one
JSONL file per UTC day under `<root>/trail/`, every machine commit joined to it by
`Shift:`/`Event:` trailers. `sandman reconcile <event-id>` reverts the commits of a
revoked event and its descendants across the grove, on a `reconcile/<event-id>` branch
per repo. The contract is `docs/BANK-FORMAT.md` § trail.

## Shift

`sandman shift` runs the perpetual machine's unit of work, one five-hour session at a
time; `~/.routines/shift.md` drives it every five minutes.

```sh
sandman shift clock --check     # remaining=3h56m used=6% … — exit 1 under 15 min or at 90 % used
sandman shift pick              # the rotation's next repo; --path for its trunk checkout
sandman shift brief             # compose prompts/shift.md into shifts/<id>/brief.md; prints the id
sandman shift brief --dry-run   # print the brief, write nothing
sandman shift handoff --shift <id> --from-session   # or --file PATH, or --abandoned
sandman shift status            # the open shift and the clock
```

The contract is `docs/BANK-FORMAT.md` § shifts.

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
