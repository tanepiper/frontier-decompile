"""One function per command of the CLI. Each takes its parsed arguments and returns an exit code.

Output is plain text for people, or JSON (--json) for agents and scripts. The text comes from
the skills and the tools themselves (catalog.py, skills.py); nothing here copies it.
"""
import json
import shutil
import subprocess
import sys
import textwrap

import agents
import catalog
import skills
from paths import NAME, SCRIPTS, python, rel
from search import search

GROUPS = {'disc': 'Discs and disk images', 'pe': 'Windows PE', 'dos': '16-bit DOS', 'emu': 'Emulators', 'triage': 'Triage',
          'formats': 'Formats', 'verify': 'Verify', 'ghidra': 'Ghidra scripts',
          'scripts': 'Scripts'}


def _width():
    return min(shutil.get_terminal_size((100, 24)).columns, 100)


def _row(name, text, pad):
    """`name` in a column of `pad`, then `text` wrapped under itself."""
    lines = textwrap.wrap(text, max(_width() - pad - 2, 30)) or ['']
    out = [f'  {name:<{pad}}{lines[0]}'] + [' ' * (pad + 2) + line for line in lines[1:]]
    return '\n'.join(out)


def _dump(data):
    print(json.dumps(data, indent=2))
    return 0


def _not_found(what, name, close):
    hint = f' Did you mean: {", ".join(close)}?' if close else ''
    print(f'{NAME}: no {what} named {name!r}.{hint} See `{NAME} {what}s`.', file=sys.stderr)
    return 2


def steps(args):
    diagram, rows = skills.steps()
    if args.json:
        return _dump([{'step': s, 'question': q, 'skill': k} for s, q, k in rows])
    print(diagram + '\n')
    pad = max(len(s) for s, _, _ in rows) + 2
    for step, question, skill in rows:
        print(_row(step, f'{skill}: {question}', pad))
    print(f'\nRead a step with `{NAME} skill NAME`. Start with `{NAME} skill re-start`.')
    return 0


def list_skills(args):
    every = skills.skills()
    if args.json:
        return _dump([s.as_dict() for s in every])
    pad = max(len(s.name) for s in every) + 2
    for s in every:
        print(_row(s.name, s.description.split(' Use ')[0], pad))
    print(f'\nPrint one with `{NAME} skill NAME`, or one part with `--section TITLE`.')
    return 0


def skill(args):
    s, close = skills.find(args.name)
    if not s:
        return _not_found('skill', args.name, close)
    if args.sections:
        for level, title, _ in skills.headings(s.body):
            print('  ' * (level - 1) + title)
        return 0
    if args.section:
        text = skills.section(s.body, args.section)
        if text is None:
            print(f'{NAME}: {s.name} has no section matching {args.section!r}. '
                  f'See `{NAME} skill {s.name} --sections`.', file=sys.stderr)
            return 2
        print(text)
        return 0
    print(s.body.rstrip())
    return 0


def list_agents(args):
    if args.table:
        print(agents.table())
        return 0
    found = agents.agents()
    if args.json:
        return _dump(found)
    pad = max(len(a['name']) for a in found) + 2
    for a in found:
        print(_row(a['name'], a['summary'], pad))
    return 0


def list_tools(args):
    every = catalog.tools()
    if args.json:
        return _dump([t.as_dict() for t in every])
    pad = max(len(t.name) for t in every) + 2
    group = None
    for t in every:
        if t.group != group:
            group = t.group
            print(f'\n{GROUPS.get(group, group)} ({"scripts" if group == "scripts" else "tools/" + group})')
        mark = '' if t.runs else f'[{t.kind}] '
        print(_row(t.name, mark + (t.summary or '(no help yet)'), pad))
    print(f'\nShow one with `{NAME} tool NAME`; run it with `{NAME} run NAME ARGS`.')
    return 0


def tool(args):
    t, close = catalog.find(args.name)
    if not t:
        return _not_found('tool', args.name, close)
    if args.json:
        return _dump(dict(t.as_dict(), doc=t.doc))
    print(f'{rel(t.path)}  ({t.kind})\n')
    print(t.doc.rstrip() or '(this tool has no help yet)')
    if t.runs:
        print(f'\nRun it: {NAME} run {t.name} ARGS')
    elif t.kind == 'ghidra':
        print(f'\nRun it through Ghidra: {NAME} run ghidra-run BINARY ... or {NAME} run ghidra-export')
    return 0


def run(name, rest):
    t, close = catalog.find(name)
    if not t:
        return _not_found('tool', name, close)
    if not t.runs:
        how = ('a Ghidra script: run it with scripts/ghidra-run.sh or ghidra-export.sh'
               if t.kind == 'ghidra' else 'a library to import from Python code')
        print(f'{NAME}: {t.name} is {how}. See `{NAME} tool {t.name}`.', file=sys.stderr)
        return 2
    py, note = python()
    if note and t.kind == 'program':
        print(note, file=sys.stderr)
    return subprocess.call(catalog.command(t, py, rest))


def find(args):
    hits = search(args.word)
    if args.json:
        return _dump([dict(zip(('kind', 'name', 'where', 'line'), h)) for h in hits])
    if not hits:
        print(f'Nothing mentions {args.word!r}.')
        return 1
    shown = {}
    for kind, name, where, line in hits:
        key = (kind, name)
        shown[key] = shown.get(key, 0) + 1
        if shown[key] == 1:
            print(f'\n{kind} {name}')
        if shown[key] <= args.max:
            print(f'  [{where}] {line}' if where else f'  {line}')
    more = {k: n - args.max for k, n in shown.items() if n > args.max}
    if more:
        print('\nMore lines: ' + ', '.join(f'{name} +{n}' for (_, name), n in more.items())
              + ' (use --max N)')
    return 0


def doctor(rest):
    return subprocess.call(['bash', str(SCRIPTS / 'setup.sh'), *rest])
