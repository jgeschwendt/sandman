# sandman

Memory engine for Claude sessions — sandman-format banks, session archiving, and the dream/reflect passes. A zero-dependency Rust CLI that plugs into Claude Code's hooks: `recall` on SessionStart, `take` on SessionEnd, three-mind consensus dreaming, a nightly reflect that writes the voyage log and keeps the banks. Design in `docs/DESIGN.md`, on-disk format in `docs/BANK-FORMAT.md`.

## Trail

`sandman trail` is the append-only event log every autonomous decision lands in — one
JSONL file per UTC day under `<root>/trail/`, every machine commit joined to it by
`Shift:`/`Event:` trailers. `sandman reconcile <event-id>` reverts the commits of a
revoked event and its descendants across the grove, on a `reconcile/<event-id>` branch
cut from each repo's trunk — never a merge: the machine lands by fast-forward only, so a
trailered merge refuses its repo for the operator; a shift id (`s-…`) in place of the event takes back the
whole shift.

```sh
sandman trail revoke s-20261016T1600Z --reason "wrong path"   # or an e-… event id
sandman reconcile s-20261016T1600Z --dry-run                  # the plan, nothing changed; exit 1 on a conflict or failure
```

The contract is `docs/BANK-FORMAT.md` § trail.

## Shift

`sandman shift` runs the perpetual machine's unit of work, one five-hour session at a
time; `~/.routines/shift.md` drives it every five minutes.

```sh
sandman shift clock --check     # remaining=3h56m used=6% … — exit 1 under 15 min or at 90 % used, 3 on an unreadable window file
sandman shift pick              # the rotation's next repo; --path for its trunk checkout
sandman shift brief             # compose prompts/shift.md into shifts/<id>/brief.md; prints the id
sandman shift brief --dry-run   # print the brief, write nothing
sandman shift handoff --shift <id> --from-session   # or --file PATH, or --abandoned; over 2,000 chars needs --truncate
sandman shift status            # the open shift and the clock
```

The contract is `docs/BANK-FORMAT.md` § shifts.

## Breathe

`sandman breathe` is the ruach's store — the feeds in `<root>/breath/sources.toml`, the
breaths filed from them, and the pass prompt a reading session starts from. It never
calls a model or the network; the session does the reading.

```sh
sandman breathe init                         # breath/, ledger/, passes/; seeds sources.toml
sandman breathe sources --due                # the feeds a skim reads now (unread for an hour)
sandman breathe pass --kind skim             # compose prompts/breathe.md into passes/<id>/; prints the id
sandman breathe file --url U --repo owner/name --title T --problem P --summary S --source NAME
sandman breathe list --repo owner/name       # state  repo  file  title
sandman breathe adopt <file>                 # or spike <file>, drop <file> --reason TEXT
sandman breathe close --id <id> --read N --filed M --dropped K --cost USD --cursor NAME=VALUE
sandman breathe reweight --dry-run           # +0.1 per adoption (cap 2.0), −0.1 per silent month (floor 0.2)
```

The contract is `docs/BANK-FORMAT.md` § breath.

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
