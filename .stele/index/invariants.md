# Invariants

| claim | node | anchor |
| --- | --- | --- |
| single format authority — every bank write goes through this crate's commit path; every consumer uses the CLI contract and never re-implements slugging, `replaces` archiving, lineage, collision suffixes, or index regen | / | lm:format-authority |
| one config point — the data root is `$SANDMAN_ROOT`, else `~/.sandman`; nothing outside `src/paths.rs` names it | / | README.md#sandman |
| dream owns what "queued" means — every reader of the queue goes through `dream::depth` / `dream::pending`, which count only undreamed pointers; nothing counts `.recent/*.json` off disk for itself. Take's dream trigger drifted from the run this way: spent pointers inflated the depth, so every ending past the tenth spawned a dream with nothing to route | src | lm:queue-definition |
