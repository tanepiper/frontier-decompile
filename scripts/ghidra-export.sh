#!/usr/bin/env bash
# Analyse one or more binaries with Ghidra (no window) and export them as text an agent can
# search: decomp.c, functions.tsv, strings.txt, imports.txt, exports.txt per binary.
#
#   scripts/ghidra-export.sh [--project DIR] [--out DIR] [--labels FILE] BINARY [BINARY ...]
#
# Defaults: the Ghidra project in analysis/ghidra-project, the export in analysis/export/<binary>/.
# --labels FILE applies a labels TSV (address, name) before the export: named functions are
# created where Ghidra missed them (for example from tools/dos/tdinfo.py labels).
# Import the game's executable and its own DLLs together, so calls between them are resolved.
# A large game takes 10-30 minutes; it prints only progress lines and writes the full log to
# analysis/headless.log. Set GHIDRA_CPUS (default 6) and MAXMEM (default 8G) to tune it.
# Ghidra does not search the disk for imported libraries (-loader-libraryLoadDepth 0): pass every
# binary you want. The search can crash the import, for example when a 16-bit NE program imports
# a module (RTM) whose file next to it (RTM.EXE) is a plain DOS program.
set -euo pipefail
. "$(dirname "$0")/lib.sh"
project="$ROOT/analysis/ghidra-project"; out="$ROOT/analysis/export"; labels=()
while [[ "${1:-}" == --* ]]; do
  case "$1" in
    --project) project="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --labels) labels=(-postScript ApplyLabels.java "$ROOT/analysis/labels-report.txt" "$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"); shift 2 ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
done
[[ $# -gt 0 ]] || { sed -n '2,17p' "$0"; exit 2; }
ghidra="$(find_ghidra)"; java="$(find_java21)"
[[ -n "$ghidra" ]] || { echo "Ghidra not found: run scripts/setup.sh, or set GHIDRA_INSTALL_DIR" >&2; exit 1; }
[[ -n "$java" ]] || { echo "JDK 21 not found: run scripts/setup.sh, or set JAVA_HOME" >&2; exit 1; }
bins=(); for b in "$@"; do bins+=("$(cd "$(dirname "$b")" && pwd)/$(basename "$b")"); done
mkdir -p "$project" "$out" "$ROOT/analysis"
log="$(dirname "$project")/headless.log"
project="$(ghidra_project_dir "$project")"
echo "Analysing ${#bins[@]} binaries into $project (log: $log)"
JAVA_HOME="$java" MAXMEM="${MAXMEM:-8G}" "$ghidra/support/analyzeHeadless" "$project" re \
  -overwrite -max-cpu "${GHIDRA_CPUS:-6}" -import "${bins[@]}" -loader-libraryLoadDepth 0 \
  -scriptPath "$ROOT/tools/ghidra" ${labels[@]+"${labels[@]}"} -postScript ExportAll.java "$out" \
  2>&1 | tee "$log" \
  | grep -E --line-buffered "EXPORT DONE|LABELS DONE|Analysis succeeded|ERROR.*Exception" || true
for b in "${bins[@]}"; do
  f="$out/$(basename "$b")/decomp.c"
  [[ -s "$f" ]] && echo "exported $(basename "$b"): $(grep -c '^// ==== ' "$f") functions in $f" \
    || { echo "no export for $(basename "$b"): see the log" >&2; exit 1; }
done
