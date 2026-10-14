#!/bin/bash
# The `perpetual` cloud environment's setup script: runs as root on Ubuntu
# before each session, its result cached about seven days. Installs mise, then
# sandman, docket and stele into ~/.local/bin — the tools prompts/cloud-shift.md
# names. Budget: under five minutes (two zero-dependency cargo builds, one
# prebuilt download).
set -euo pipefail
bin="$HOME/.local/bin"
mkdir -p "$bin"
export PATH="$bin:$PATH"

# mise, by its official installer, which drops it in ~/.local/bin; every repo's
# mise.toml is trusted and every prompt answered, since nobody is there to
# commits the session makes carry the owner's identity (the proxy authenticates the push)
git config --global user.name jgeschwendt
git config --global user.email joshua@geschwendt.com
command -v mise >/dev/null || curl -fsSL https://mise.run | sh
mise settings add trusted_config_paths /
mise settings set yes true
# the image's cargo when it has one, else mise's rust
command -v cargo >/dev/null || mise use -g rust@1.94.0

# build <repo> <binary>: a shallow clone of jgeschwendt/<repo>, built release
build() {
  local src
  src=$(mktemp -d)
  git clone --quiet --depth 1 "https://github.com/jgeschwendt/$1" "$src" &&
    mise x -C "$src" -- cargo build --quiet --release &&
    install "$src/target/release/$2" "$bin/"
  local status=$?
  rm -rf "$src"
  return $status
}
build sandman sandman
# docket is private: an anonymous clone fails unless the proxy authenticates
# setup too — then the session builds it from its attached clone
build docket docket || echo "setup: docket not built; the session builds it from ./docket" >&2
# stele ships a prebuilt binary per release; CI pins the same version
curl -fsSL https://raw.githubusercontent.com/jgeschwendt/stele/main/scripts/install.sh | bash -s v0.3.0

sandman version && stele --version
