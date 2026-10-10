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
- `~/.local/state/claude/window.json` — the five-hour window as the harness statusline last saw it; `sandman shift clock --check` exits 1 with under `SANDMAN_SHIFT_MIN_WINDOW_MIN` minutes left or at 90 % used, 3 when the file is unreadable or of the wrong shape, and 0 when the file is missing (the session then runs until the window cuts it off).

## Knobs

These reach `sandman shift brief` through this routine's environment (the tick's launchd agent); nothing reaches the session itself, which `--bg` starts from Claude Code's daemon.

- `SANDMAN_SHIFT_MIND` — orchestrator model, default `opus`. Fable takes the Sabbath self-pass and Friday's verdicts while quota allows.
- `SANDMAN_SHIFT_BUDGET_USD` — per-shift cost cap the brief states, default `15`.
- `SANDMAN_SHIFT_SKIP` — colon-separated repo slugs the rotation passes over.
- `SANDMAN_SHIFT_GRACE_MIN` — minutes past `handoff_by` before a still-working shift is stopped, default `20`.
- `SANDMAN_SHIFT_MIN_WINDOW_MIN` — the least window a shift may open with, default `60`; `handoff_by` is the reset minus fifteen, so a shift always has at least 45 minutes.

## The sandbox

A shift runs in `dontAsk` — no prompts, no classifier, anything that would have asked is denied — inside Claude Code's strict Bash sandbox, with the profile `sandman shift profile` prints: writes only under `~/.grove/code`, `~/.sandman`, `~/.routines` and `~/.local/state/claude`; network only to the allowlisted hosts; credential files unreadable; force-pushes and the mutate-to-inspect git commands denied by rule. `git fetch`, `git pull`, `git push` and `grove sync` run outside the sandbox with the host's own git auth, because git over SSH fails inside the macOS sandbox and a `--bg` session inherits no environment. The contract is sandman's `docs/BANK-FORMAT.md` § shifts, "The sandbox".

## Spawn's answers

`spawn run` reads one JSON request per line and answers one JSON line: the result object or `{"error": …}`. `claude start` answers `{"id": <8-char bg id>, "session_id": <uuid>, …}`; `claude list` (with or without `ids`) answers `{"sessions": [row…]}`, each row carrying `id`, `name`, `state`, `status`; a session that has left the listing is simply absent. A row's `state` (`working` · `blocked` · `done` · `failed`) says whether the session is working; its `status` (`busy` · `idle`) does not — a finished session reports `status: busy`. `claude stop` with `id` ends a session and keeps its conversation. Every request is built with `python3 -I` `json.dumps`, never by hand — the brief is thousands of characters of markdown, quotes and backticks.

## Repair

- `unknown verb` from `sandman shift` (exit 2): the installed `~/.local/bin/sandman` predates the shift verbs. Install a binary built from sandman's trunk at a clean commit, as `sandman-reflect.md` describes; never build from a live checkout.
- `spawn` missing or answering `{"error": …}` in the close block: leave `current.json` alone and exit non-zero; the next tick retries. A session cannot be judged finished from an error.
- A wedged lock — `current.json` naming a `bg_id` the listing no longer shows — is exactly what the close block releases; run it by hand (`routine run shift`) rather than editing state.
- A shift stopped for running past `handoff_by` is abandoned with that reason on the trail; its branch and worktree stay for the next shift on that repo, whose brief carries the abandonment in the trail slot.
- `brief` refusing with `shift … is open`: `current.json` exists, so the open block should not have reached it; a concurrent hand run is the cause. Wait for the next tick.
- `brief` refusing on a template slot: `prompts/shift.md` in sandman's trunk names a slot the binary does not fill — a template ahead of the installed binary. Install the newer binary; do not edit the template here.
- `spawn start` answering `Workspace not trusted`: the picked trunk has never had Claude Code's trust prompt accepted, and `--bg` will not start there (`-p` would; `--bg` will not). Accepting trust is the owner's call, so never set `hasTrustDialogAccepted` yourself. `sandman shift pick` skips untrusted trunks and `shift pick --untrusted` lists them; abandon the shift with the cause as its `--reason`, then open a thread keyed `shift:trust:<repo>` asking the owner to run `claude` once in that path or add the repo to `SANDMAN_SHIFT_SKIP`. (observed 2026-10-10 · s-20261010T0225Z, jlg.io/main)
- Exit 124 is the routine watchdog and fires no catch. A start that timed out may still have started a session: the close block finds `current.json` without `bg_id`, stops any session named `shift <id>`, and abandons it.

