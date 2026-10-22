#!/usr/bin/env bash
# Run a browser build locally. Serves your git-ignored game-data/ folder on 127.0.0.1:8742 and the
# app via trunk's proxy: the game on http://127.0.0.1:8743/?data=/game-data/ (default) or, with
# `--app viewer`, the resource viewer on http://127.0.0.1:8741/?data=/game-data/. Never deploy
# this proxy.
set -euo pipefail
repo="$(cd "$(dirname "$0")/.." && pwd)"
[[ -f "$repo/game-data/original/FRONTIER.EXE" ]] || { echo "Run tools/extract-assets.sh first." >&2; exit 1; }
app=game
if [[ "${1:-}" == "--app" ]]; then
  app="$2"
  shift 2
fi
case "$app" in
  game) port=8743 ;;
  viewer) port=8741 ;;
  *) echo "unknown app: $app (game or viewer)" >&2; exit 1 ;;
esac
python3 -m http.server 8742 --bind 127.0.0.1 --directory "$repo/game-data" >/dev/null 2>&1 &
server=$!
trap 'kill $server 2>/dev/null' EXIT
cd "$repo/apps/$app"
# Separate target dir: avoids lock contention with native cargo builds in the workspace.
export CARGO_TARGET_DIR="$repo/target/web"
echo "$app: http://127.0.0.1:$port/?data=/game-data/"
trunk serve --release "$@"
