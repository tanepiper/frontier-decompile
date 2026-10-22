#!/usr/bin/env bash
# Record every coverage run in tools/coverage/scenarios.tsv on the runtime (docs/COVERAGE.md).
#
#   tools/coverage/run.sh [NAME...]     # all runs, or only these
#
# DATA (default game-data/original) and OUT (default game-data/derived/coverage) can be set for a
# worktree that has no game-data of its own. Runs go in parallel, JOBS at a time (default 8).
set -euo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
data="${DATA:-$repo/game-data/original}"
out="${OUT:-$repo/game-data/derived/coverage}"
jobs="${JOBS:-8}"

# Shared event sequences (times in virtual seconds, keys as set-1 scancodes).
P="20:39 24:02"                                    # space at the intro, then 1: start at Ross 154
SEL="34:48+ 34.08:48- 34.4:48+ 34.48:48- 34.8:4d+ 34.88:4d-"  # up, up, right: Barnard's Star
LAUNCH="33:3e 34:@129,18 35.5:41"                  # F4, Launch Request, F7
CLIMB="36.9:b2+ $(for i in $(seq 0 19); do printf '%s:~0,20 ' "$(echo "37 + $i * 0.05" | bc)"; done)38.1:b2- 38.5:1c+ 44:1c-"
TARGET="46:3c 47:48+ 47.08:48- 47.4:48+ 47.48:48- 47.8:4d+ 47.88:4d- 50:3b"

cd "$repo"
cargo build --release -p frontier-dos --example coverage
bin="$repo/target/release/examples/coverage"

grep -v '^#' tools/coverage/scenarios.tsv | while IFS=$'\t' read -r name _base secs events _what; do
  [[ -z "$name" ]] && continue
  [[ "$events" == "-" ]] && events=""
  if [[ $# -gt 0 && ! " $* " == *" $name "* ]]; then continue; fi
  events="${events//\$P/$P}"; events="${events//\$SEL/$SEL}"; events="${events//\$LAUNCH/$LAUNCH}"
  events="${events//\$CLIMB/$CLIMB}"; events="${events//\$TARGET/$TARGET}"
  echo "--data $data --out $out --name $name --secs $secs${events:+ $events}"  # no trailing blank: xargs -L would join lines
done | xargs -P "$jobs" -L 1 "$bin" 2>/dev/null | grep '^run '
