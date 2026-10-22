#!/usr/bin/env python3
"""Turn a DOSBox-X file log into a per-file map of reads: which file, at which offset, how many
bytes, in what order. The read sizes of a file that a program reads field by field (Pascal
BlockRead, C fread of a struct member) are the file's record layout, measured.

Usage: fileio_trace.py DOSBOX_LOG [--file NAME] [--game DIR] [--raw]

  DOSBOX_LOG   the log written by scripts/dosbox-run.sh --fileio (analysis/dosbox/dosbox.log)
  --file NAME  only this file (case does not matter)
  --game DIR   the game folder, to resolve seeks from the end of a file (SEEK_END)
  --raw        every read on its own line, instead of grouping repeated patterns

Prints, for each file in the order it was first opened: the open count and mode, then the reads
as "offset +size". A run of identical patterns is shown once with its repeat count, for example
"0x0020: [2, 1, 4] x 120 (stride 7)", which reads as a table of 120 seven-byte records.
Evidence from this tool is *measured* (the original program ran); name the run's inputs.
It does not see memory-mapped or BIOS disk access, and the log has no timestamps beyond
DOSBox-X's own cycle counter.
"""
import os
import re
import sys

OPEN = re.compile(r'FILES:(?:Special )?file open command (\w+) file (.+?)\s*$')
READ = re.compile(r'FILES:Reading (\d+) bytes from (.+?)\s*$')
WRITE = re.compile(r'FILES:Writing (\d+) bytes to (.+?)\s*$')
SEEK = re.compile(r'FILES:Seeking to (-?\d+) bytes from position type \((\d)\) in (.+?)\s*$')


def key(name):
    return os.path.basename(name.replace('\\', '/')).upper()


def parse(path, game=None):
    files = {}      # NAME -> dict(opens, modes, events [(kind, offset, size)])
    pos = {}
    order = []
    for line in open(path, encoding='latin-1'):
        if m := OPEN.search(line):
            name = key(m.group(2))
            f = files.setdefault(name, dict(opens=0, modes=set(), events=[]))
            if name not in order:
                order.append(name)
            f['opens'] += 1
            f['modes'].add(m.group(1))
            pos[name] = 0
        elif m := READ.search(line):
            name = key(m.group(2))
            if name in files:
                n = int(m.group(1))
                files[name]['events'].append(('r', pos.get(name, 0), n))
                pos[name] = pos.get(name, 0) + n
        elif m := WRITE.search(line):
            name = key(m.group(2))
            if name in files:
                n = int(m.group(1))
                files[name]['events'].append(('w', pos.get(name, 0), n))
                pos[name] = pos.get(name, 0) + n
        elif m := SEEK.search(line):
            name = key(m.group(3))
            if name in files:
                off, how = int(m.group(1)), int(m.group(2))
                if how == 0:
                    pos[name] = off
                elif how == 1:
                    pos[name] = pos.get(name, 0) + off
                else:
                    size = file_size(game, name)
                    pos[name] = (size + off) if size is not None else None
    return order, files


def file_size(game, name):
    if not game:
        return None
    for f in os.listdir(game):
        if f.upper() == name:
            return os.path.getsize(os.path.join(game, f))
    return None


def grouped(events):
    """Collapse repeating read patterns: [(start, pattern sizes, repeats)]."""
    out, i = [], 0
    sizes = [(k, n) for k, _, n in events]
    while i < len(events):
        best = (1, 1)   # (pattern length, repeats)
        for plen in range(1, 13):
            pat = sizes[i:i + plen]
            if len(pat) < plen:
                break
            reps = 1
            while sizes[i + reps * plen:i + (reps + 1) * plen] == pat and _contiguous(events, i, plen, reps + 1):
                reps += 1
            if reps > 1 and reps * plen > best[0] * best[1]:
                best = (plen, reps)
        plen, reps = best
        out.append((events[i][1], sizes[i:i + plen], reps))
        i += plen * reps
    return out


def _contiguous(events, i, plen, reps):
    end = i + plen * reps
    for j in range(i + 1, min(end, len(events))):
        prev = events[j - 1]
        if prev[1] is None or events[j][1] != prev[1] + prev[2]:
            return False
    return end <= len(events)


def main(argv):
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__)
        sys.exit(2)
    log, rest = argv[0], argv[1:]
    only = rest[rest.index('--file') + 1].upper() if '--file' in rest else None
    game = rest[rest.index('--game') + 1] if '--game' in rest else None
    order, files = parse(log, game)
    for name in order:
        if only and name != only:
            continue
        f = files[name]
        reads = [e for e in f['events'] if e[0] == 'r']
        writes = [e for e in f['events'] if e[0] == 'w']
        print(f'{name}: opened {f["opens"]}x (mode {",".join(sorted(f["modes"]))}), '
              f'{len(reads)} reads ({sum(e[2] for e in reads):,} bytes), {len(writes)} writes')
        if '--raw' in rest:
            for kind, off, n in f['events']:
                print(f'    {"read " if kind == "r" else "write"} {off:#08x} +{n}' if off is not None
                      else f'    {kind} ? +{n}')
            continue
        for start, pat, reps in grouped(f['events']):
            where = f'{start:#08x}' if start is not None else '       ?'
            shown = [n if k == 'r' else f'w{n}' for k, n in pat]
            stride = sum(n for _, n in pat)
            text = f'    {where}: {shown if len(shown) > 1 else shown[0]}'
            if reps > 1:
                text += f' x {reps} (stride {stride})'
            print(text)


if __name__ == '__main__':
    main(sys.argv[1:])
