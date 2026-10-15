#!/usr/bin/env bash
# Run Ghidra headless analysis over FRONTIER.EXE and the EL2*.OVL overlays and export decompiled
# C, function lists and strings into analysis/export/<binary>/ (git-ignored).
# Requires tools/extract-assets.sh to have been run first.
#
#   GHIDRA_INSTALL_DIR=/opt/homebrew/opt/ghidra/libexec tools/ghidra/export.sh
set -euo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
ghidra="${GHIDRA_INSTALL_DIR:-/opt/homebrew/opt/ghidra/libexec}"
bins="$repo/game-data/original"
out="$repo/analysis"

if [[ ! -x "$ghidra/support/analyzeHeadless" ]]; then
  echo "Ghidra not found at $ghidra; set GHIDRA_INSTALL_DIR." >&2; exit 1
fi
if [[ ! -f "$bins/FRONTIER.EXE" ]]; then
  echo "Original binaries missing; run tools/extract-assets.sh first." >&2; exit 1
fi
export JAVA_HOME="${JAVA_HOME:-/opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home}"

# Every overlay is its own MZ image, imported as a separate program in one project.
# SeedFunctions.java adds the functions reached only through vector tables and overlay export
# blocks (docs/OVERLAYS.md) before ExportAll.java writes the export.
mkdir -p "$out/ghidra-project" "$out/export"
MAXMEM="${MAXMEM:-4G}" "$ghidra/support/analyzeHeadless" "$out/ghidra-project" frontier \
  -overwrite -max-cpu "${GHIDRA_CPUS:-6}" \
  -import "$bins/FRONTIER.EXE" "$bins"/EL2*.OVL \
  -scriptPath "$repo/tools/ghidra" -postScript SeedFunctions.java -postScript ExportAll.java "$out/export" \
  2>&1 | tee "$out/headless.log" | grep -E --line-buffered "SEED DONE|call overrides|EXPORT DONE|ERROR.*Exception"
