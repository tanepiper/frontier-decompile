#!/usr/bin/env python3
"""Claude Code hooks for GameFrame and the projects made with it.

Usage (from .claude/settings.json or the plugin's hooks/hooks.json; the hook's JSON is on stdin):

    gate.py commit     PreToolUse, Bash `git commit ...`
    gate.py push       PreToolUse, Bash `git push ...`
    gate.py write      PreToolUse, Write and Edit
    gate.py session    SessionStart

What each one does:

- commit: blocks a commit that stages game material (a file in target/, analysis/, derived/,
  game-data/ or save-game/, or a game binary or data file such as .exe, .dll, .pkg, .sav) or
  added lines that look like decompiled code. It also blocks a commit made by a GameFrame stage
  or task agent other than re-lander, which is the only agent that commits.
- push: blocks a push made by a GameFrame agent other than re-lander. The main session can
  push; the project's permissions ask the user first.
- write: blocks writing decompiled code (Ghidra pseudocode) into a file outside analysis/,
  derived/ and target/. Findings are written in prose, with addresses and constants.
- session: adds a few facts to the start of a session: the tools found, and the next open
  items in docs/CHECKLIST.md.

A block exits with status 2 and says why on stderr, which Claude Code shows to the agent. Every
other case exits 0 and leaves the decision to the normal permissions. Hooks are a safety net
for honest mistakes. They are not a security boundary: an agent that wants to get around them
can. Standard library only.
"""
import json
import os
import re
import subprocess
import sys

# Folders that hold the game, its derivatives or decompiled code. They are ignored by git; a
# staged path there was force-added.
GAME_DIRS = ('target/', 'analysis/', 'derived/', 'game-data/', 'save-game/')
# Extensions of game binaries and data files that never belong in a repository.
GAME_EXTS = {
    '.exe', '.dll', '.ovl', '.com', '.sys', '.pkg', '.crs', '.rsc', '.str', '.sav', '.wad',
    '.pak', '.pk3', '.grp', '.mpq', '.bif', '.gob', '.rez', '.vpk', '.iso', '.cue', '.bin',
    '.nes', '.sfc', '.smc', '.gba', '.gb', '.gbc', '.n64', '.z64', '.v64', '.md', '.gen',
}
# .md is a Mega Drive ROM extension too; Markdown is far more common in a repository.
GAME_EXTS.discard('.md')
# Lines that Ghidra's decompiler writes and people do not.
PSEUDOCODE = re.compile(
    r'\bundefined[1248]?\b|\bCONCAT\d\d\(|\bSUB\d\d\(|\bZEXT\d\d\(|\(\*\*\(code \*\*\)|'
    r'/\* WARNING: |\bunaff_\w+|\bin_stack_\w+|\bextraout_\w+'
)
# Paths where decompiled code may live (all ignored by git).
PRIVATE_DIRS = ('analysis/', 'derived/', 'target/')
LANDER = 're-lander'


def block(reason):
    print(reason, file=sys.stderr)
    sys.exit(2)


def gameframe_agent(event):
    """The GameFrame agent this hook fires in (re-tracer or gameframe:re-tracer), or None."""
    name = (event.get('agent_type') or '').split(':')[-1]
    return name if name.startswith('re-') else None


def git(*args, cwd):
    return subprocess.run(['git', *args], cwd=cwd, capture_output=True, text=True).stdout


def pseudocode_lines(text):
    return [line for line in text.splitlines() if PSEUDOCODE.search(line)]


def commit(event):
    cwd = event.get('cwd') or os.getcwd()
    agent = gameframe_agent(event)
    if agent and agent != LANDER:
        block(f'{agent} does not commit: re-lander commits and pushes. End with your hand-off '
              f'and name re-lander as Next.')
    staged = [p for p in git('diff', '--cached', '--name-only', cwd=cwd).splitlines() if p]
    # `git commit -a` stages tracked changes as it commits: check those too.
    if re.search(r'(^|\s)(-a|--all|-[a-zA-Z]*a[a-zA-Z]*)(\s|$)', event['tool_input'].get('command', '')):
        staged += [p for p in git('diff', '--name-only', cwd=cwd).splitlines() if p]
    bad = [p for p in staged
           if p.startswith(GAME_DIRS) or os.path.splitext(p)[1].lower() in GAME_EXTS]
    if bad:
        block('This commit stages game material, which never goes into git:\n  '
              + '\n  '.join(bad[:10])
              + '\nUnstage it (git restore --staged PATH). See docs/LEGAL.md.')
    added = [l[1:] for l in git('diff', '--cached', '--unified=0', cwd=cwd).splitlines()
             if l.startswith('+') and not l.startswith('+++')]
    hits = pseudocode_lines('\n'.join(added))
    if len(hits) >= 2:
        block('The staged changes contain lines that look like decompiled code:\n  '
              + '\n  '.join(h.strip()[:100] for h in hits[:5])
              + '\nDescribe the behaviour in prose with addresses and constants instead.')


def push(event):
    agent = gameframe_agent(event)
    if agent and agent != LANDER:
        block(f'{agent} does not push: re-lander commits and pushes. End with your hand-off.')


def write(event):
    tool = event.get('tool_input', {})
    path = tool.get('file_path', '')
    root = event.get('cwd') or os.getcwd()
    rel = os.path.relpath(path, root) if os.path.isabs(path) else path
    if rel.startswith(PRIVATE_DIRS) or rel.startswith('..'):
        return
    text = tool.get('content') or tool.get('new_string') or ''
    hits = pseudocode_lines(text)
    if len(hits) >= 2:
        block(f'{rel}: this looks like decompiled code, which stays in analysis/:\n  '
              + '\n  '.join(h.strip()[:100] for h in hits[:5])
              + '\nWrite the finding in prose, with the function address, offsets and constants.')


def session(event):
    root = event.get('cwd') or os.getcwd()
    facts = []
    env = os.path.join(root, '.env')
    facts.append('.env present' if os.path.exists(env) else 'no .env: run scripts/setup.sh')
    facts.append('.venv present' if os.path.isdir(os.path.join(root, '.venv')) else
                 'no .venv: run scripts/setup.sh --install')
    if os.path.isdir(os.path.join(root, 'analysis', 'export')):
        facts.append('Ghidra export present in analysis/export')
    checklist = os.path.join(root, 'docs', 'CHECKLIST.md')
    todo = []
    if os.path.exists(checklist):
        for line in open(checklist, encoding='utf-8', errors='replace'):
            if re.match(r'\s*[-*] \[ \]', line):
                todo.append(line.strip()[6:].strip())
            if len(todo) == 3:
                break
    text = 'GameFrame project: ' + '; '.join(facts) + '.'
    if todo:
        text += ' Next open items in docs/CHECKLIST.md: ' + ' | '.join(todo) + '.'
    print(json.dumps({'hookSpecificOutput': {'hookEventName': 'SessionStart',
                                             'additionalContext': text}}))


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in ('commit', 'push', 'write', 'session'):
        print(__doc__)
        return 0 if len(sys.argv) == 1 else 2
    try:
        event = json.load(sys.stdin)
    except ValueError:
        return 0  # no usable input: leave the decision to the permissions
    {'commit': commit, 'push': push, 'write': write, 'session': session}[sys.argv[1]](event)
    return 0


if __name__ == '__main__':
    sys.exit(main())
