#!/usr/bin/env bash
# Recreate game-data/run (the DOSBox working copy) from game-data/original.
set -euo pipefail
repo="$(cd "$(dirname "$0")/../.." && pwd)"
rm -rf "$repo/game-data/run"
cp -R "$repo/game-data/original" "$repo/game-data/run"
chmod -R u+w "$repo/game-data/run"
echo "Reset $repo/game-data/run"
