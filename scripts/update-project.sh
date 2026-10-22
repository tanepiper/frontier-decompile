#!/usr/bin/env bash
# Bring a project made with GameFrame up to GameFrame's current version.
#
#   scripts/update-project.sh DIR [--dry-run]
#
# A project made by new-project.sh holds a copy of GameFrame's toolkit, skills, agents, gates and
# guides. This script refreshes that copy. For each file GameFrame provides:
#   - missing in the project: added;
#   - the same as GameFrame's: left alone;
#   - an older version of GameFrame's file (its exact contents are in GameFrame's git history):
#     replaced with the current one;
#   - changed in the project (contents GameFrame never had): kept, and listed at the end, so a
#     project's own edits are never lost.
# The project's own files (AGENTS.md, README.md, docs/FINDINGS.md, docs/CHECKLIST.md, labels/,
# its code) are never touched. An existing .claude/settings.json is kept and listed; a missing one
# is added with the gates. With --dry-run nothing is written. The project's git history shows
# every change; commit it like any other.
set -euo pipefail
. "$(dirname "$0")/lib.sh"
[[ $# -ge 1 ]] || { sed -n '2,20p' "$0"; exit 2; }
dir="$1"; dry="${2:-}"
[[ -d "$dir/.git" || -f "$dir/.git" ]] || { echo "$dir is not a git repository" >&2; exit 1; }
"${PYTHON:-python3}" - "$ROOT" "$dir" "$dry" <<'PY'
import os, shutil, subprocess, sys
root, dest, dry = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2]), sys.argv[3] == '--dry-run'

def blob(path):
    return subprocess.run(['git', '-C', root, 'hash-object', path], capture_output=True, text=True).stdout.strip()

def known(h):
    return subprocess.run(['git', '-C', root, 'cat-file', '-e', h], capture_output=True).returncode == 0

def tracked(sub):
    out = subprocess.run(['git', '-C', root, 'ls-files', '-z', sub], capture_output=True, text=True).stdout
    return [f for f in out.split('\0') if f]

# (path in GameFrame, path in the project)
pairs = []
for sub in ('tools', 'scripts'):
    pairs += [(f, f) for f in tracked(sub)]
pairs += [(f, f) for f in tracked('.claude/skills') if f.split('/')[2].startswith('re-')]
pairs += [(f, '.claude/' + f) for f in tracked('agents')]
pairs += [(f, f) for f in ('gameframe', 'requirements.txt', '.mcp.json')]
for f in ('ABI', 'GAME-PATTERNS', 'DOS', 'LEGAL', 'OUTPUTS', 'SETUP', 'COMPILED-CODE', 'TYPES'):
    pairs.append((f'docs/{f}.md', f'docs/{f}.md'))
for d in ('games', 'observe', 'engines', 'hardware', 'consoles', 'handhelds', 'computers', 'arcade', 'media',
          'working', 'teams', 'case-studies', 'preservation', 'assets'):
    pairs += [(f, f) for f in tracked(f'docs/{d}')]

added, replaced, kept, same = [], [], [], 0
for src, dst in pairs:
    s, d = os.path.join(root, src), os.path.join(dest, dst)
    if not os.path.isfile(s):
        continue
    if not os.path.exists(d):
        added.append(dst)
    elif blob(d) == blob(s):
        same += 1
        continue
    elif known(blob(d)):
        replaced.append(dst)
    else:
        kept.append(dst)
        continue
    if not dry:
        os.makedirs(os.path.dirname(d) or '.', exist_ok=True)
        shutil.copy2(s, d)

settings = os.path.join(dest, '.claude', 'settings.json')
if os.path.exists(settings):
    kept.append('.claude/settings.json (compare it with GameFrame\'s: the gates are in "hooks")')
else:
    added.append('.claude/settings.json')
    if not dry:
        os.makedirs(os.path.dirname(settings), exist_ok=True)
        shutil.copy2(os.path.join(root, '.claude', 'settings.json'), settings)

verb = 'would be' if dry else 'were'
print(f'{len(added)} files {verb} added, {len(replaced)} {verb} replaced with the current version, {same} were already current.')
for name, files in (('Added', added), ('Replaced', replaced)):
    if files:
        print(f'{name}: ' + ', '.join(sorted(files)[:12]) + (f' and {len(files) - 12} more' if len(files) > 12 else ''))
if kept:
    print(f'Kept, because the project changed them ({len(kept)}):')
    for f in sorted(kept):
        print(f'  {f}')
PY
