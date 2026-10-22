# Source this file to use GameFrame from a shell or an agent session:
#
#   source scripts/env.sh        (bash or zsh, from any folder)
#
# It sets GAMEFRAME_ROOT, loads the ignored .env (GHIDRA_INSTALL_DIR, JAVA_HOME and other
# paths that scripts/setup.sh wrote), exports the Ghidra and Java folders it finds, puts
# GameFrame's `gameframe` command and the .venv's Python first on PATH, and prints one line
# saying what it found. It changes nothing on disk. The scripts load .env on their own; this
# file is for commands typed by hand and for agents that start a fresh shell for each command.
if [ -n "${BASH_SOURCE:-}" ]; then _gf_self="${BASH_SOURCE[0]}"; else _gf_self="${(%):-%x}"; fi
GAMEFRAME_ROOT="$(cd "$(dirname "$_gf_self")/.." && pwd)"
export GAMEFRAME_ROOT
if [ -f "$GAMEFRAME_ROOT/.env" ]; then set -a; . "$GAMEFRAME_ROOT/.env"; set +a; fi
_gf_found() { for d in "$@"; do [ -n "$d" ] && [ -x "$d/$_gf_bin" ] && { echo "$d"; return; }; done; }
_gf_bin=support/analyzeHeadless
GHIDRA_INSTALL_DIR="$(_gf_found "${GHIDRA_INSTALL_DIR:-}" /opt/homebrew/opt/ghidra/libexec \
  /usr/local/opt/ghidra/libexec /opt/ghidra /usr/share/ghidra \
  $(find "$HOME" -maxdepth 1 -name 'ghidra_*_PUBLIC' 2>/dev/null))"
_gf_bin=bin/java
JAVA_HOME="$(_gf_found "${JAVA_HOME:-}" /opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home \
  /usr/lib/jvm/java-21-openjdk-amd64 /usr/lib/jvm/java-21-openjdk-arm64 /usr/lib/jvm/java-21-openjdk)"
export GHIDRA_INSTALL_DIR JAVA_HOME
case ":$PATH:" in *":$GAMEFRAME_ROOT:"*) ;; *) PATH="$GAMEFRAME_ROOT:$PATH" ;; esac
if [ -d "$GAMEFRAME_ROOT/.venv/bin" ]; then
  case ":$PATH:" in *":$GAMEFRAME_ROOT/.venv/bin:"*) ;; *) PATH="$GAMEFRAME_ROOT/.venv/bin:$PATH" ;; esac
fi
export PATH
echo "GameFrame at $GAMEFRAME_ROOT; Ghidra: ${GHIDRA_INSTALL_DIR:-not found}; Java 21: ${JAVA_HOME:-not found}; .venv: $([ -d "$GAMEFRAME_ROOT/.venv" ] && echo yes || echo no)"
unset _gf_self _gf_bin
unset -f _gf_found
