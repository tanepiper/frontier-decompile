#!/usr/bin/env bash
# Run one of the tools/ghidra scripts on a binary in an existing, analysed Ghidra project,
# without analysing it again (about a minute of start-up, so batch addresses in one run).
#
#   scripts/ghidra-run.sh BINARY SCRIPT OUTPUT [ARGS ...]
#
#   scripts/ghidra-run.sh game.exe decompile analysis/notes/rng.txt 0x140001000 0x140001080
#   scripts/ghidra-run.sh game.exe named     analysis/notes/random.txt Random Rand
#   scripts/ghidra-run.sh game.exe refs      analysis/notes/refs.txt 0x140009000
#   scripts/ghidra-run.sh game.exe vtable    analysis/notes/vt.txt Tree 0x140008000 16
#   scripts/ghidra-run.sh game.exe labels    analysis/notes/labels.txt labels/names.tsv
#   scripts/ghidra-run.sh game.exe callargs  analysis/notes/sounds.tsv PlaySound rng_next
#   scripts/ghidra-run.sh game.bin blocks    analysis/notes/blocks.txt derived/loadmap.tsv
#
# SCRIPT is decompile (ExportAddressBatch: also creates functions Ghidra missed, such as
# callbacks reached only through tables), named (ExportNamedFunctions), refs (ExportAddressRefs:
# every reference to an address, with the referring function decompiled) or vtable
# (ExportVtableSlots: LABEL 0xTABLE [SLOTS] ...) or labels (ApplyLabels: name functions and
# globals from a TSV, creating missing functions; it saves the project, so export again after
# it with scripts/ghidra-export.sh --labels, or re-run decompile), callargs (ExportCallArgs:
# every call to the named functions with the constant arguments at each call) or blocks
# (AddMemoryBlocks: add raw pieces of memory from a load map; it saves the project, so
# analyse again). Use --project DIR for another project. Ghidra locks a project: run one job
# per project at a time.
set -euo pipefail
. "$(dirname "$0")/lib.sh"
project="$ROOT/analysis/ghidra-project"
[[ "${1:-}" == --project ]] && { project="$2"; shift 2; }
[[ $# -ge 3 ]] || { sed -n '2,27p' "$0"; exit 2; }
binary="$(basename "$1")"; script="$2"; output="$3"; shift 3
case "$script" in
  decompile) java_script=ExportAddressBatch.java ;;
  named) java_script=ExportNamedFunctions.java ;;
  refs) java_script=ExportAddressRefs.java ;;
  vtable) java_script=ExportVtableSlots.java ;;
  labels) java_script=ApplyLabels.java ;;
  callargs) java_script=ExportCallArgs.java ;;
  blocks) java_script=AddMemoryBlocks.java ;;
  *.java) java_script="$script" ;;
  *) echo "unknown script $script" >&2; exit 2 ;;
esac
ghidra="$(find_ghidra)"; java="$(find_java21)"
[[ -n "$ghidra" && -n "$java" ]] || { echo "Ghidra or JDK 21 missing: run scripts/setup.sh" >&2; exit 1; }
mkdir -p "$(dirname "$output")"
output="$(cd "$(dirname "$output")" && pwd)/$(basename "$output")"
JAVA_HOME="$java" "$ghidra/support/analyzeHeadless" "$(ghidra_project_dir "$project")" re -process "$binary" -noanalysis \
  -scriptPath "$ROOT/tools/ghidra" -postScript "$java_script" "$output" "$@" 2>&1 \
  | grep -E "written to|ERROR|Exception" || true
[[ -s "$output" ]] && echo "$output" || { echo "no output: is $binary in $project?" >&2; exit 1; }
