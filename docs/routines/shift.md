---
schedule: */5 * * * *
timeout: 300
on_error: claude
---

# Shift

The perpetual machine's clock. Every five minutes: if the open shift's session has ended, or has run past its handoff time, record its handoff and release the lock; then, if no shift is open, the machine is not paused and the five-hour window has room, open the next one. A shift is a Claude Code `--bg` session started through `spawn run`, supervised by Claude Code's own daemon, so it outlives this runner, the terminal and the desktop app. This routine never does the work; it only opens and closes shifts. The verbs are `sandman shift …`; their contract is sandman's `docs/BANK-FORMAT.md` § shifts, the plan is sandman's `docs/plan.md`.

## The creation week

Local time, America/New_York; Sunday is Day 1, Saturday the Sabbath. `sandman shift brief` reads the weekday and fills the theme into the brief; this table is the human copy.

| day | creation | theme | instrument | what a shift does |
| --- | --- | --- | --- | --- |
| Sun | Day 1 · Light divided from darkness | **Hunt** | `/gigasweep` → `/gigadebug` | sweep a repo for real defects, root-cause the confirmed ones, ship the repro as a regression test |
| Mon | Day 2 · Firmament; waters divided | **Redesign** | `/gigaredesign` → `/execute-plan` | rebuild the structure Sunday's findings and standing proposals indict, always-green staged migration |
| Tue | Day 3 · Land and seed | **Refactor** | `/gigarefine` | drive a repo's worst-scored module — tests as much as source — to its best form, behaviour preserved: the fewest statements, functions, branches and lines that still fulfil the feature set |
| Wed | Day 4 · Lights to mark seasons | **Measure** | `sandman score`, benchmarks | compute the score, run the perf and coverage passes, count statements, functions, branches and lines in source and tests, write the numbers where the Sabbath will read them |
| Thu | Day 5 · Swarms; be fruitful and multiply | **Harden** | `/security-review`, coverage, dependencies | bring the repo to 100 % coverage first (table stakes, end-to-end style), then close security findings and land dependency updates with their tests |
| Fri | Day 6 · Image; the work finished, very good | **Ship** | `/gigareview` | GO/NO-GO on the week's branches, merge the GOs, patch releases, `stele check` across the grove, CI green before sundown |
| Sat | Day 7 · Sabbath | **Rest** | the self-pass | no code commits: score the week from the trail, dream-pass, `/tighten` the machine's own prompts, rubrics and tests to their fewest lines, plan next week's targets, write the voyage entry; every seventh ISO week, the ablation |

## State

- `~/.sandman/shifts/current.json` — the lock: `{bg_id, handoff_by, path, repo, session_id, shift, started, theme}` while a shift is open, absent otherwise. `sandman shift brief` writes it, `sandman shift attach` adds the session ids, `sandman shift handoff` removes it. Never delete it by hand: `handoff --abandoned` is the release, and it records the abandonment on the trail.
- `~/.sandman/shifts/<id>/` — `brief.md` (the prompt the session got), `handoff.md` (the session writes it, then runs `sandman shift handoff --shift <id> --from-session` itself), `meta.json`.
- `~/.sandman/shifts/PAUSE` — the owner's off switch. While it exists no shift opens; an open one still closes. Only the owner removes it.
- `~/.local/state/claude/window.json` — the five-hour window as the harness statusline last saw it; `sandman shift clock --check` exits 1 under fifteen minutes left or at 90 % used, 3 when the file cannot be read or parsed, and 0 when the file is missing (the session then runs until the window cuts it off).

## Knobs

These reach `sandman shift brief` through this routine's environment (the tick's launchd agent); nothing reaches the session itself, which `--bg` starts from Claude Code's daemon.

- `SANDMAN_SHIFT_MIND` — orchestrator model, default `opus`. Fable takes the Sabbath self-pass and Friday's verdicts while quota allows.
- `SANDMAN_SHIFT_BUDGET_USD` — per-shift cost cap the brief states, default `15`.
- `SANDMAN_SHIFT_SKIP` — colon-separated repo slugs the rotation passes over.
- `SANDMAN_SHIFT_GRACE_MIN` — minutes past `handoff_by` before a still-working shift is stopped, default `20`.

