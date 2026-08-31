# sandman

Memory engine for Claude sessions — sandman-format banks, session archiving, and the dream/reflect passes. A zero-dependency Rust CLI that plugs into Claude Code's hooks: `recall` on SessionStart, `take` on SessionEnd, three-mind consensus dreaming, a nightly reflect. Design in `docs/DESIGN.md`, on-disk format in `docs/BANK-FORMAT.md`.

## License

Copyright Joshua Geschwendt.

Licensed under the [PolyForm Noncommercial License 1.0.0](https://polyformproject.org/licenses/noncommercial/1.0.0) — the full text is in [`LICENSE.md`](LICENSE.md). Commercial use requires a separate license from the author.

External contributions are not accepted.
