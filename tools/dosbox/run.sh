#!/usr/bin/env bash
# Run the original game in DOSBox Staging (brew install dosbox-staging).
# Uses game-data/run, a writable copy, so game-data/original stays pristine.
#
#   tools/dosbox/run.sh [extra dosbox-staging args]
set -euo pipefail
repo="$(cd "$(dirname "$0")/../.." && pwd)"
run="$repo/game-data/run"
[[ -f "$run/FRONTIER.EXE" ]] || "$repo/tools/dosbox/reset.sh"
exec dosbox-staging -conf "$repo/tools/dosbox/frontier.conf" -c "mount c \"$run\"" -c "c:" -c "frontier" "$@"