```sh
# close: when the open shift's session has ended or overrun, record its handoff and release the lock
root="${SANDMAN_ROOT:-$HOME/.sandman}"
cur="$root/shifts/current.json"
[ -f "$cur" ] || exit 0
sm="$HOME/.local/bin/sandman"
sp="$HOME/.local/bin/spawn"
field() {   # one field of current.json; "-" when absent, null or empty
  python3 -I -c '
import json, sys
v = json.load(open(sys.argv[1])).get(sys.argv[2])
print(v if isinstance(v, str) and v else "-")
' "$cur" "$1"
}
id=$(field shift); bg=$(field bg_id); due=$(field handoff_by)
[ "$id" != "-" ] || { echo "current.json names no shift: $(cat "$cur")" >&2; exit 1; }
ask() {   # one spawn request; prints the answer; returns 1 when spawn fails, says nothing, or answers an error
  answer=$(printf '%s\n' "$1" | "$sp" run) || return 1
  [ -n "$answer" ] || return 1
  printf '%s\n' "$answer"
  printf '%s' "$answer" | python3 -I -c '
import json, sys
a = json.loads(sys.stdin.readline())
sys.exit(1 if "error" in a or not ("sessions" in a or "id" in a or a.get("ok")) else 0)
'
}
stop_session() {   # stop one bg session; a failed stop is a fault — never release the lock over a running session
  req=$(python3 -I -c 'import json,sys; print(json.dumps({"verb":"claude","action":"stop","id":sys.argv[1]}))' "$1")
  ask "$req" >/dev/null || { echo "spawn could not stop $1; lock kept" >&2; exit 1; }
}
stop_named() {   # stop every working or blocked session spawn lists under the name "shift <id>"
  req=$(python3 -I -c 'import json; print(json.dumps({"verb": "claude", "action": "list"}))')
  answer=$(ask "$req") || { echo "spawn list failed: $answer" >&2; exit 1; }
  strays=$(printf '%s' "$answer" | python3 -I -c '
import json, sys
answer = json.loads(sys.stdin.readline() or "{}")
for row in answer.get("sessions", []):
    if row.get("name") == "shift " + sys.argv[1] and row.get("state") in ("working", "blocked"):
        print(row["id"])
' "$1")
  for stray in $strays; do stop_session "$stray"; done
}
if [ "$bg" = "-" ]; then
  # briefed but never attached: the open block failed between brief and spawn; a session may still have started
  stop_named "$id"
  "$sm" shift handoff --shift "$id" --abandoned --reason "the open block failed between brief and attach; any session named shift $id was stopped"
  exit 0
fi
req=$(python3 -I -c 'import json,sys; print(json.dumps({"verb": "claude", "action": "list", "ids": [sys.argv[1]]}))' "$bg")
answer=$(ask "$req") || { echo "spawn list for $bg answered: $answer" >&2; exit 1; }
state=$(printf '%s' "$answer" | python3 -I -c '
import json, sys
rows = json.loads(sys.stdin.readline() or "{}").get("sessions", [])
print(rows[0].get("state", "") if rows else "gone")
')
case "$state" in
  done|failed|gone)
    "$sm" shift handoff --shift "$id" --from-session || "$sm" shift handoff --shift "$id" --abandoned
    ;;
  working|blocked)
    late=$(python3 -I -c '
import sys, datetime
due = sys.argv[1]
if due == "-":
    print(-1); sys.exit()
d = datetime.datetime.fromisoformat(due.replace("Z", "+00:00"))
print(int((datetime.datetime.now(datetime.timezone.utc) - d).total_seconds() // 60))
' "$due")
    if [ "$late" -gt "${SANDMAN_SHIFT_GRACE_MIN:-20}" ]; then
      stop_session "$bg"
      # always abandoned-with-reason: a handoff.md the session wrote is kept beneath the Abandoned line
      "$sm" shift handoff --shift "$id" --abandoned --reason "stopped at $(date -u +%FT%TZ): still $state $late min past handoff_by"
    fi
    ;;
  *)
    echo "spawn list for $bg answered an unknown state: $answer" >&2
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
rc=0
id=$("$sm" shift brief) || rc=$?
[ "$rc" -ne 1 ] || exit 0   # brief refused: the Sabbath is kept for today, or nothing to do — wait
[ "$rc" -eq 0 ] || exit "$rc"   # 2 or 3: a fault — template, trust, window file
cwd=$("$sm" shift pick --path --shift "$id")
profile=$("$sm" shift profile)
req=$(python3 -I -c '
import json, sys
brief, cwd, mind, shift, profile = sys.argv[1:6]
print(json.dumps({
    "verb": "claude",
    "action": "start",
    "prompt": open(brief).read(),
    "cwd": cwd,
    "model": mind,
    "permission_mode": "dontAsk",
    "settings": json.loads(profile),
    "name": "shift " + shift,
}))
' "$root/shifts/$id/brief.md" "$cwd" "${SANDMAN_SHIFT_MIND:-opus}" "$id" "$profile")
answer=$(printf '%s\n' "$req" | "$sp" run)
ids=$(printf '%s' "$answer" | python3 -I -c '
import json, sys
answer = json.loads(sys.stdin.readline() or "{}")
print(answer.get("id", ""), answer.get("session_id", ""))
')
bg=${ids% *}
sid=${ids#* }
if [ -z "$bg" ]; then
  # the start may have succeeded without a parsable answer: keep the lock (current.json has no bg_id) and let
  # the close block's briefed-but-never-attached path stop any session named "shift <id>" and abandon it
  echo "spawn start failed for $id: $answer" >&2
  exit 1
fi
"$sm" shift attach --shift "$id" --bg-id "$bg" --session-id "$sid" || { echo "attach failed for $id ($bg); the close block will stop and abandon it" >&2; exit 1; }
```
