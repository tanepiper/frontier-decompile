#!/usr/bin/env bash
# Run the browser build locally. Serves your git-ignored game-data/ folder on 127.0.0.1:8742 and the
# game via trunk's proxy on http://127.0.0.1:8743/?data=/game-data/. Never deploy this proxy.
set -euo pipefail
repo="$(cd "$(dirname "$0")/.." && pwd)"
[[ -f "$repo/game-data/original/FRONTIER.EXE" ]] || { echo "Run tools/extract-assets.sh first." >&2; exit 1; }
python3 -m http.server 8742 --bind 127.0.0.1 --directory "$repo/game-data" >/dev/null 2>&1 &
server=$!
trap 'kill $server 2>/dev/null' EXIT
cd "$repo/apps/game"
# Separate target dir: avoids lock contention with native cargo builds in the workspace.
export CARGO_TARGET_DIR="$repo/target/web"
echo "frontier: http://127.0.0.1:8743/?data=/game-data/"
trunk serve --release "$@"