## Spawn's answers

`spawn run` reads one JSON request per line and answers one JSON line: the result object or `{"error": …}`. `claude start` answers `{"id": <8-char bg id>, "session_id": <uuid>, …}`; `claude list` (with or without `ids`) answers `{"sessions": [row…]}`, each row carrying `id`, `name`, `state`, `status`; a session that has left the listing is simply absent. A row's `state` (`working` · `blocked` · `done` · `failed`) says whether the session is working; its `status` (`busy` · `idle`) does not — a finished session reports `status: busy`. `claude stop` with `id` ends a session and keeps its conversation. Every request is built with `python3 -I` `json.dumps`, never by hand — the brief is thousands of characters of markdown, quotes and backticks.

## Repair

- `unknown verb` from `sandman shift` (exit 2): the installed `~/.local/bin/sandman` predates the shift verbs. Install a binary built from sandman's trunk at a clean commit, as `sandman-reflect.md` describes; never build from a live checkout.
- `spawn` missing or answering `{"error": …}` in the close block: leave `current.json` alone and exit non-zero; the next tick retries. A session cannot be judged finished from an error.
- A wedged lock — `current.json` naming a `bg_id` the listing no longer shows — is exactly what the close block releases; run it by hand (`routine run shift`) rather than editing state.
- A shift stopped for running past `handoff_by` is abandoned with that reason on the trail; its branch and worktree stay for the next shift on that repo, whose brief carries the abandonment in the trail slot.
- `brief` refusing with `shift … is open`: `current.json` exists, so the open block should not have reached it; a concurrent hand run is the cause. Wait for the next tick.
- `brief` refusing on a template slot: `prompts/shift.md` in sandman's trunk names a slot the binary does not fill — a template ahead of the installed binary. Install the newer binary; do not edit the template here.
- Exit 124 is the routine watchdog and fires no catch. A start that timed out may still have started a session: the close block finds `current.json` without `bg_id`, stops any session named `shift <id>`, and abandons it.

```sh
# close: when the open shift's session has ended or overrun, record its handoff and release the lock
root="${SANDMAN_ROOT:-$HOME/.sandman}"
cur="$root/shifts/current.json"
[ -f "$cur" ] || exit 0
sm="$HOME/.local/bin/sandman"
sp="$HOME/.local/bin/spawn"
read -r id bg due < <(python3 -I -c '
import json, sys
c = json.load(open(sys.argv[1]))
print(c.get("shift", ""), c.get("bg_id", "-"), c.get("handoff_by", "-"))
' "$cur")
[ -n "$id" ] || { echo "current.json names no shift: $(cat "$cur")" >&2; exit 1; }
stop_named() {   # stop every session spawn lists under the name "shift <id>"
  req=$(python3 -I -c 'import json; print(json.dumps({"verb": "claude", "action": "list"}))')
  printf '%s\n' "$req" | "$sp" run | python3 -I -c '
import json, sys
answer = json.loads(sys.stdin.readline() or "{}")
for row in answer.get("sessions", []):
    if row.get("name") == "shift " + sys.argv[1] and row.get("state") in ("working", "blocked"):
        print(row["id"])
' "$1" | while read -r stray; do
    printf '%s\n' "$(python3 -I -c 'import json,sys; print(json.dumps({"verb":"claude","action":"stop","id":sys.argv[1]}))' "$stray")" | "$sp" run >/dev/null || true
  done
}
if [ "$bg" = "-" ]; then
  # briefed but never attached: the open block failed between brief and spawn; a session may still have started
  stop_named "$id"
  "$sm" shift handoff --shift "$id" --abandoned
  exit 0
fi
req=$(python3 -I -c 'import json,sys; print(json.dumps({"verb": "claude", "action": "list", "ids": [sys.argv[1]]}))' "$bg")
answer=$(printf '%s\n' "$req" | "$sp" run)
state=$(printf '%s' "$answer" | python3 -I -c '
import json, sys
answer = json.loads(sys.stdin.readline() or "{}")
if "error" in answer or "sessions" not in answer:
    print("error")
else:
    rows = answer["sessions"]
    print(rows[0].get("state", "") if rows else "gone")
')
case "$state" in
  done|failed|gone)
    "$sm" shift handoff --shift "$id" --from-session || "$sm" shift handoff --shift "$id" --abandoned
    ;;
  working|blocked)
    overrun=$(python3 -I -c '
import sys, datetime
due = sys.argv[1]
if due == "-":
    print("no"); sys.exit()
d = datetime.datetime.fromisoformat(due.replace("Z", "+00:00"))
late = datetime.datetime.now(datetime.timezone.utc) - d
print("yes" if late.total_seconds() > int(sys.argv[2]) * 60 else "no")
' "$due" "${SANDMAN_SHIFT_GRACE_MIN:-20}")
    if [ "$overrun" = "yes" ]; then
      printf '%s\n' "$(python3 -I -c 'import json,sys; print(json.dumps({"verb":"claude","action":"stop","id":sys.argv[1]}))' "$bg")" | "$sp" run >/dev/null
      "$sm" shift handoff --shift "$id" --from-session || "$sm" shift handoff --shift "$id" --abandoned
    fi
    ;;
  *)
    echo "spawn list for $bg answered: $answer" >&2
    exit 1
    ;;
esac
```

