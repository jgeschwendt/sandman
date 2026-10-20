# prompts

```stele
kind: container
purpose: the machine's prompts — `shift.md`, `sabbath.md`, `breathe.md`, whose slot names are the contract (docs/bank-format.html#shifts, docs/bank-format.html#breath), and the cloud routine's `cloud-shift.md`
```

## Working here

- A `{{name}}` is a slot: `sandman shift brief` and `sandman breathe pass` refuse a
  template naming a slot they do not fill, so add the slot in `src/verbs/shift.rs` (or
  `src/verbs/breathe.rs`) and docs/bank-format.html#shifts (or docs/bank-format.html#breath) in the same change. A
  literal `{{` in prose reads as a slot.
- The installed binary reads the trunk checkout's `prompts/` (`$SANDMAN_PROMPTS`
  overrides) — an edit here goes live for the next shift once it reaches trunk, with no
  rebuild; a new slot needs the binary that fills it installed first.
- `cloud-shift.md` is no template: it is pasted into the routine `perpetual-shift` as its
  prompt (`cloud/README.md`), so an edit reaches the cloud only when the owner re-pastes
  it; the brief it runs comes from `shift.md`/`sabbath.md` through `shift open --cloud`.
- `tests/shift.rs` composes the real `shift.md` and `sabbath.md`, `tests/breathe.rs` the
  real `breathe.md`, so a template edit runs through the suite.

<!-- @stele -->
<!-- @end -->
