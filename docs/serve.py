#!/usr/bin/env python3
"""Serve docs/ over HTTP, so the plan page is visited rather than file-opened.

Adapted from jgeschwendt/ui serve.py (@ 4f1d9db) — same three corrections a dev
server owes a wrapper: HTTP/1.1 keep-alive, a threaded handler, and no
Content-Security-Policy, so a proxy's injected script is never silently blocked.

    docs/serve.py              # http://127.0.0.1:7875/
    docs/serve.py --port 4321
    PORT=4321 docs/serve.py    # $PORT wins over the default, for a wrapping proxy
"""

from __future__ import annotations

import argparse
import os
import sys
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parent
DEFAULT_PORT = 7875


class Handler(SimpleHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    extensions_map = {
        **SimpleHTTPRequestHandler.extensions_map,
        ".css": "text/css; charset=utf-8",
        ".html": "text/html; charset=utf-8",
        ".js": "text/javascript; charset=utf-8",
        ".md": "text/markdown; charset=utf-8",
        ".svg": "image/svg+xml",
    }

    def end_headers(self) -> None:
        # An edit to the page or the stylesheet has to show on reload.
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def log_message(self, fmt: str, *args) -> None:
        sys.stderr.write("%s %s\n" % (self.log_date_time_string(), fmt % args))


class Server(ThreadingHTTPServer):
    allow_reuse_address = True
    daemon_threads = True


def main() -> int:
    ap = argparse.ArgumentParser(description="serve sandman's docs/ over HTTP")
    ap.add_argument("--host", default="127.0.0.1", help="bind address (default: 127.0.0.1 — loopback only)")
    ap.add_argument("--port", type=int, default=None, help=f"bind port (default: $PORT, else {DEFAULT_PORT})")
    args = ap.parse_args()

    port = args.port if args.port is not None else int(os.environ.get("PORT") or DEFAULT_PORT)

    try:
        httpd = Server((args.host, port), partial(Handler, directory=str(ROOT)))
    except OSError as err:
        print(f"serve: cannot bind {args.host}:{port} — {err}", file=sys.stderr)
        return 1

    bound = httpd.socket.getsockname()[1]
    print(f"plan → http://{args.host}:{bound}/", file=sys.stderr)
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        httpd.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
