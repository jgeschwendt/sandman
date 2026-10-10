# prompts

```stele
kind: container
purpose: the machine's prompt templates — `shift.md`, Saturday's `sabbath.md` (`shift brief`) and `breathe.md` (`breathe pass`); their slot names are the contract (BANK-FORMAT § shifts, § breath)
```

## Working here

- A `{{name}}` is a slot: `sandman shift brief` and `sandman breathe pass` refuse a
  template naming a slot they do not fill, so add the slot in `src/verbs/shift.rs` (or
  `src/verbs/breathe.rs`) and BANK-FORMAT § shifts (or § breath) in the same change. A
  literal `{{` in prose reads as a slot.
- The installed binary reads the trunk checkout's `prompts/` (`$SANDMAN_PROMPTS`
  overrides) — an edit here goes live for the next shift once it reaches trunk, with no
  rebuild; a new slot needs the binary that fills it installed first.
- `tests/shift.rs` composes the real `shift.md` and `sabbath.md`, `tests/breathe.rs` the
  real `breathe.md`, so a template edit runs through the suite.

<!-- @stele -->
<!-- @end -->
