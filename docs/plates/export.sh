#!/bin/sh
# Render every Claude Design artboard source under docs/plates/*/project/ to its PNG at 2×,
# the same pixels the canvas exports — agent-browser, headless. Run from anywhere: mise run plates.
set -e
P=$(cd "$(dirname "$0")" && pwd)
shoot() { # <source .dc.html> <height> <out.png>
  agent-browser set viewport 1600 "$2" 2 >/dev/null
  agent-browser open "file://$1" >/dev/null
  agent-browser wait 1500 >/dev/null
  agent-browser screenshot "$3" >/dev/null
  printf '%s %s\n' "$(basename "$3")" "$(file "$3" | sed 's/.*PNG image data, //; s/,.*//')"
}
h() { python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["boards"][sys.argv[2]]["h"])' "$1" "$2"; }
S=$P/sandman/project
shoot "$S/Main.dc.html"     "$(h "$S/canvas.json" Main.dc.html)"     "$P/verbs.png"
shoot "$S/DataRoot.dc.html" "$(h "$S/canvas.json" DataRoot.dc.html)" "$P/data-root.png"
shoot "$S/Cadence.dc.html"  "$(h "$S/canvas.json" Cadence.dc.html)"  "$P/cadence.png"
shoot "$S/Flow.dc.html"     "$(h "$S/canvas.json" Flow.dc.html)"     "$P/flow.png"
shoot "$S/Ending.dc.html"   "$(h "$S/canvas.json" Ending.dc.html)"   "$P/ending.png"
shoot "$S/Voyage.dc.html"   "$(h "$S/canvas.json" Voyage.dc.html)"   "$P/voyage.png"
S=$P/perpetual-shift/project
shoot "$S/Main.dc.html" "$(h "$S/canvas.json" Main.dc.html)" "$P/perpetual-shift.png"
agent-browser close >/dev/null
