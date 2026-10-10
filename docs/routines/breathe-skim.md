---
schedule: 17 * * * *
timeout: 900
on_error: claude
---

# Breathe · skim

The ruach's hourly pass. Research is its own perpetual engine, outside the creation week: every hour at :17 it skims what the due feeds published since their cursors, against the open problems, and files what bears. This routine never reads a feed itself; it composes the pass prompt with `sandman breathe pass --kind skim` and starts a Claude Code `--bg` session through `spawn run`, exactly as `shift.md` does, with cwd at the breath store. The session reads the due sources, files breaths with provenance, and ends with `sandman breathe close`. The contract is sandman's `docs/BANK-FORMAT.md` § breath; the plan is sandman's `docs/plan.md` § The ruach.

## State

- `~/.sandman/breath/sources.toml` — the feeds, their weights, budgets and cursors; `sandman breathe reweight` (the Sabbath) rewrites it.
- `~/.sandman/breath/passes/<id>/` — one record per pass: `pass.json` with `started`, the sources it was given, and after `close` the counters and `ended`; `prompt.md`, the prompt the session got. A pass without `ended` older than its timeout died; the Sabbath sweeps those, this routine does not.
- `~/.sandman/breath/<date>-<slug>.md` — the breaths. `~/.sandman/breath/problems.md` — what the shift engine wants answered, written by `sandman shift brief`.
- `~/.sandman/shifts/PAUSE` — the machine's one off switch; while it exists no pass starts.

## Knobs

- `SANDMAN_BREATH_MIND` — the pass model, default `opus`. The skim spawns no readers.
- `SANDMAN_BREATH_BUDGET_USD` — the day's cost cap the prompt states, default `10`; the engine stops when today's passes have spent it.

## Repair

- `unknown verb` (exit 2): the installed `~/.local/bin/sandman` predates `breathe`; install a trunk build as `sandman-reflect.md` describes.
- `spawn` answering `{"error": …}`: the pass record stays without session ids and without `ended`; nothing else to undo. The next tick starts a fresh pass.
- Two passes of the same kind in flight is not an error — passes are independent and the budget line in each prompt is the brake.

```sh
# open: unless paused, compose a skim pass and start its session
root="${SANDMAN_ROOT:-$HOME/.sandman}"
[ ! -f "$root/shifts/PAUSE" ] || exit 0
sm="$HOME/.local/bin/sandman"
rc=0
id=$("$sm" breathe pass --kind skim) || rc=$?
[ "$rc" -ne 1 ] || exit 0   # nothing due, or the day's budget is spent: wait
[ "$rc" -eq 0 ] || exit "$rc"   # 2 or 3: a fault — unknown verb, bad sources.toml, missing template
req=$(python3 -I -c '
import json, sys
prompt, cwd, mind, pid = sys.argv[1:5]
print(json.dumps({
    "verb": "claude",
    "action": "start",
    "prompt": open(prompt).read(),
    "cwd": cwd,
    "model": mind,
    "permission_mode": "auto",
    "name": "breath skim " + pid,
}))
' "$root/breath/passes/$id/prompt.md" "$root/breath" "${SANDMAN_BREATH_MIND:-opus}" "$id")
answer=$(printf '%s\n' "$req" | "$HOME/.local/bin/spawn" run)
ids=$(printf '%s' "$answer" | python3 -I -c '
import json, sys
answer = json.loads(sys.stdin.readline() or "{}")
print(answer.get("id", ""), answer.get("session_id", ""))
')
bg=${ids% *}
sid=${ids#* }
[ -n "$bg" ] || { echo "spawn start failed for pass $id: $answer" >&2; exit 1; }
"$sm" breathe attach --id "$id" --bg-id "$bg" --session-id "$sid"
```
