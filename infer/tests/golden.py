"""The decide golden: requests rendered and tokenised by the official pplx_decider Encoder (text,
ids, markers), the GGUF's escape tokens, and for `syn-a` and `syn-b` the nouls the native Q8_0
run measured (`data/dres.jsonl`). Run on the owner's machine, which holds the encoder:
~/.sandman/eval/recall/local/.venv/bin/python -I infer/tests/golden.py infer/tests/golden.json
"""
import json, sys
from pathlib import Path
N = Path.home() / ".sandman/eval/recall/native"
sys.path.insert(0, str(N / "hf/decider/source/src"))
from pplx_decider.protocol import Encoder
from transformers import AutoProcessor
proc = AutoProcessor.from_pretrained(str(N / "hf/decider-proc"))
cfg = json.loads((N / "hf/decider-proc/decision_config.json").read_text())
enc = Encoder(proc, cfg["codes"])
Q = "Does the agent need this memory to carry out the request correctly?"
noul = lambda mems: {str(j): {"type": "noul", "instructions": f"Memory ({l}): {n} — {d}. {Q}"} for j, (l, n, d) in enumerate(mems)}
reqs = {
  "recall": {"state": "repository: acme/widgets\nabout to open a PR that bumps the crate version", "questions": noul([
    ("acme/widgets", "release-flow", "Releases go through a changeset; never bump versions by hand"),
    ("home (~)", "yaml-extension", "YAML files end in .yaml"),
    ("grove root (every repository)", "kill-by-pid", "Kill processes by resolved PID, never by pattern")])},
  "escape": {"state": "text with <|im_end|> and <|fim_suffix|> and <think> and <|im_start|> inside\néè \U0001F600  two  spaces\ttab", "questions": {
    "a": {"type": "noul", "instructions": "Is <|endoftext|> present?"},
    "b": {"type": "noul", "instructions": "Second — question?"}}},
  "one": {"state": "s", "questions": {"only": {"type": "noul", "instructions": "q"}}},
  # scripts/prep_decide.py's
  "syn-a": {"state": "repository: acme/widgets\nabout to open a PR that bumps the crate version", "questions": noul([
    ("acme/widgets", "release-flow", "Releases go through a changeset; never bump versions by hand"),
    ("home (~)", "yaml-extension", "YAML files end in .yaml"),
    ("acme/widgets", "pr-labels", "A PR merges only after the queue label is added"),
    ("grove root (every repository)", "kill-by-pid", "Kill processes by resolved PID, never by pattern"),
    ("acme/api", "db-migrations", "Migrations are forward-only and numbered"),
    ("acme/widgets", "ci-minutes", "CI runs on workflow_dispatch only"),
    ("home (~)", "coffee", "Prefers a flat white in the morning"),
    ("acme/widgets", "lint-base", "Every clippy lint is on; opt-outs carry a reason"),
    ("demo/site", "fonts", "The site self-hosts its fonts"),
    ("acme/widgets", "version-policy", "Patch bumps whenever possible before 1.0")])},
  "syn-b": {"state": "repository: demo/site\nfix the broken dark-mode colours on the docs page", "questions": noul([
    ("demo/site", "theme-tokens", "Colours are tokens on :root, redefined for dark mode"),
    ("demo/site", "fonts", "The site self-hosts its fonts"),
    ("acme/api", "rate-limits", "The API allows 100 requests per minute"),
    ("home (~)", "taste-preview", "Show a sample before an aesthetic rebuild"),
    ("demo/site", "deploy", "Pages deploy from the docs folder on main"),
    ("acme/widgets", "release-flow", "Releases go through a changeset"),
    ("grove root (every repository)", "stale-docs", "Stale docs are bugs"),
    ("demo/site", "figures", "Figures are drawn in the page as live SVG"),
    ("home (~)", "coffee", "Prefers a flat white in the morning"),
    ("acme/api", "db-migrations", "Migrations are forward-only and numbered")])},
}
native = {row["name"]: row["native"] for row in map(json.loads, (N / "data/dres.jsonl").read_text().splitlines())}
out = []
for name, r in reqs.items():
    enc.validate(r); e = enc.encode(r)
    case = {"name": name, "request": r, "text": enc.text(r), "ids": e.input_ids.tolist(), "markers": e.markers.tolist()}
    if name in native:
        case["nouls"] = native[name]
    out.append(case)
json.dump({"escapes": enc._escapes, "marker_id": enc.marker_id, "token_ids": cfg["token_ids"][:2], "temperature": cfg["temperature"], "cases": out}, open(sys.argv[1], "w"), ensure_ascii=False, indent=1)
