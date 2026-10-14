"""Rewrite a board's light token hexes to their dark values, in place: darken.py <tokens.json> <board.dc.html>.

Builds light→dark over color.tokens (6-digit hex only; first token listed wins a shared light value,
so #ffffff is ui-bg), matches whole-token and case-insensitive as #rrggbb and %23rrggbb, and prints
every 6-digit hex left in the board that is not a light token value — nothing stays light silently.
"""
import json
import re
import sys

HEX = re.compile(r"^#[0-9a-fA-F]{6}$")
tokens, board = sys.argv[1], sys.argv[2]
dark = {}
for t in json.load(open(tokens))["color"]["tokens"]:
    lo, hi = t["value"]["light"], t["value"]["dark"]
    if HEX.match(lo) and HEX.match(hi):
        dark.setdefault(lo[1:].lower(), hi[1:].lower())

src = open(board).read()
pat = re.compile(r"(#|%23)([0-9a-fA-F]{6})(?![0-9a-zA-Z_-])")
unmapped = sorted({m.group(2).lower() for m in pat.finditer(src)} - dark.keys())
open(board, "w").write(pat.sub(lambda m: m.group(1) + dark.get(m.group(2).lower(), m.group(2)), src))
for h in unmapped:
    print(f"unmapped #{h}")
