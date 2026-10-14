#!/bin/sh
# Render every board of the Claude Design canvas "sandman figures" (docs/figures/project/,
# read back from https://claude.ai/artifact/MKRUSNkEimZQHvWmZ4VASL) to its PNG at 2×, the same
# pixels the canvas exports — agent-browser, headless. Run from anywhere: mise run figures.
# Each board renders twice: <name>.png, then <name>-dark.png from a temp copy whose light token
# hexes darken.py rewrites to their dark values (project/ds/gigantic/tokens.json, read back from
# the canvas) — derived at export, never drawn twice.
set -e
F=$(cd "$(dirname "$0")" && pwd)
P=$F/project
# its own browser session, so a parallel agent-browser user never shares the window
export AGENT_BROWSER_SESSION="${AGENT_BROWSER_SESSION:-sandman-figures}"
D=$(mktemp -d)
trap 'rm -rf "$D"' EXIT
cp "$P/support.js" "$D/"
render() { # <dir> <board .dc.html> <out.png> <h>
  agent-browser set viewport 1600 "$4" 2 >/dev/null
  agent-browser open "file://$1/$2" >/dev/null
  agent-browser wait 1500 >/dev/null
  agent-browser screenshot "$F/$3" >/dev/null
  printf '%s %s\n' "$3" "$(file "$F/$3" | sed 's/.*PNG image data, //; s/,.*//')"
}
shoot() { # <board .dc.html> <out.png>
  h=$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["boards"][sys.argv[2]]["h"])' "$P/canvas.json" "$1")
  render "$P" "$1" "$2" "$h"
  cp "$P/$1" "$D/$1"
  python3 -I "$F/darken.py" "$P/ds/gigantic/tokens.json" "$D/$1" | sed "s|^|$1: |"
  render "$D" "$1" "${2%.png}-dark.png" "$h"
}
shoot Verbs.dc.html verbs.png
shoot DataRoot.dc.html data-root.png
shoot Cadence.dc.html cadence.png
shoot Flow.dc.html flow.png
shoot Ending.dc.html ending.png
shoot Voyage.dc.html voyage.png
shoot PerpetualShift.dc.html perpetual-shift.png
agent-browser close >/dev/null
