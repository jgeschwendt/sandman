# breath

```stele
kind: container
purpose: the ruach's seed — `sources.toml`, the feed list `sandman breathe init` copies to `<root>/breath/` when absent; its schema is `docs/BANK-FORMAT.md` § breath
```

## Working here

- The seed is compiled into the binary (`include_str!` in `src/verbs/breathe.rs`): an
  edit here reaches a store only through a rebuilt binary and only where `sources.toml`
  is still absent. The live feeds are the data root's copy, which `breathe reweight`
  rewrites.
- Keep it canonical — tables sorted by name, keys in the schema's order, every
  `bears_on` an `owner/name` slug or `"*"` — so the first `reweight` diff shows only
  what changed; `tests/breathe.rs` holds the seed to that.

<!-- @stele -->
<!-- @end -->
