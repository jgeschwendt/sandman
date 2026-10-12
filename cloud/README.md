# cloud — the perpetual machine as a Claude Code routine

Nightly shifts run as a Claude Code cloud routine: each hourly fire is a fresh VM
holding the attached repos as clones, `setup.sh` has installed the tools, and the
routine's prompt (`prompts/cloud-shift.md`) runs `sandman shift open --cloud`,
follows the brief it prints, lands on each target's default branch and pushes,
then `shift handoff --cloud`. The machine's state travels through
`jgeschwendt/.sandman`: the open shift's `shifts/current.json` is committed there
and is the lock every VM sees (BANK-FORMAT § cloud).

## The routine

Created by hand at claude.ai/code (routines):

```text
name          perpetual-shift
repositories  jgeschwendt/bifrost        jgeschwendt/routines
              jgeschwendt/bridge         jgeschwendt/sandman
              jgeschwendt/claude         jgeschwendt/spawn
              jgeschwendt/gigantic.dev   jgeschwendt/stave
              jgeschwendt/grove          jgeschwendt/stele
              jgeschwendt/ingress        jgeschwendt/threads
              jgeschwendt/jgeschwendt    jgeschwendt/typescript
              jgeschwendt/jlg.io         jgeschwendt/ui
              jgeschwendt/pty            jgeschwendt/visor
              jgeschwendt/voice          jgeschwendt/voice-corpus
              jgeschwendt/.sandman       (the state)
              jgeschwendt/-              (the threads)
environment   perpetual
  network     Custom: the defaults (package registries, GitHub) + api.typesafe.ai
  setup       the contents of cloud/setup.sh
schedule      7 21-23,0-6 * * *   America/New_York (the UI converts local time;
              in UTC that is 7 1-10 * * * under EDT, 7 2-11 * * * under EST)
prompt        the contents of prompts/cloud-shift.md
```

The twenty grove repos are the rotation's: every `jgeschwendt/*` trunk under
`~/.grove/code` (`$SANDMAN_SHIFT_OWNERS`' default), nineteen in the manifest plus
`voice-corpus` from the tree. A repo left off the list is out of the rotation.

Ten fires a night, each shift handing off fifty minutes after it opens; the next
fire finds the lock released, or exits `busy` while it is younger than seventy
minutes. Saturday's night needs no second routine: `open --cloud` composes the
Sabbath's brief from `prompts/sabbath.md` on the 21:07 fire and refuses the rest of
the night with `sabbath kept`.

## Before the first fire

- `setup.sh` builds sandman from GitHub's `main`, so `shift open --cloud` must have
  landed there.
- `jgeschwendt/threads` is private: an anonymous clone in `setup.sh` fails unless
  the GitHub proxy also serves setup, and the session then builds `thread` from its
  attached clone.
- The Mac's `~/.routines/shift.md` stays paused (`~/.sandman/shifts/PAUSE`): its
  `current.json` is gitignored there, so a cloud VM cannot see a Mac shift's lock.
- The Mac's home-sync must pull `~/.sandman` and `~/.threads` before it pushes, or
  every night's cloud commits reject its pushes.
- The off switch is the routine's toggle; `shifts/PAUSE` is gitignored and never
  reaches a VM.
