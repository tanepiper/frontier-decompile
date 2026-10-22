#!/usr/bin/env bash
# Check, and optionally install, everything GameFrame uses.
#
#   scripts/setup.sh            check only: a table of what is present and what is missing
#   scripts/setup.sh --install  install what is missing (Homebrew on macOS, apt on Debian/Ubuntu)
#                               and create .venv with the Python tools
#   scripts/setup.sh --rea      also show REA's own registration plan for your agent (dry run)
#
# What gets installed, and roughly how big it is:
#   Ghidra (decompiler, ~500 MB) and JDK 21 (~300 MB)   static analysis
#   Python 3 + .venv: unicorn, pefile, capstone (~40 MB) emulation, PE reading, disassembly
#   objdump (binutils or LLVM)                           reading instructions exactly
#   Node.js (for npx)                                    REA, the reverse-engineering MCP server
#   Zig (~200 MB, optional)                              builds the sample games from any host
#   DOSBox-X (~50 MB, optional)                          runs DOS games for ground truth (DOS only)
#   innoextract, 7-Zip (~5 MB, optional)                 extract installers (scripts/extract-installer.sh)
# Nothing is installed without --install. On Linux, Ghidra is not in apt: the script prints the
# download page instead of fetching a large archive itself.
set -uo pipefail
. "$(dirname "$0")/lib.sh"
install=0; rea=0
for a in "$@"; do
  case "$a" in
    --install) install=1 ;;
    --rea) rea=1 ;;
    -h|--help) sed -n '2,19p' "$0"; exit 0 ;;
    *) echo "unknown option $a" >&2; exit 2 ;;
  esac
done
os="$(uname -s)"
missing=0
row() { printf '  %-14s %-8s %s\n' "$1" "$2" "$3"; }
have() { command -v "$1" >/dev/null 2>&1; }

pkg_install() { # brew-name apt-name
  if [[ $install == 0 ]]; then return 1; fi
  if [[ $os == Darwin ]] && have brew; then brew install "$1" >/dev/null && return 0
  elif have apt-get && [[ -n "${2:-}" ]]; then sudo apt-get install -y "$2" >/dev/null && return 0
  fi
  return 1
}

check() { # label test-command brew apt hint
  local label="$1" test="$2" brew="$3" apt="$4" hint="$5"
  if eval "$test" >/dev/null 2>&1; then row "$label" ok "$(eval "$test" 2>/dev/null | head -1)"; return; fi
  if pkg_install "$brew" "$apt" && eval "$test" >/dev/null 2>&1; then row "$label" installed ""; return; fi
  row "$label" MISSING "$hint"; missing=$((missing + 1))
}

echo "Checking the GameFrame toolkit ($os):"
check python3 "python3 --version" python@3.12 python3-venv "Python 3.10 or newer"
check java21 "find_java21" openjdk@21 openjdk-21-jdk "JDK 21 (Ghidra 11/12 needs it); set JAVA_HOME"
if [[ -z "$(find_ghidra)" && $install == 1 && $os == Darwin ]] && have brew; then brew install ghidra >/dev/null; fi
if [[ -n "$(find_ghidra)" ]]; then row ghidra ok "$(find_ghidra)"
else row ghidra MISSING "https://github.com/NationalSecurityAgency/ghidra/releases (unzip, set GHIDRA_INSTALL_DIR)"; missing=$((missing + 1)); fi
check objdump "objdump --version" binutils binutils "binutils or LLVM objdump"
check node "node --version" node nodejs "Node.js 18+ (REA runs with npx)"
check zig "zig version" zig "" "optional: builds examples/sample-game (https://ziglang.org/download)"
check innoextract "innoextract --version | head -1" innoextract innoextract "optional: extracts Inno Setup installers (GOG): scripts/extract-installer.sh"
check 7z "command -v 7z || command -v 7zz" sevenzip p7zip-full "optional: extracts NSIS, MSI, cab, 7z and rar: scripts/extract-installer.sh"
check dosbox-x "command -v dosbox-x >/dev/null && { dosbox-x -version 2>&1 || true; } | sed -n 's/^\\(DOSBox-X version [^ ,]*\\).*/\\1/p'" dosbox-x dosbox-x "optional, for DOS games: brew install --formula dosbox-x, or https://dosbox-x.com"
if command -v dosbox-x >/dev/null && ! dosbox_has_debug_channel; then
  row "dosbox-x debugger" OLD "$(dosbox_version) has no debugger control channel (needs 2026.10+): build a current one, see the re-setup skill"
fi

if [[ ! -x "$ROOT/.venv/bin/python" && $install == 1 ]]; then
  python3 -m venv "$ROOT/.venv" && "$ROOT/.venv/bin/pip" install -q -r "$ROOT/requirements.txt"
fi
if "$PYTHON" -c "import unicorn, pefile, capstone" 2>/dev/null; then
  row .venv ok "unicorn $("$PYTHON" -c 'import unicorn; print(unicorn.__version__)'), pefile, capstone"
else
  row .venv MISSING "run scripts/setup.sh --install (python3 -m venv .venv; pip install -r requirements.txt)"
  missing=$((missing + 1))
fi

# The environment file the scripts and .mcp.json read.
if [[ -n "$(find_ghidra)" && -n "$(find_java21)" && ! -f "$ROOT/.env" ]]; then
  printf 'GHIDRA_INSTALL_DIR=%s\nJAVA_HOME=%s\n' "$(find_ghidra)" "$(find_java21)" > "$ROOT/.env"
  echo "  wrote .env (GHIDRA_INSTALL_DIR, JAVA_HOME)"
fi

if [[ $rea == 1 ]]; then
  echo
  echo "REA registration plan (dry run; nothing is changed):"
  npx -y rea-agents@6.4.0 setup --client "${REA_CLIENT:-claude_code}" --dry-run || \
    echo "  could not run REA's setup; see docs/SETUP.md"
fi

echo
if [[ $missing == 0 ]]; then echo "Everything is in place."
else echo "$missing missing. Run scripts/setup.sh --install, or see docs/SETUP.md."; exit 1; fi