```sh
# open: if no shift is open, the machine is not paused and the window has room, start one
root="${SANDMAN_ROOT:-$HOME/.sandman}"
[ ! -f "$root/shifts/PAUSE" ] || exit 0
[ ! -f "$root/shifts/current.json" ] || exit 0
sm="$HOME/.local/bin/sandman"
sp="$HOME/.local/bin/spawn"
rc=0
"$sm" shift clock --check >/dev/null || rc=$?
[ "$rc" -ne 1 ] || exit 0   # the window is nearly spent: wait for the reset
[ "$rc" -eq 0 ] || exit "$rc"   # 3: the clock file is unreadable — a fault, not a wait
id=$("$sm" shift brief)
cwd=$("$sm" shift pick --path --shift "$id")
req=$(python3 -I -c '
import json, sys
brief, cwd, mind, shift = sys.argv[1:5]
print(json.dumps({
    "verb": "claude",
    "action": "start",
    "prompt": open(brief).read(),
    "cwd": cwd,
    "model": mind,
    "permission_mode": "auto",
    "name": "shift " + shift,
}))
' "$root/shifts/$id/brief.md" "$cwd" "${SANDMAN_SHIFT_MIND:-opus}" "$id")
answer=$(printf '%s\n' "$req" | "$sp" run)
ids=$(printf '%s' "$answer" | python3 -I -c '
import json, sys
answer = json.loads(sys.stdin.readline() or "{}")
print(answer.get("id", ""), answer.get("session_id", ""))
')
bg=${ids% *}
sid=${ids#* }
if [ -z "$bg" ]; then
  # the start may have succeeded without a parsable answer: stop anything already running under this name
  req=$(python3 -I -c 'import json; print(json.dumps({"verb": "claude", "action": "list"}))')
  printf '%s\n' "$req" | "$sp" run | python3 -I -c '
import json, sys
answer = json.loads(sys.stdin.readline() or "{}")
for row in answer.get("sessions", []):
    if row.get("name") == "shift " + sys.argv[1]:
        print(row["id"])
' "$id" | while read -r stray; do
    printf '%s\n' "$(python3 -I -c 'import json,sys; print(json.dumps({"verb":"claude","action":"stop","id":sys.argv[1]}))' "$stray")" | "$sp" run >/dev/null || true
  done
  "$sm" shift handoff --shift "$id" --abandoned
  echo "spawn start failed for $id: $answer" >&2
  exit 1
fi
"$sm" shift attach --shift "$id" --bg-id "$bg" --session-id "$sid" || {
  printf '%s\n' "$(python3 -I -c 'import json,sys; print(json.dumps({"verb":"claude","action":"stop","id":sys.argv[1]}))' "$bg")" | "$sp" run >/dev/null || true
  "$sm" shift handoff --shift "$id" --abandoned
  exit 1
}
```
