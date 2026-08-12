# sandman

Memory engine for Claude sessions — sandman-format banks, session archiving, and the dream/reflect passes. A zero-dependency Rust CLI that plugs into Claude Code's hooks: `recall` on SessionStart, `take` on SessionEnd, three-mind consensus dreaming, a nightly reflect. Design in `docs/DESIGN.md`, on-disk format in `docs/BANK-FORMAT.md`.
