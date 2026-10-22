#!/usr/bin/env bash
# Get a game's files out of the installer or package it came in, without running it.
#
#   scripts/extract-installer.sh INSTALLER OUT_DIR    detect the kind, then extract into OUT_DIR
#   scripts/extract-installer.sh --detect INSTALLER   only say what kind it is and which tool reads it
#
# Kinds and the tool used (install the tool with scripts/setup.sh --install):
#   Inno Setup .exe (GOG and many others; GOG's extra .bin parts next to it are read too)  innoextract
#   NSIS .exe (Nullsoft)                    7z
#   InstallShield cabinets (data1.cab ...)  unshield, else 7z
#   Windows Installer .msi                  7z
#   .zip .7z .rar .cab, self-extracting archives          7z (or unzip for .zip)
#   GOG Linux .sh (MojoSetup: a zip appended to a script)  unzip
#   macOS .dmg                              hdiutil (mounted read-only, copied, detached)
#   macOS .pkg                              pkgutil --expand-full
#   CD images: .cue with .bin, .iso, raw .bin  tools/disc/discinfo.py --extract (and --audio
#                                           for CD audio tracks; it also lists the tracks)
#   macOS .app                              copied as it is; a tiny app is often only a web
#                                           installer (GOG Galaxy's), with no game inside
# It never runs the installer. Extract into an ignored folder (target/), never into git.
set -euo pipefail
PY="$(command -v python3)"
detect_only=0
[[ "${1:-}" == --detect ]] && { detect_only=1; shift; }
[[ $# -ge 1 ]] || { sed -n '2,22p' "$0"; exit 2; }
src="$1"; out="${2:-}"
[[ -e "$src" ]] || { echo "$src: no such file" >&2; exit 1; }
[[ $detect_only == 1 || -n "$out" ]] || { echo "give an output folder" >&2; exit 2; }

# grep the file itself: `strings | grep -q` fails under pipefail when grep stops early.
has() { local f="$1"; shift; local p; for p in "$@"; do LC_ALL=C grep -a -q -F -- "$p" "$f" && return 0; done; return 1; }
need() { command -v "$1" >/dev/null || { echo "$kind needs $1: scripts/setup.sh --install (or install $1)" >&2; exit 1; }; }

kind=unknown; tool=
lower="$(echo "$src" | tr '[:upper:]' '[:lower:]')"
if [[ -d "$src" && "$lower" == *.app ]]; then
  kind="macOS app bundle"; tool=copy
  size=$(du -sk "$src" | cut -f1)
  if [[ -e "$src/Contents/MacOS/GalaxyWebInstaller" ]] || (( size < 2048 )); then
    kind="macOS app bundle (a ${size} KB launcher or web installer: probably no game files inside)"
  fi
elif [[ -d "$src" ]]; then
  kind="folder"; tool=copy
else
  desc="$(file -b "$src")"
  head4="$(head -c 4 "$src" | xxd -p)"
  case "$desc" in
    *"Zip archive"*) kind="zip"; tool=unzip ;;
    *"7-zip"*) kind="7z"; tool=7z ;;
    *RAR*) kind="rar"; tool=7z ;;
    *"Microsoft Cabinet"*) kind="cab"; tool=7z ;;
    *"zlib compressed"*|*"Apple Disk Image"*|*"bzip2"*) [[ "$lower" == *.dmg ]] && { kind="macOS disk image"; tool=hdiutil; } ;;
    *"xar archive"*) kind="macOS package"; tool=pkgutil ;;
    *"Composite Document File"*|*"CDFV2"*) kind="Windows Installer (MSI)"; tool=7z ;;
  esac
  if [[ "$lower" == *.cue ]] || { [[ $kind == unknown ]] && [[ "$lower" == *.iso || "$lower" == *.bin ]] &&
       "$PY" "$(dirname "$0")/../tools/disc/discinfo.py" "$src" --json 2>/dev/null | grep -q '"kind": "\(ISO 9660\|Joliet\|HFS\)'; }; then
    kind="CD image"; tool=discinfo
  fi
  if [[ $kind == unknown && "$lower" == *.dmg ]]; then kind="macOS disk image"; tool=hdiutil; fi
  if [[ $kind == unknown && "$desc" == *"PE32"* ]]; then
    if has "$src" "Inno Setup Setup Data"; then kind="Inno Setup"; tool=innoextract
    elif has "$src" "Nullsoft"; then kind="NSIS"; tool=7z
    elif has "$src" "InstallShield"; then kind="InstallShield"; tool=7z
    else kind="Windows program (maybe a self-extracting archive)"; tool=7z; fi
  fi
  if [[ $kind == unknown && "$head4" == 23212f* ]] && has "$src" MojoSetup makeself; then
    kind="GOG Linux installer (MojoSetup)"; tool=unzip
  fi
fi
echo "$src: $kind${tool:+ -> $tool}"
[[ $detect_only == 1 ]] && exit 0
[[ $kind == unknown ]] && { echo "unknown kind: look at it with file and xxd" >&2; exit 1; }
mkdir -p "$out"
case "$tool" in
  innoextract) need innoextract; innoextract --silent --gog -d "$out" "$src" ;;
  7z) if command -v 7z >/dev/null; then 7z x -y -o"$out" "$src" >/dev/null
      elif command -v 7zz >/dev/null; then 7zz x -y -o"$out" "$src" >/dev/null
      elif [[ $kind == InstallShield ]] && command -v unshield >/dev/null; then unshield -d "$out" x "$src"
      else need 7z; fi ;;
  unzip) need unzip; unzip -qo "$src" -d "$out" || [[ $? == 1 ]] ;;   # 1: warnings (a script before the zip)
  hdiutil) mnt="$(mktemp -d)"; hdiutil attach -readonly -nobrowse -mountpoint "$mnt" "$src" >/dev/null
           cp -R "$mnt"/. "$out"/; hdiutil detach "$mnt" >/dev/null ;;
  pkgutil) rmdir "$out" 2>/dev/null || true; pkgutil --expand-full "$src" "$out" ;;
  copy) cp -R "$src" "$out"/ ;;
  discinfo) "$PY" "$(dirname "$0")/../tools/disc/discinfo.py" "$src" --extract "$out" --audio "$out/cd-audio" ;;
esac
echo "extracted into $out: $(find "$out" -type f | wc -l | tr -d ' ') files, $(du -sh "$out" | cut -f1)"
echo "next: compare with any other copy (tools/triage/compare_trees.py), then re-inventory"
