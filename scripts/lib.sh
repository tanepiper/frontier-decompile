# Shared settings for the scripts: where Ghidra, Java and the Python environment are.
# Override any of them in the environment, or in a `.env` file at the repository's root.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
[[ -f "$ROOT/.env" ]] && set -a && . "$ROOT/.env" && set +a

find_ghidra() {
  for d in "${GHIDRA_INSTALL_DIR:-}" /opt/homebrew/opt/ghidra/libexec /usr/local/opt/ghidra/libexec \
           /opt/ghidra /usr/share/ghidra "$HOME"/ghidra_*_PUBLIC; do
    [[ -n "$d" && -x "$d/support/analyzeHeadless" ]] && { echo "$d"; return; }
  done
}

find_java21() {
  for d in "${JAVA_HOME:-}" /opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home \
           /usr/lib/jvm/java-21-openjdk-amd64 /usr/lib/jvm/java-21-openjdk-arm64 /usr/lib/jvm/java-21-openjdk /usr/lib/jvm/temurin-21-jdk-amd64; do
    [[ -n "$d" && -x "$d/bin/java" ]] && { echo "$d"; return; }
  done
}

PYTHON="${PYTHON:-$ROOT/.venv/bin/python}"

# A folder path for a Ghidra project that Ghidra accepts. Ghidra refuses project paths with a
# folder that starts with "." (a worktree in .claude/worktrees/, a checkout under ~/.cache):
# for those, a link without dots in $TMPDIR points at the real folder.
ghidra_project_dir() {
  local dir; dir="$(cd "$1" && pwd -P)" || return 1
  case "$dir/" in
    */.*) local link="${TMPDIR:-/tmp}"; link="${link%/}/re-ghidra-$(printf %s "$dir" | cksum | cut -d' ' -f1)"
          ln -sfn "$dir" "$link" && echo "$link" ;;
    *) echo "$dir" ;;
  esac
}

# DOSBox-X's version as YYYY.MM.DD, or nothing when it is not installed.
dosbox_version() {
  command -v dosbox-x >/dev/null || return 0
  { dosbox-x -version 2>&1 || true; } | sed -n 's/^DOSBox-X version \([0-9][0-9.]*\).*/\1/p' | head -1
}

# True when DOSBox-X has the debugger control channel (the mcp_server setting), which
# dosbox-run.sh --mcp, dbx_control.py and the dosbox MCP server need: 2026.10 or later.
# Distribution packages are often older; build a current one (the re-setup skill).
dosbox_has_debug_channel() {
  local v; v="$(dosbox_version)"
  [[ -n "$v" ]] || return 1
  local y="${v%%.*}" rest="${v#*.}"; local m="${rest%%.*}"
  (( 10#$y > 2026 || (10#$y == 2026 && 10#$m >= 10) ))
}
