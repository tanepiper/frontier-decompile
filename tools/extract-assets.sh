#!/usr/bin/env bash
# Copy your own copy of Frontier: Elite II (DOS) into game-data/ (git-ignored).
#
#   FRONTIER_DIR=/path/to/frontier tools/extract-assets.sh
#
# FRONTIER_DIR is the folder containing FRONTIER.EXE and the EL2*.OVL overlays.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
game="${FRONTIER_DIR:-}"
if [[ -z "$game" || ! -f "$game/FRONTIER.EXE" ]]; then
  echo "Set FRONTIER_DIR to the folder containing FRONTIER.EXE." >&2
  exit 1
fi

out="$repo/game-data"
echo "Game:   $game"
echo "Output: $out"
mkdir -p "$out/original"
# Pristine inputs: analysis and tests read these; nothing ever writes here.
cp -Rp "$game"/. "$out/original/"
chmod -R a-w "$out/original"

(cd "$out/original" && find . -type f ! -name .DS_Store | LC_ALL=C sort | xargs shasum -a 256) \
  > "$out/MANIFEST.sha256"
echo "Wrote $out/MANIFEST.sha256"

"$repo/tools/dosbox/reset.sh"
