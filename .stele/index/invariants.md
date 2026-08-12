# Invariants

| claim | node | anchor |
| --- | --- | --- |
| single format authority — every bank write goes through this crate's commit path; every consumer uses the CLI contract and never re-implements slugging, `replaces` archiving, lineage, collision suffixes, or index regen | / | lm:format-authority |
| one config point — the data root is `$SANDMAN_ROOT`, else `~/.sandman`; nothing outside `src/paths.rs` names it | / | README.md#sandman |
