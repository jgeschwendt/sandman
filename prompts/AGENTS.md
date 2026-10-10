# prompts

```stele
kind: container
purpose: the machine's prompt templates — `shift.md` is the dynamic shift prompt `sandman shift brief` fills; its slot names are the contract (`docs/BANK-FORMAT.md` § shifts)
```

## Working here

- A `{{name}}` is a slot: `sandman shift brief` refuses a template naming a slot it
  does not fill, so add the slot in `src/verbs/shift.rs` and BANK-FORMAT § shifts in
  the same change. A literal `{{` in prose reads as a slot.
- The installed binary reads the trunk checkout's `prompts/` (`$SANDMAN_PROMPTS`
  overrides) — an edit here goes live for the next shift once it reaches trunk, with no
  rebuild; a new slot needs the binary that fills it installed first.
- `tests/shift.rs` composes the real `shift.md`, so a template edit runs through the
  suite.

<!-- @stele -->
<!-- @end -->
