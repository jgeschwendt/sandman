#!/bin/sh
# Render every board of the Claude Design canvas "sandman figures" (docs/figures/project/,
# read back from https://claude.ai/artifact/MKRUSNkEimZQHvWmZ4VASL) to its PNG at 2×, the same
# pixels the canvas exports — agent-browser, headless. Run from anywhere: mise run figures.
set -e
F=$(cd "$(dirname "$0")" && pwd)
P=$F/project
# its own browser session, so a parallel agent-browser user never shares the window
export AGENT_BROWSER_SESSION="${AGENT_BROWSER_SESSION:-sandman-figures}"
shoot() { # <board .dc.html> <out.png>
  h=$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["boards"][sys.argv[2]]["h"])' "$P/canvas.json" "$1")
  agent-browser set viewport 1600 "$h" 2 >/dev/null
  agent-browser open "file://$P/$1" >/dev/null
  agent-browser wait 1500 >/dev/null
  agent-browser screenshot "$F/$2" >/dev/null
  printf '%s %s\n' "$2" "$(file "$F/$2" | sed 's/.*PNG image data, //; s/,.*//')"
}
shoot Verbs.dc.html verbs.png
shoot DataRoot.dc.html data-root.png
shoot Cadence.dc.html cadence.png
shoot Flow.dc.html flow.png
shoot Ending.dc.html ending.png
shoot Voyage.dc.html voyage.png
shoot PerpetualShift.dc.html perpetual-shift.png
agent-browser close >/dev/null
