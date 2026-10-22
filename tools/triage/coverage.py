#!/usr/bin/env python3
"""How much of the original's code does the reimplementation cite?

Every reimplemented item names the original function it came from (the re-port skill:
``Original: `FUN_1400be680` ``). This tool reads those citations from the sources and compares
them with the functions in a Ghidra export, to show how much of the original is covered and
where the largest uncovered functions are.

Usage:
  coverage.py [--export DIR] [--binary NAME] [--top N] [--list-cited] SOURCE_DIR ...

  SOURCE_DIR    the reimplementation's folders to read
  --export      the Ghidra export (default: analysis/export)
  --binary      the binary in the export (default: the only one, or the first .exe)
  --top N       list the N largest functions nobody cites (default 20; 0 for none)
  --list-cited  also list every cited function

A function counts as cited when the sources contain its address as `FUN_<hex>` or `0x<hex>`
(16-bit: `FUN_1000_873a` or `1000:873a`), or its name in backticks after `Original:` (a Ghidra
name, or one of your labels from labels/names.tsv). Thunks are left out of the totals. The share
by size weighs each function by its bytes, which says more than the count: a thousand small
getters are less work than one large update.

What it does not do: decide whether a cited function is fully reimplemented. A citation means
somebody ported from it; the checklist (docs/CHECKLIST.md) says how far.
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from decomp_query import parse_addr  # noqa: E402

SKIP = {'.git', 'target', 'build', 'analysis', 'derived', 'node_modules', '.venv', '__pycache__'}
ADDRESS = re.compile(r'\bFUN_[0-9a-fA-F]+(?:_[0-9a-fA-F]{4})?\b|\b0x[0-9a-fA-F]{5,}\b|\b[0-9a-fA-F]{4}:[0-9a-fA-F]{4}\b')
ORIGINAL = re.compile(r'Original:\s*((?:`[^`]+`[,\s]*(?:and\s*)?)+)')
TICKED = re.compile(r'`([^`]+)`')


def functions(export, binary):
    """The export's functions: {address: (size, name)}, thunks left out."""
    if not binary:
        names = sorted(os.listdir(export))
        exes = [n for n in names if n.lower().endswith('.exe')]
        binary = (exes or names)[0]
    out = {}
    for line in open(os.path.join(export, binary, 'functions.tsv'), encoding='utf-8'):
        parts = line.rstrip('\n').split('\t')
        if len(parts) >= 3 and parts[-1] != 'thunk':
            out[parse_addr(parts[0])] = (int(parts[1]), parts[2])
    return binary, out


def labels(path='labels/names.tsv'):
    """Your labels: {label: address}."""
    out = {}
    if os.path.exists(path):
        for line in open(path, encoding='utf-8'):
            parts = line.rstrip('\n').split('\t')
            if len(parts) >= 2 and not line.startswith('#'):
                out[parts[1]] = parse_addr(parts[0])
    return out


def citations(dirs, by_name):
    """Every address the sources cite, with the first file and line that cites it."""
    cited = {}
    for top in dirs:
        for dirpath, subdirs, files in os.walk(top):
            subdirs[:] = sorted(d for d in subdirs if d not in SKIP)
            for name in sorted(files):
                path = os.path.join(dirpath, name)
                try:
                    lines = open(path, encoding='utf-8').read().split('\n')
                except (UnicodeDecodeError, OSError):
                    continue
                for i, line in enumerate(lines, 1):
                    found = [parse_addr(t) for t in ADDRESS.findall(line)]
                    for m in ORIGINAL.finditer(line):
                        found += [by_name[t] for t in TICKED.findall(m.group(1)) if t in by_name]
                    for a in found:
                        cited.setdefault(a, f'{path}:{i}')
    return cited


def main(argv):
    if not argv or '-h' in argv or '--help' in argv:
        print(__doc__)
        return 0
    export, binary, top, list_cited, dirs = 'analysis/export', None, 20, False, []
    args = iter(argv)
    for a in args:
        if a == '--export':
            export = next(args)
        elif a == '--binary':
            binary = next(args)
        elif a == '--top':
            top = int(next(args))
        elif a == '--list-cited':
            list_cited = True
        else:
            dirs.append(a)
    binary, fns = functions(export, binary)
    by_name = {name: a for a, (_, name) in fns.items()}
    by_name.update(labels())
    cited = {a: where for a, where in citations(dirs, by_name).items() if a in fns}
    total = sum(s for s, _ in fns.values())
    done = sum(fns[a][0] for a in cited)
    print(f'{binary}: {len(cited)} of {len(fns)} functions cited '
          f'({100 * len(cited) / max(len(fns), 1):.1f} %), '
          f'{done} of {total} bytes ({100 * done / max(total, 1):.1f} % by size)')
    if list_cited:
        print('\ncited:')
        for a in sorted(cited):
            print(f'  {a:x}\t{fns[a][0]}\t{fns[a][1]}\t{cited[a]}')
    if top:
        rest = sorted(((s, a, n) for a, (s, n) in fns.items() if a not in cited), reverse=True)[:top]
        if rest:
            print(f'\nlargest not cited (bytes, address, name):')
            for s, a, n in rest:
                print(f'  {s}\t{a:x}\t{n}')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
