#!/usr/bin/env python3
"""Find the code that runs only when you do something in the game: compare the code coverage
of two runs, one without the action and one with it.

Usage: coverdiff.py BASELINE ACTION [--module NAME] [--base ADDR] [--functions functions.tsv]
                    [--both] [--json]

  BASELINE, ACTION   two coverage logs of the original: DynamoRIO's drcov format (drrun -t
                     drcov, also written by Frida and other tracers for Lighthouse), or text
                     files with one address per line
  --module NAME      only the blocks of the module whose path contains NAME (the game's .exe)
  --base ADDR        add this to the module offsets in a drcov log: the image base Ghidra uses
                     (0x400000, 0x140000000), so the addresses match the export
  --functions FILE   a Ghidra export's functions.tsv: name the function each block is in
  --both             also list the code that ran only in the baseline

A game runs most of its code every frame. The code that handles one action, such as buying a
unit, opening a door or saving, runs only when you do it. Record coverage while playing
without the action, then again doing the same and the action once. The blocks in the second
log and not in the first are the action's code, usually a handful of functions out of
thousands. Start tracing there.

Keep the two runs as alike as you can: same save, same menus, same length. The fewer
differences, the shorter the list. A drcov log lists each basic block once, however often it
ran. Standard library only.
"""
import bisect
import json
import re
import struct
import sys


def read_log(path, module, base):
    """The set of block addresses (and their sizes) in a coverage log."""
    with open(path, 'rb') as f:
        data = f.read()
    if not data.startswith(b'DRCOV VERSION'):
        out = {}
        for line in data.decode('latin-1').split():
            line = line.strip().rstrip(',')
            if re.fullmatch(r'(0x)?[0-9a-fA-F]+', line):
                out[int(line, 16)] = 1
        return out
    head_end = data.index(b'BB Table:')
    header = data[:head_end].decode('latin-1').splitlines()
    columns, mods = None, {}
    for line in header:
        if line.startswith('Columns:'):
            columns = [c.strip() for c in line[len('Columns:'):].split(',')]
        elif columns and re.match(r'\s*\d+,', line):
            parts = [p.strip() for p in line.split(',', len(columns) - 1)]
            row = dict(zip(columns, parts))
            mods[int(row['id'])] = row.get('path', '')
    line_end = data.index(b'\n', head_end)
    count = int(re.search(rb'(\d+)', data[head_end:line_end]).group(1))
    at = line_end + 1
    out = {}
    for i in range(count):
        start, size, mod = struct.unpack_from('<IHH', data, at + 8 * i)
        if module and module.lower() not in mods.get(mod, '').lower():
            continue
        out[base + start] = size
    return out


def functions(path):
    rows = []
    for line in open(path, encoding='utf-8', errors='replace'):
        parts = line.rstrip('\n').split('\t')
        if len(parts) >= 3 and ':' not in parts[0]:
            rows.append((int(parts[0], 16), parts[2]))
    rows.sort()
    return rows


def group(blocks, funcs):
    starts = [a for a, _ in funcs]
    by = {}
    for addr in sorted(blocks):
        i = bisect.bisect_right(starts, addr) - 1
        key = (funcs[i][0], funcs[i][1]) if i >= 0 and funcs else (addr, '')
        by.setdefault(key, []).append(addr)
    return by


def main(argv):
    if len(argv) < 2 or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0 if argv and argv[0] in ('-h', '--help') else 2
    opt = lambda name, default=None: argv[argv.index(name) + 1] if name in argv else default
    base = int(opt('--base', '0'), 0)
    a = read_log(argv[0], opt('--module'), base)
    b = read_log(argv[1], opt('--module'), base)
    funcs = functions(opt('--functions')) if opt('--functions') else []
    only_b = set(b) - set(a)
    only_a = set(a) - set(b)
    report = {'baseline_blocks': len(a), 'action_blocks': len(b),
              'only_action': [{'function': f'{fa:x}', 'name': n, 'blocks': [f'{x:x}' for x in bl]}
                              for (fa, n), bl in group(only_b, funcs).items()]}
    if '--both' in argv:
        report['only_baseline'] = [{'function': f'{fa:x}', 'name': n, 'blocks': [f'{x:x}' for x in bl]}
                                   for (fa, n), bl in group(only_a, funcs).items()]
    if '--json' in argv:
        print(json.dumps(report, indent=2))
        return 0
    print(f'{len(a)} blocks in the baseline, {len(b)} with the action; '
          f'{len(only_b)} blocks ran only with the action')
    for item in report['only_action']:
        name = f'  {item["name"]}' if item['name'] else ''
        print(f'  {item["function"]}{name}  ({len(item["blocks"])} blocks)')
    for item in report.get('only_baseline', []):
        print(f'  baseline only: {item["function"]} {item["name"]} ({len(item["blocks"])} blocks)')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
