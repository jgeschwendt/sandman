---
schedule: */5 * * * *
timeout: 300
on_error: claude
---

# Shift

The perpetual machine's clock. Every five minutes: if the open shift's session has ended, or has run past its handoff time, record its handoff and release the lock; then, if no shift is open, the machine is not paused, it is night and the five-hour window has room, open the next one. A shift is a Claude Code `--bg` session started through `dispatch run`, supervised by Claude Code's own daemon, so it outlives this runner, the terminal and the desktop app. This routine never does the work; it opens and closes shifts and carries what they landed to GitHub — the session cannot reach it, this host can, `gh` authenticated as the owner. Trunk is never pushed: every tick opens with the landing sweep, which opens each handed-off shift's `landing` (`sandman shift land`) as a pull request, reads every open one back, and merges by rebase the green ones auto-merge will not (a queue needs an organization-owned repo), asking GitHub to rebase any branch behind main; red CI, a refused push and a pull request closed unmerged are corrections on the trail, once each; a merge is a `change` event and fast-forwards the local trunk when it is clean (sandman's `docs/bank-format.html#shifts-landing`). The block runs with mise's shims and `~/.local/bin` on `PATH`, so the repos' pre-push gates find their tools under launchd's bare one. The sweep runs before the close block's "no open shift" exit, because a shift that hands itself off (`--from-session`) has already removed `current.json` by the time the tick looks. The same sweep reaps: every ended shift and breath pass not yet `reaped` has its session `claude rm`ed once dispatch lists it finished, its transcript taken into sandman's archive (`take --force`), and `reaped` stamped — the owner keeps no machine session in `claude agents` or the desktop app. Closing records the handoff. The verbs are `sandman shift …`; their contract is sandman's `docs/bank-format.html#shifts`, the plan is sandman's `docs/index.html` § The perpetual shift.

## The creation week

Local time, America/New_York; Sunday is Day 1, Saturday the Sabbath. The owner works by day; the machine works nights: a night of weekday D runs D 21:00 → D+1 07:00 New York and carries D's theme, so Monday 02:00 is still Sunday's Hunt and the Sabbath is Saturday's night. `sandman shift clock --check` refuses by day and within `SANDMAN_SHIFT_MIN_WINDOW_MIN` of dawn, and `handoff_by` never runs past 07:00, so by day the tick only closes a shift that overran. `sandman shift brief` reads the night's day and fills the theme into the brief; this table is the human copy.

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
- `~/.sandman/shifts/<id>/` — `brief.md` (the prompt the session got), `handoff.md` (the session writes it, then runs `sandman shift handoff --shift <id> --from-session` itself), `meta.json` (`handoff` copies the session ids in from `current.json`; the reap stamps `reaped`; `shift land` writes `landing`, the sweep's `shift pr` writes `pr`).
- `~/.sandman/shifts/PAUSE` — the owner's off switch. While it exists no shift opens; an open one still closes. Only the owner removes it.
- `~/.local/state/claude/window.json` — the five-hour window as the harness statusline last saw it; `sandman shift clock --check` exits 1 by day, with under `SANDMAN_SHIFT_MIN_WINDOW_MIN` minutes of night or window left, or at 90 % used, 3 when the file is unreadable or of the wrong shape, and 0 when the file is missing or stale — over 30 minutes old, as it is overnight with no interactive session to refresh it (the session then runs until the window or the runner's timeout cuts it off).

## Knobs

These reach `sandman shift brief` through this routine's environment (the tick's launchd agent); nothing reaches the session itself, which `--bg` starts from Claude Code's daemon.

- `SANDMAN_SHIFT_MIND` — orchestrator model, default `opus`, under `~/.sandman/shifts/rotation.toml`'s `[mind]` (per theme, else `default`). Fable takes the Sabbath self-pass and Friday's verdicts while quota allows.
- `SANDMAN_SHIFT_BUDGET_USD` — per-shift cost cap the brief states, default `15`.
- `SANDMAN_SHIFT_SKIP` — colon-separated repo slugs the rotation passes over.
- `SANDMAN_SHIFT_GRACE_MIN` — minutes past `handoff_by` before a still-working shift is stopped, default `20`.
- `SANDMAN_SHIFT_MIN_WINDOW_MIN` — the least window, and the least night, a shift may open with, default `60`; `handoff_by` is the reset minus fifteen, never past dawn, so a shift always has at least 45 minutes.
- `SANDMAN_SHIFT_NIGHT` — the night, New York hours `HH-HH`, default `21-07`, under `rotation.toml`'s `night`, which every verb and the report server (per request) read, so a night set there needs no agent restart.

`rotation.toml` is the owner's one knob file — `night`, `off`, `rounds`, `skim_minutes`, `[mind]`, `[dream]`; the contract is sandman's `docs/bank-format.html#shifts-rotations-file`. `pick` exiting 1 and `brief` 3 with `every rotation repo is spent` means each repo has had its `rounds` this night: idle until the next.

## The sandbox

A shift runs in `dontAsk` — no prompts, no classifier, anything that would have asked is denied — inside Claude Code's strict Bash sandbox, with the profile `sandman shift profile` prints: writes only under `~/.grove`, `~/.sandman`, `~/.routines`, `~/.docket`, `~/.local/state/claude`, mise's state and the toolchain caches (`~/.bun`, `~/.cache`, `~/.cargo`, `~/.hex`, `~/.mix`, `~/Library/Caches`; never their `bin/` or config), never `.claude/` config or `~/.claude`; no project `.mcp.json` server; network only to the allowlisted hosts; credential files unreadable; force-pushes and the mutate-to-inspect git commands denied by rule. Tests run as `~/.local/bin/shift-test [-C <dir>] [--log <file>] <command>` — the whole Bash call, or it stays sandboxed and exits 71 — the one other exclusion, inside its own narrower Seatbelt profile (`profiles/shift-test.sb`: credentials unreadable, writes to checkouts, caches and temp, loopback only). `git fetch`, `git pull`, `git push` and `grove sync` are `excludedCommands`, yet `git push` from a session still fails at the sandbox proxy ("This proxy requires authentication", exit 128; observed 2026-10-11), so a shift records its landing with `sandman shift land` and the landing sweep pushes the branch and opens the pull request. The contract is sandman's `docs/bank-format.html#shifts-sandbox`.

## Dispatch's answers

`dispatch run` reads one JSON request per line and answers one JSON line: the result object or `{"error": …}`. `claude start` answers `{"id": <8-char bg id>, "session_id": <uuid>, …}`; `claude list` (with or without `ids`) answers `{"sessions": [row…]}`, each row carrying `id`, `name`, `state`, `status`; a session that has left the listing is simply absent. A row's `state` (`working` · `blocked` · `done` · `failed` · `stopped`) says whether the session is working; its `status` (`busy` · `idle`) does not — a finished session reports `status: busy`. `claude stop` with `id` ends a session and keeps its conversation; `claude rm` with `id` deletes a finished one, which otherwise stays in the owner's `claude agents` view forever. A stopped session's SessionEnd hook races the removal of its job dir, so `rm` waits three seconds after a `stop`; one that ended on its own is removed at once. Every request is built with `python3 -I` `json.dumps`, never by hand — the brief is thousands of characters of markdown, quotes and backticks.

## Repair

- `unknown verb` from `sandman shift` (exit 2): the installed `~/.local/bin/sandman` predates the shift verbs. The install agent `dev.gigantic.sandman.install` brings it to a clean trunk within fifteen minutes (log `~/Library/Logs/dev.gigantic.sandman.install.log`; a dirty trunk is skipped and named there); never build from a live checkout.
- `dispatch` missing or answering `{"error": …}` in the close block: leave `current.json` alone and exit non-zero; the next tick retries. A session cannot be judged finished from an error.
- A wedged lock — `current.json` naming a `bg_id` the listing no longer shows — is exactly what the close block releases; run it by hand (`routine run shift`) rather than editing state.
- A shift stopped for running past `handoff_by` is abandoned with that reason on the trail; its branch and worktree stay for the next shift on that repo, whose brief carries the abandonment in the trail slot.
- `brief` refusing with `shift … is open`: `current.json` exists, so the open block should not have reached it; a concurrent hand run is the cause. Wait for the next tick.
- `brief` refusing on a template slot: `prompts/shift.md` in sandman's trunk names a slot the binary does not fill — a template ahead of the installed binary. Install the newer binary; do not edit the template here.
- `dispatch start` answering `Workspace not trusted`: the picked trunk has never had Claude Code's trust prompt accepted, and `--bg` will not start there (`-p` would; `--bg` will not). Accepting trust is the owner's call, so never set `hasTrustDialogAccepted` yourself. `sandman shift pick` skips untrusted trunks and `shift pick --untrusted` lists them; abandon the shift with the cause as its `--reason`, then open a thread keyed `shift:trust:<repo>` asking the owner to run `claude` once in that path or add the repo to `SANDMAN_SHIFT_SKIP`. (observed 2026-10-10 · s-20261010T0225Z, jlg.io/main)
- A branch push the sweep finds rejected (a red pre-push gate, a ruleset) is a `correction` event on the trail naming the repo, the branch and the shift, never a fault: the tick goes on, and the landing waits. It is recorded once per (shift, head): `$ROUTINE_STATE_DIR/rejected/<shift>` holds the refused sha (written before the push, so a gate the watchdog kills mid-push counts), the landing is skipped until its head moves or six hours pass, and a retry refused again adds nothing to the trail. Every push runs the repo's pre-push gate, minutes of tests; three red trunks retried each tick ran the block past `timeout` and logged 150–267 identical corrections a night (observed 2026-10-12/13).
- A `CI red on PR` correction is the next shift's on that repo: its brief lists the pull request under `prs.open` marked `CI red`, and it branches from it, fixes, and lands on its own `shift/<id>`; `shift land` stamps the red one `superseded_by`, and the merge pass closes it. A `gh` that fails (network, auth) skips that pull request for the tick.
- A session dispatch lists `working` with no transcript (its row has no `last_activity`) ten minutes after `started` never took its first turn: a `--bg` `dontAsk` session cannot answer a start-up prompt such as a project `.mcp.json` server's approval. The close block stops it, abandons the shift with `session never started (a project MCP prompt?)` and appends a `correction`; the fix is the server's name in the profile's `disabledMcpjsonServers` (sandman's `docs/bank-format.html#shifts-sandbox`). (observed 2026-10-12/13 · stele, s-20261012T0855Z and s-20261013T0816Z, each stalled until handoff_by)
- Exit 124 is the routine watchdog and fires no catch. A start that timed out may still have started a session: the close block finds `current.json` without `bg_id`, stops any session named `shift <id>`, and abandons it.

```sh
# sweep: every landing becomes a pull request and CI decides — auto-merge or this sweep merges it by rebase (a queue needs an organization-owned repo), a branch behind main is rebased by GitHub; trunk is never pushed
export PATH="$HOME/.local/bin:$HOME/.local/share/mise/shims:$PATH"   # launchd's bare PATH hides mise's tools from the pre-push gates
sm="$HOME/.local/bin/sandman"
root="${SANDMAN_ROOT:-$HOME/.sandman}"
# one line per shift whose landing is unsettled: id repo path branch head theme ended superseded_by number url auto red, "-" for none
landings=$(python3 -I -c '
import glob, json, sys
for path in sorted(glob.glob(sys.argv[1] + "/shifts/s-*/meta.json")):
    try:
        m = json.load(open(path))
        land, pr = m["landing"], m.get("pr") or {}
        row = [m.get("shift"), m.get("repo"), m.get("path"), land.get("branch"), land.get("head"), m.get("theme"), m.get("ended"),
               land.get("superseded_by"), pr.get("number"), pr.get("url"), pr.get("auto"), pr.get("red")]
    except (OSError, ValueError, LookupError, TypeError, AttributeError):
        continue
    if "merged" not in pr and "closed" not in pr:
        print(" ".join("-" if v is None else str(v).lower() if isinstance(v, bool) else str(v) for v in row))
' "$root")
# merge pass first, so a pull request opened this tick is first read on the next, its checks registered by then
while read -r id repo path branch head theme ended sup number url auto red <&3; do
  [ "${number:--}" != - ] || continue   # the heredoc of no landings is one empty line
  view=$(gh pr view "$number" --repo "$repo" --json state,mergeStateStatus,statusCheckRollup,headRefOid) || continue
  read -r verdict status prhead failing <<VIEW
$(printf '%s' "$view" | python3 -I -c '
import json, sys
v = json.load(sys.stdin)
red, pending = [], False
for check in v.get("statusCheckRollup") or []:
    said = (check.get("conclusion") or check.get("state") or "").upper()
    if check.get("status", "COMPLETED") != "COMPLETED" or said in ("", "PENDING", "EXPECTED"):
        pending = True
    elif said not in ("SUCCESS", "SKIPPED", "NEUTRAL"):
        red.append(check.get("name") or check.get("context") or "?")
status = v.get("mergeStateStatus") or "UNKNOWN"
if status == "DIRTY":
    red.append("conflicts with main")
state = v.get("state")
verdict = state.lower() if state in ("MERGED", "CLOSED") else "red" if red else "pending" if pending or status == "UNKNOWN" else "green"
print(verdict, status, v.get("headRefOid") or "-", ", ".join(red))
')
VIEW
  case $verdict in
    merged)
      "$sm" shift pr --shift "$id" --merged
      "$sm" trail append --kind change --shift "$id" --repo "$repo" --branch "$branch" --rationale "merged PR $url" >/dev/null
      # the local trunk follows when clean, as grove sync would within five minutes: never ahead of origin, never dirtied
      if git -C "$path" fetch -q origin main && [ -z "$(git -C "$path" status --porcelain --untracked-files=no)" ]; then
        git -C "$path" merge -q --ff-only '@{upstream}' || :
      fi ;;
    closed)
      "$sm" shift pr --shift "$id" --closed
      [ "$sup" != - ] || "$sm" trail append --kind correction --shift "$id" --repo "$repo" --branch "$branch" --rationale "PR $url closed unmerged" >/dev/null ;;
    red|pending|green)
      if [ "$sup" != - ]; then   # a later shift's landing carries this work on
        gh pr close "$number" --repo "$repo" --delete-branch --comment "Superseded by $sup's pull request." >/dev/null && "$sm" shift pr --shift "$id" --closed
      elif [ "$verdict" = red ]; then   # one correction per (shift, head)
        [ "$red" = "$prhead" ] || "$sm" trail append --kind correction --shift "$id" --repo "$repo" --branch "$branch" --rationale "CI red on PR $url: $failing" >/dev/null
        [ "$red" = "$prhead" ] || "$sm" shift pr --shift "$id" --red "$prhead"
      elif [ "$status" = BEHIND ]; then   # strict required checks hold auto-merge until the branch is current: GitHub rebases it, the next tick reads the new head
        gh pr update-branch "$number" --repo "$repo" --rebase >/dev/null 2>&1 && echo "updated PR $url onto main" >&2 || :
      else
        if [ "$auto" = - ] && [ "$status" != UNKNOWN ]; then
          # gh merges at once, CI or not, a branch nothing blocks (CLEAN, HAS_HOOKS, UNSTABLE): only a blocked one tries auto-merge
          auto=false
          case $status in CLEAN | HAS_HOOKS | UNSTABLE) ;; *) gh pr merge "$number" --repo "$repo" --auto --rebase --delete-branch >/dev/null 2>&1 && auto=true ;; esac
          "$sm" shift pr --shift "$id" --auto "$auto"
        fi
        # nothing merges it for us: merge on green; the next tick reads it merged
        [ "$verdict $auto" != "green false" ] || gh pr merge "$number" --repo "$repo" --rebase --delete-branch >/dev/null || :
      fi ;;
  esac
done 3<<LANDINGS
$landings
LANDINGS
# landing pass: a handed-off shift's landing is pushed as its branch, then opened as a pull request
while read -r id repo path branch head theme ended sup number url auto red <&3; do
  [ "$number$sup" = "--" ] && [ "$ended" != - ] || continue
  if [ "$(git -C "$path" rev-parse -q --verify "refs/remotes/origin/$branch")" != "$head" ]; then
    # a head refused within six hours waits for a new one: each push runs the repo's pre-push gate (minutes of tests)
    seen="$ROUTINE_STATE_DIR/rejected/$id"
    told=$(cat "$seen" 2>/dev/null) || told=
    [ -z "$(find "$seen" -mmin -360 2>/dev/null)" ] || [ "${told%% *}" != "$head" ] || continue
    # recorded before the push: a gate the watchdog kills mid-push still counts as rejected
    mkdir -p "${seen%/*}" && printf '%s\n' "$head" > "$seen"
    if ! err=$(git -C "$path" push --no-verify origin "$head:refs/heads/$branch" 2>&1 >/dev/null); then   # the pull request's CI is the gate; the trunk checkout's pre-push hook would test main, not the branch
      printf '%s told\n' "$head" > "$seen"
      echo "push of $branch in $path rejected: $err" >&2
      why=$(printf '%s\n' "$err" | awk 'NR == 1 { first = $0 } /^(error|fatal):|^ ?! / { first = $0; exit } END { gsub(/ +/, " ", first); sub(/^ /, "", first); print first }')
      [ "$told" = "$head told" ] || "$sm" trail append --kind correction --shift "$id" --repo "$repo" --branch "$branch" --rationale "push rejected: $why" >/dev/null
      continue
    fi
    rm -f "$seen"
  fi
  body="$root/shifts/$id/report.md"
  [ -f "$body" ] || body="$root/shifts/$id/handoff.md"
  # a pull request already open for the branch (a create whose record failed) is adopted: gh names it in its refusal
  out=$(gh pr create --repo "$repo" --base main --head "$branch" --title "$theme $id · $repo" --body-file "$body" 2>&1) || :
  url=$(printf '%s\n' "$out" | grep -Eo 'https://[^ ]+/pull/[0-9]+' | tail -n 1)
  [ -n "$url" ] || { echo "gh pr create for $id: $out" >&2; continue; }
  "$sm" shift pr --shift "$id" --number "${url##*/}" --url "$url"
done 3<<LANDINGS
$landings
LANDINGS
# reap: every ended shift or breath pass not yet reaped — its session leaves Claude Code's listing, its transcript goes to the archive
sp="$HOME/.local/bin/dispatch"
python3 -I -c '
import glob, json, os, sys
for path in sorted(glob.glob(sys.argv[1] + "/shifts/s-*/meta.json") + glob.glob(sys.argv[1] + "/breath/passes/*/pass.json")):
    try:
        m = json.load(open(path))
    except (OSError, ValueError):
        continue
    if isinstance(m, dict) and m.get("ended") and not m.get("reaped"):
        print(os.path.basename(os.path.dirname(path)), m.get("bg_id") or "-", m.get("session_id") or "-")
' "$root" | while read -r name bg sid; do
  if [ "$bg" != "-" ]; then
    req=$(python3 -I -c 'import json,sys; print(json.dumps({"verb":"claude","action":"list","ids":[sys.argv[1]]}))' "$bg")
    state=$(printf '%s\n' "$req" | "$sp" run 2>/dev/null | python3 -I -c '
import json, sys
try:
    rows = json.loads(sys.stdin.readline())["sessions"]
except (ValueError, KeyError, TypeError):
    sys.exit()
print(rows[0].get("state", "") if rows else "gone")
')
    case "$state" in
      done|failed|stopped) req=$(python3 -I -c 'import json,sys; print(json.dumps({"verb":"claude","action":"rm","id":sys.argv[1]}))' "$bg")
        printf '%s\n' "$req" | "$sp" run >/dev/null 2>&1 ;;
      gone) ;;
      *) continue ;;   # still working, or dispatch could not say: never take a live transcript; next tick
    esac
  fi
  # --force: dispatch's state, not the transcript's mtime, says the session is over; "no transcript" means already taken
  [ "$sid" = "-" ] || "$sm" take "$sid" --force >/dev/null 2>&1 || :
  case $name in
    s-*) "$sm" shift reaped --shift "$name" ;;
    *) "$sm" breathe reaped --id "$name" ;;
  esac
done
# close: when the open shift's session has ended or overrun, record its handoff and release the lock
cur="$root/shifts/current.json"
[ -f "$cur" ] || exit 0
field() {   # one field of current.json; "-" when absent, null or empty
  python3 -I -c '
import json, sys
v = json.load(open(sys.argv[1])).get(sys.argv[2])
print(v if isinstance(v, str) and v else "-")
' "$cur" "$1"
}
id=$(field shift); bg=$(field bg_id); due=$(field handoff_by); started=$(field started); repo=$(field repo)   # read before handoff removes current.json
since() {   # whole minutes since an ISO-8601 instant; -1 for "-"
  python3 -I -c '
import sys, datetime
if sys.argv[1] == "-":
    print(-1); sys.exit()
d = datetime.datetime.fromisoformat(sys.argv[1].replace("Z", "+00:00"))
print(int((datetime.datetime.now(datetime.timezone.utc) - d).total_seconds() // 60))
' "$1"
}
[ "$id" != "-" ] || { echo "current.json names no shift: $(cat "$cur")" >&2; exit 1; }
ask() {   # one dispatch request; prints the answer; returns 1 when dispatch fails, says nothing, or answers an error
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
  ask "$req" >/dev/null || { echo "dispatch could not stop $1; lock kept" >&2; exit 1; }
}
rm_session() {   # forget one stray bg session so it leaves the owner's `claude agents` view; a failed rm is only noise
  req=$(python3 -I -c 'import json,sys; print(json.dumps({"verb":"claude","action":"rm","id":sys.argv[1]}))' "$1")
  ask "$req" >/dev/null || echo "dispatch could not rm $1; left in the listing" >&2
}
stop_named() {   # stop every working or blocked session dispatch lists under the name "shift <id>"; leaves their ids in $strays
  req=$(python3 -I -c 'import json; print(json.dumps({"verb": "claude", "action": "list"}))')
  answer=$(ask "$req") || { echo "dispatch list failed: $answer" >&2; exit 1; }
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
  "$sm" shift handoff --shift "$id" --abandoned --reason "the open block failed between brief and attach; any session named shift $id was stopped" || exit 1
  [ -z "$strays" ] || sleep 3   # a stopped session's SessionEnd hook races the removal of its job dir
  for stray in $strays; do rm_session "$stray"; done
  exit 0
fi
req=$(python3 -I -c 'import json,sys; print(json.dumps({"verb": "claude", "action": "list", "ids": [sys.argv[1]]}))' "$bg")
answer=$(ask "$req") || { echo "dispatch list for $bg answered: $answer" >&2; exit 1; }
# a row without last_activity has no transcript: the session never took its first turn
row=$(printf '%s' "$answer" | python3 -I -c '
import json, sys
rows = json.loads(sys.stdin.readline() or "{}").get("sessions", [])
print(rows[0].get("state", "") if rows else "gone", "talking" if rows and "last_activity" in rows[0] else "silent")
')
state=${row% *}; talk=${row#* }
case "$state" in
  done|failed|gone)
    "$sm" shift handoff --shift "$id" --from-session || "$sm" shift handoff --shift "$id" --abandoned || exit 1
    ;;
  working|blocked)
    late=$(since "$due")
    if [ "$talk" = silent ] && [ "$(since "$started")" -gt 10 ]; then
      # a --bg dontAsk session cannot answer a start-up prompt (a project .mcp.json server); ten minutes, not handoff_by
      stop_session "$bg"
      why="session never started (a project MCP prompt?)"
      "$sm" shift handoff --shift "$id" --abandoned --reason "$why" || exit 1
      "$sm" trail append --kind correction --shift "$id" --repo "$repo" --rationale "$why" >/dev/null
    elif [ "$late" -gt "${SANDMAN_SHIFT_GRACE_MIN:-20}" ]; then
      stop_session "$bg"
      # always abandoned-with-reason: a handoff.md the session wrote is kept beneath the Abandoned line
      "$sm" shift handoff --shift "$id" --abandoned --reason "stopped at $(date -u +%FT%TZ): still $state $late min past handoff_by" || exit 1
    fi
    ;;
  *)
    echo "dispatch list for $bg answered an unknown state: $answer" >&2
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
sp="$HOME/.local/bin/dispatch"
rc=0
"$sm" shift clock --check >/dev/null || rc=$?
[ "$rc" -ne 1 ] || exit 0   # day, dawn near, or the window nearly spent: wait
[ "$rc" -eq 0 ] || exit "$rc"   # 3: the clock file is unreadable — a fault, not a wait
rc=0
err=$(mktemp)
id=$("$sm" shift brief 2>"$err") || rc=$?
cat "$err" >&2
spent=$(grep -c '^sandman: every rotation repo is spent' "$err" || :)
rm -f "$err"
[ "$rc" -ne 1 ] || exit 0   # brief refused: the Sabbath is kept for today, or nothing to do — wait
[ "$spent" -eq 0 ] || exit 0   # every repo has had its rounds this night — wait for the next
[ "$rc" -eq 0 ] || exit "$rc"   # 2 or 3: a fault — template, trust, window file, rotation.toml
cwd=$("$sm" shift pick --path --shift "$id")
profile=$("$sm" shift profile)
req=$(python3 -I -c '
import json, sys
brief, cwd, meta, shift, profile = sys.argv[1:6]
print(json.dumps({
    "verb": "claude",
    "action": "start",
    "prompt": open(brief).read(),
    "cwd": cwd,
    "model": json.load(open(meta))["mind"],
    "permission_mode": "dontAsk",
    "settings": json.loads(profile),
    "name": "shift " + shift,
}))
' "$root/shifts/$id/brief.md" "$cwd" "$root/shifts/$id/meta.json" "$id" "$profile")
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
  echo "dispatch start failed for $id: $answer" >&2
  exit 1
fi
"$sm" shift attach --shift "$id" --bg-id "$bg" --session-id "$sid" || { echo "attach failed for $id ($bg); the close block will stop and abandon it" >&2; exit 1; }
```
