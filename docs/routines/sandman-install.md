---
schedule: */15 * * * *
timeout: 900
on_error: fail
---

# Sandman · install

The installed `sandman` is always trunk's. Every fifteen minutes: if the trunk checkout at `~/.grove/code/jgeschwendt/sandman/main` is clean and its commit differs from the one `~/.local/bin/sandman version` was built from, build it under the repo's pinned toolchain (`mise x -C <trunk> -- cargo build --release`), install the binary, the `shift-test` wrapper and its Seatbelt profile, refresh every routine under `docs/routines/` whose installed copy in `~/.routines` carries no local edit, and restart the report agent (`dev.gigantic.sandman.report`) so it serves from the new build. The trunk moves on its own: `grove-sync` fast-forwards it every five minutes after the shift runner merges a pull request, so a landing reaches the installed binary within twenty minutes of merging. (since 2026-10-22 · the owner: "make sure we are always running latest sandman"; the installed binary had sat two landings behind trunk — `score` took 90 ms against 28)

A dirty trunk is skipped, not built: the owner edits on main, and a binary stamped `-dirty` would carry his half-written change into every hook and shift. The block says so and exits 0; the trunk's own `git status` is the next step, never a stash or checkout from here. A routine with local edits in `~/.routines` is likewise left alone and named — the owner reconciles it with the canonical copy by hand (`docs/bank-format.html#shifts-sandbox`). The block runs under launchd's bare `PATH`, so every tool is an absolute path; `install` replaces by unlink and create, so a shift mid-run keeps the binary it started on. `on_error: fail` — a red build is trunk's bug, the next shift's job, and a catch could only hide it; `routine-health` reports `failed: sandman-install`.

```sh
trunk="$HOME/.grove/code/jgeschwendt/sandman/main"
sm="$HOME/.local/bin/sandman"
want=$(git -C "$trunk" rev-parse --short HEAD)
have=$("$sm" version 2>/dev/null | sed 's/.*-//')
if [ -n "$(git -C "$trunk" status --porcelain)" ]; then echo "dirty: trunk at $want, installed $have — skipped"; exit 0; fi
if [ "$have" = "$want" ]; then echo "current: $want"; exit 0; fi
"$HOME/.local/bin/mise" x -C "$trunk" -- cargo build --release --quiet
install -m755 "$trunk/target/release/sandman" "$sm"
install -m755 "$trunk/profiles/shift-test" "$HOME/.local/bin/shift-test"
install -d "$HOME/.local/share/sandman"
install -m644 "$trunk/profiles/shift-test" "$trunk/profiles/shift-test.sb" "$HOME/.local/share/sandman/"
for doc in "$trunk"/docs/routines/*.md; do
  name=$(basename "$doc")
  if [ -n "$(git -C "$HOME/.routines" status --porcelain -- "$name")" ]; then echo "edited: ~/.routines/$name — left alone"; continue; fi
  cmp -s "$doc" "$HOME/.routines/$name" || install -m644 "$doc" "$HOME/.routines/$name"
done
launchctl kickstart -k "gui/$(id -u)/dev.gigantic.sandman.report"
echo "installed: $have → $("$sm" version)"
```
