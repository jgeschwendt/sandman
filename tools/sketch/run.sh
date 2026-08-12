#!/usr/bin/env bash
# sketch — pair-drawing Excalidraw over design/*.excalidraw. http://127.0.0.1:7873
# Serves the pre-built dist/; no npm install needed.
set -euo pipefail
HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

if [[ ! -f "$HERE/dist/index.html" ]]; then
  echo "dist/ is missing — build it once: (cd $HERE && mise exec -- npm install && mise exec -- npm run build)" >&2
  exit 1
fi

cd "$HERE"
exec mise exec -- node server.js "$@"
