---
schedule: 17 * * * *
timeout: 900
on_error: claude
---

# Breathe · skim

The ruach's hourly pass. Research is its own perpetual engine, outside the creation week: every hour at :17 it skims what the due feeds published since their cursors, against the open problems, and files what bears. This routine never reads a feed itself; it composes the pass prompt with `sandman breathe pass --kind skim` and starts a Claude Code `--bg` session through `dispatch run`, exactly as `shift.md` does, with cwd at the breath store. The session reads the due sources, files breaths with provenance, and ends with `sandman breathe close`. Each tick first removes the sessions of closed passes from Claude Code's listing, where a finished session otherwise stays forever. The contract is sandman's `docs/bank-format.html#breath`; the plan is sandman's `docs/index.html` § The perpetual shift, The ruach.

## State

- `~/.sandman/breath/sources.toml` — the feeds, their weights, budgets and cursors; `sandman breathe reweight` (the Sabbath) rewrites it.
- `~/.sandman/breath/passes/<id>/` — one record per pass: `pass.json` with `started`, the sources it was given, and after `close` the counters and `ended`; `prompt.md`, the prompt the session got. A pass without `ended` older than its timeout (skim 15 min, deep 60 min) is dead: the next `breathe pass` marks it so and charges `SANDMAN_BREATH_DEAD_COST_USD` (default 2) to the day it started.
- `~/.sandman/breath/<date>-<slug>.md` — the breaths. `~/.sandman/breath/problems.md` — what the shift engine wants answered, written by `sandman shift brief`.
- `~/.sandman/shifts/PAUSE` — the machine's one off switch; while it exists no pass starts.

## The sandbox

A pass runs in `dontAsk` inside Claude Code's strict Bash sandbox with the profile `sandman shift profile --kind breath` prints — the same boundary as `shift.md` § The sandbox: writes under the machine's own trees only, network to the allowlisted feeds and GitHub, credentials unreadable.

## Knobs

- `SANDMAN_BREATH_MIND` — the pass model, default `opus`. The skim spawns no readers.
- `SANDMAN_BREATH_BUDGET_USD` — the day's cost cap the prompt states, default `10`; the engine stops when today's passes have spent it.

## Repair

- `unknown verb` (exit 2): the installed `~/.local/bin/sandman` predates `breathe`; install a trunk build as `sandman-reflect.md` describes.
- `dispatch` answering `{"error": …}`: the pass record stays without session ids and without `ended`; nothing else to undo. The next tick starts a fresh pass.
- A second pass of the same kind while one is in flight is refused by `breathe pass` (exit 1, `pass in flight`) until the first closes or outlives its timeout, at which point it is marked dead and charged `SANDMAN_BREATH_DEAD_COST_USD` (default 2) against the day; a skim and a deep pass may run together.

```sh
# close: remove every "breath skim" session whose pass has ended — closed, or marked dead — stopping it first if it still runs
root="${SANDMAN_ROOT:-$HOME/.sandman}"
sp="$HOME/.local/bin/dispatch"
ask() {   # one dispatch request; prints the answer; returns 1 when dispatch fails, says nothing, or answers an error
  answer=$(printf '%s\n' "$1" | "$sp" run) || return 1
  [ -n "$answer" ] || return 1
  printf '%s\n' "$answer"
  printf '%s' "$answer" | python3 -I -c '
import json, sys
a = json.loads(sys.stdin.readline())
sys.exit(1 if "error" in a or not ("sessions" in a or a.get("ok")) else 0)
'
}
request() {   # one claude request for one bg id
  python3 -I -c 'import json,sys; print(json.dumps({"verb":"claude","action":sys.argv[1],"id":sys.argv[2]}))' "$1" "$2"
}
req=$(python3 -I -c 'import json; print(json.dumps({"verb": "claude", "action": "list"}))')
answer=$(ask "$req") || { echo "dispatch list failed: $answer" >&2; exit 0; }
ended=$(printf '%s' "$answer" | python3 -I -c '
import json, os, sys
for row in json.loads(sys.stdin.readline() or "{}").get("sessions", []):
    name = (row.get("name") or "").split(" ")
    if name[:2] != ["breath", sys.argv[2]] or len(name) != 3:
        continue
    try:
        record = json.load(open(os.path.join(sys.argv[1], name[2], "pass.json")))
    except (OSError, ValueError):
        continue
    if record.get("ended"):
        print(row["id"], "stop" if row.get("state") in ("working", "blocked") else "rm")
' "$root/breath/passes" skim)
stopped=$(printf '%s\n' "$ended" | awk '$2 == "stop" { print $1 }')
for bg in $stopped; do ask "$(request stop "$bg")" >/dev/null || echo "dispatch could not stop $bg" >&2; done
[ -z "$stopped" ] || sleep 3   # a stopped session's SessionEnd hook races the removal of its job dir
for bg in $(printf '%s\n' "$ended" | awk '{ print $1 }'); do
  ask "$(request rm "$bg")" >/dev/null || echo "dispatch could not rm $bg; left in the listing" >&2
done
```

```sh
# open: unless paused, compose a skim pass and start its session
root="${SANDMAN_ROOT:-$HOME/.sandman}"
[ ! -f "$root/shifts/PAUSE" ] || exit 0
sm="$HOME/.local/bin/sandman"
rc=0
id=$("$sm" breathe pass --kind skim) || rc=$?
[ "$rc" -ne 1 ] || exit 0   # nothing due, budget spent, a pass of this kind in flight, or the lock held: wait
[ "$rc" -eq 0 ] || exit "$rc"   # 2 or 3: a fault — unknown verb, bad sources.toml, missing template
profile=$("$sm" shift profile --kind breath)
req=$(python3 -I -c '
import json, sys
prompt, cwd, mind, pid, profile = sys.argv[1:6]
print(json.dumps({
    "verb": "claude",
    "action": "start",
    "prompt": open(prompt).read(),
    "cwd": cwd,
    "model": mind,
    "permission_mode": "dontAsk",
    "settings": json.loads(profile),
    "name": "breath skim " + pid,
}))
' "$root/breath/passes/$id/prompt.md" "$root/breath" "${SANDMAN_BREATH_MIND:-opus}" "$id" "$profile")
answer=$(printf '%s\n' "$req" | "$HOME/.local/bin/dispatch" run)
ids=$(printf '%s' "$answer" | python3 -I -c '
import json, sys
answer = json.loads(sys.stdin.readline() or "{}")
print(answer.get("id", ""), answer.get("session_id", ""))
')
bg=${ids% *}
sid=${ids#* }
[ -n "$bg" ] || { echo "dispatch start failed for pass $id: $answer" >&2; exit 1; }
"$sm" breathe attach --id "$id" --bg-id "$bg" --session-id "$sid"
```
