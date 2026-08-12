#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

devbox run build

# --inputs-from binds every nixpkgs tool below to this repository's flake.lock.
nix --extra-experimental-features "nix-command flakes" \
  shell --inputs-from "$repo_root" nixpkgs#asciinema nixpkgs#expect -c \
  asciinema rec --quiet --overwrite --cols 120 --rows 36 --idle-time-limit 2 \
  --command 'expect assets/demo.exp' assets/dumbgram-demo.cast

# Keep the final TUI frame in the GIF instead of recording alternate-screen teardown.
nix --extra-experimental-features "nix-command flakes" \
  shell --inputs-from "$repo_root" nixpkgs#python3 -c python3 - <<'PY'
import json
from pathlib import Path

cast_path = Path("assets/dumbgram-demo.cast")
lines = cast_path.read_text().splitlines()
for index in range(1, len(lines)):
    event = json.loads(lines[index])
    if event[1] == "o":
        event[2] = event[2].replace("\x1b[?1049l", "")
    lines[index] = json.dumps(event, separators=(",", ":"))
cast_path.write_text("\n".join(lines) + "\n")
PY

font_store=$(nix --extra-experimental-features "nix-command flakes" \
  eval --inputs-from "$repo_root" --raw nixpkgs#dejavu_fonts.outPath)
nix --extra-experimental-features "nix-command flakes" \
  shell --inputs-from "$repo_root" nixpkgs#asciinema-agg nixpkgs#dejavu_fonts -c \
  agg --quiet --theme dracula --font-family "DejaVu Sans Mono" \
  --font-dir "$font_store/share/fonts/truetype" --font-size 14 \
  --fps-cap 12 --idle-time-limit 2 --last-frame-duration 0.1 \
  assets/dumbgram-demo.cast assets/dumbgram-demo.gif

rm assets/dumbgram-demo.cast
