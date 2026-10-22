#!/usr/bin/env bash
# Start a reverse-engineering project for one game, with GameFrame's toolkit and playbook.
#
#   scripts/new-project.sh DIR "Game Name" [PATH/TO/GAME]
#
# Creates DIR as a git repository with:
#   AGENTS.md (+ CLAUDE.md)   the rules, with the game's name; edit the goal paragraph first
#   README.md, LICENSE-APACHE, LICENSE-CC0-1.0   front page (build/run per part); code Apache 2.0, content CC0 1.0
#   .claude/skills/           the re-* skills (every step of the method)
#   .claude/agents/           the stage and task agents, which hand off to each other
#   .claude/settings.json     light permissions and the gates (scripts/hooks/gate.py)
#   tools/, scripts/          GameFrame's toolkit, requirements.txt, .mcp.json (REA)
#   gameframe                 the help and discovery command (./gameframe help)
#   docs/FINDINGS.md, docs/CHECKLIST.md, labels/names.tsv, labels/fields.tsv
#   docs/DOS.md, ABI.md, GAME-PATTERNS.md, COMPILED-CODE.md, TYPES.md, LEGAL.md, OUTPUTS.md, SETUP.md
#                             the reference documents
#   docs/games/, observe/, working/ and the other guides, with their diagrams in docs/assets/
# scripts/update-project.sh brings a project made earlier up to GameFrame's current version.
#   target/, analysis/, derived/   for the game, Ghidra output and ground truth: ignored by git
# With PATH/TO/GAME it copies the game's executables and DLLs into target/ (data files are
# left where they are: point the tools at them) and prints a first look at each binary.
set -euo pipefail
. "$(dirname "$0")/lib.sh"
[[ $# -ge 2 ]] || { sed -n '2,15p' "$0"; exit 2; }
dir="$1"; name="$2"; game="${3:-}"
[[ -e "$dir" && -n "$(ls -A "$dir" 2>/dev/null)" ]] && { echo "$dir exists and is not empty" >&2; exit 1; }
mkdir -p "$dir"/{target,analysis,derived,docs,labels,.claude}
cp -R "$ROOT/tools" "$ROOT/scripts" "$dir/"
cp -R "$ROOT/.claude/skills" "$dir/.claude/"
cp -R "$ROOT/agents" "$dir/.claude/agents"
# The permissions and the gates (scripts/hooks/gate.py): docs/working/gates.md.
cp "$ROOT/.claude/settings.json" "$dir/.claude/settings.json"
cp "$ROOT/requirements.txt" "$ROOT/.mcp.json" "$ROOT/gameframe" "$dir/"
# Code is Apache 2.0; documentation and other prose are CC0 1.0 (as in GameFrame).
cp "$ROOT/templates/project/licences/LICENSE-APACHE" "$ROOT/templates/project/licences/LICENSE-CC0-1.0" "$dir/"
# The reference documents the skills point to (docs/DOS.md, docs/ABI.md, ...).
for f in ABI GAME-PATTERNS DOS LEGAL OUTPUTS SETUP COMPILED-CODE TYPES; do cp "$ROOT/docs/$f.md" "$dir/docs/"; done
# The guides, and the diagrams and pictures they show (docs/assets/).
for d in games observe engines hardware consoles handhelds computers arcade media working teams case-studies preservation assets; do
  cp -R "$ROOT/docs/$d" "$dir/docs/"
done
cp "$ROOT/templates/project/gitignore" "$dir/.gitignore"
cp "$ROOT/templates/project/labels/"*.tsv "$dir/labels/"
for f in AGENTS.md README.md docs/FINDINGS.md docs/CHECKLIST.md; do
  sed -e "s/GAME_NAME/$name/g" -e "s/DATE/$(date +%Y-%m-%d)/" "$ROOT/templates/project/$f" > "$dir/$f"
done
printf 'The instructions for this repository are in AGENTS.md:\n\n@AGENTS.md\n' > "$dir/CLAUDE.md"
# target/ holds the game: Cargo must build elsewhere, or `cargo clean` deletes the game.
mkdir -p "$dir/.cargo"
printf '[build]\n# target/ holds your copy of the game: never let Cargo build (or clean) there.\ntarget-dir = "build"\n' > "$dir/.cargo/config.toml"
[[ -f "$ROOT/.env" ]] && cp "$ROOT/.env" "$dir/.env"
find "$dir/tools" -name '__pycache__' -prune -exec rm -r {} + 2>/dev/null || true
if [[ -n "$game" ]]; then
  find "$game" -maxdepth 2 -type f \( -iname '*.exe' -o -iname '*.dll' -o -iname '*.ovl' \) -exec cp {} "$dir/target/" \;
  echo "Copied $(ls "$dir/target" | wc -l | tr -d ' ') binaries into $dir/target/"
fi
(cd "$dir" && git init -q && git add -A && git commit -qm "Start reverse engineering $name" && echo "Created $dir (git repository, first commit made)")
if [[ -n "$game" && -x "$PYTHON" ]]; then
  for b in "$dir"/target/*; do
    if "$PYTHON" "$ROOT/tools/pe/peinfo.py" "$b" >/dev/null 2>&1; then "$PYTHON" "$ROOT/tools/pe/peinfo.py" "$b" | head -4
    else "$PYTHON" "$ROOT/tools/dos/dosinfo.py" "$b" 2>/dev/null | head -6; fi
  done
fi
cat <<NEXT

Next:
  1. Edit the goal paragraph in $dir/AGENTS.md.
  2. cd $dir && scripts/setup.sh --install
  3. Ask your agent to start with the re-start skill (it routes every step from there).
NEXT
