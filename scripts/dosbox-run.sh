#!/usr/bin/env bash
# Run a DOS program under DOSBox-X from the command line: the whole original game, for ground
# truth that a single emulated function cannot give (frames, demos, file access, timing).
#
#   scripts/dosbox-run.sh [options] GAME_DIR PROGRAM [ARGS ...]
#
#   --out DIR      logs, the exit code and the copied drive go here (default analysis/dosbox)
#   --time N       stop DOSBox-X after N seconds (default 60)
#   --window       show the window (default: no window, no sound: SDL dummy drivers)
#   --fileio       log every file the program opens, reads, seeks and writes (INT 21h);
#                  summarise the log with tools/dos/fileio_trace.py
#   --int21        log every INT 21h call
#   --debug        start in DOSBox-X's debugger (implies --window; see docs/DOS.md)
#   --break        load PROGRAM with DEBUGBOX: it stops at its first instruction in the
#                  debugger (with --mcp, a script or agent sets breakpoints, then RUN)
#   --mcp PORT     connect the debugger control channel to 127.0.0.1:PORT, where the
#                  dosbox-x-mcp-server (agents, port 58991) or tools/dos/dbx_control.py
#                  (scripts) listens; start that listener first
#   --cycles N     a fixed CPU speed (DOSBox-X "cycles=fixed N"); use one value for every run
#   --conf FILE    a DOSBox configuration to start from (repeatable), such as the one a GOG or
#                  Steam release ships; this script's own settings come after it and win
#                  (window, autoexec, logging). Fullscreen is turned off.
#   --set "SECTION KEY=VALUE"   any other DOSBox-X setting (repeatable)
#   --in-place     mount GAME_DIR itself (default: a fresh copy in OUT/drive, so saves and
#                  settings the program writes never change your copy of the game)
#   --then LINE    a DOS command to run after PROGRAM (repeatable), for example to copy a file
#
# It writes RUN_RE.BAT into the mounted drive (removed afterwards; with --in-place that is your
# game folder), OUT/dosbox.log (DOSBox-X's log, with the file and INT 21h logs when asked),
# OUT/exitcode.txt (the program's exit code, read with IF ERRORLEVEL) and prints the exit code.
# Needs dosbox-x (scripts/setup.sh --install). It does not script the debugger, take
# screenshots or feed keys: see docs/DOS.md for those.
set -euo pipefail
. "$(dirname "$0")/lib.sh"
out="$ROOT/analysis/dosbox"; time=60; window=0; inplace=0; flags=(); sets=(); after=(); prefix=""; tty=0; confs=()

while [[ "${1:-}" == --* ]]; do
  case "$1" in
    --out) out="$2"; shift 2 ;;
    --time) time="$2"; shift 2 ;;
    --window) window=1; shift ;;
    --fileio) flags+=(-log-fileio); sets+=(-set "log files=debug"); shift ;;
    --int21) flags+=(-log-int21); shift ;;
    --debug) flags+=(-break-start); window=1; tty=1; shift ;;
    --break) prefix="DEBUGBOX "; tty=1; shift ;;
    --mcp)
      if ! dosbox_has_debug_channel; then
        echo "--mcp needs DOSBox-X 2026.10 or later with the debugger (this one: $(dosbox_version)); see the re-setup skill" >&2
        exit 2
      fi
      sets+=(-set "dosbox mcp_server=$2"); tty=1; shift 2 ;;
    --cycles) sets+=(-set "cpu cycles=fixed $2"); shift 2 ;;
    --set) sets+=(-set "$2"); shift 2 ;;
    --conf) confs+=(-conf "$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"); shift 2 ;;
    --in-place) inplace=1; shift ;;
    --then) after+=("$2"); shift 2 ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
done
[[ $# -ge 2 ]] || { sed -n '2,31p' "$0"; exit 2; }
command -v dosbox-x >/dev/null || { echo "dosbox-x not found: scripts/setup.sh --install" >&2; exit 1; }
game="$(cd "$1" && pwd)"; shift
mkdir -p "$out"; out="$(cd "$out" && pwd)"
if [[ $inplace == 0 && "$out/" == "$game/"* ]]; then
  echo "--out ($out) is inside GAME_DIR ($game): the copy of GAME_DIR would include it. Put it elsewhere." >&2
  exit 2
fi
drive="$game"
if [[ $inplace == 0 ]]; then
  drive="$out/drive"; rm -rf "$drive"; cp -R "$game" "$drive"
fi
rm -f "$drive/EXITCODE.TXT" "$drive/RUN_RE.BAT" "$out/dosbox.log"
# The commands go in a batch file on the drive: a long [autoexec] section overflows.
{
  printf '@ECHO OFF\r\n%s%s\r\n' "$prefix" "$*"
  # A redirect inside a false IF still creates the file in DOSBox-X: append, keep the largest.
  for n in $(seq 1 255); do printf 'IF ERRORLEVEL %d ECHO E%d>> EXITCODE.TXT\r\n' "$n" "$n"; done
  for line in ${after[@]+"${after[@]}"}; do printf '%s\r\n' "$line"; done
} > "$drive/RUN_RE.BAT"
conf="$out/run.conf"
printf '[log]\nlogfile = %s\n[autoexec]\nmount c "%s"\nc:\nCALL RUN_RE.BAT\nexit\n' "$out/dosbox.log" "$drive" > "$conf"
env=()
[[ $window == 0 ]] && env=(SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy)
mode=(-silent)
[[ $window == 1 ]] && mode=(-exit)
cmd=(dosbox-x ${confs[@]+"${confs[@]}"} -conf "$conf" -set "sdl fullscreen=false" "${mode[@]}" -fastlaunch -nomenu
  -time-limit "$time" ${flags[@]+"${flags[@]}"} ${sets[@]+"${sets[@]}"})
# DOSBox-X enables its debugger only in a terminal: give it a pseudo-terminal with script(1).
# macOS script takes the command as separate arguments; util-linux script (Linux) takes it as
# one string after -c, so the arguments are quoted into that string.
if [[ $tty == 1 && $(uname -s) == Darwin ]]; then
  cmd=(script -q /dev/null "${cmd[@]}")
elif [[ $tty == 1 && $(uname -s) == Linux ]]; then
  cmd=(script -qec "$(printf '%q ' "${cmd[@]}")" /dev/null)
fi
env TERM="${TERM:-xterm}" ${env[@]+"${env[@]}"} "${cmd[@]}" > "$out/console.txt" 2>&1 || true
code=0
[[ -f "$drive/EXITCODE.TXT" ]] && code="$(tr -d '\r E' < "$drive/EXITCODE.TXT" | sed '/^$/d' | tail -1)"
rm -f "$drive/RUN_RE.BAT" "$drive/EXITCODE.TXT"
echo "$code" > "$out/exitcode.txt"
echo "exit code $code (log: $out/dosbox.log)"
