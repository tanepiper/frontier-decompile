#!/usr/bin/env python3
"""How alike two files are, when they are not identical: two versions of an executable, a
patched data file, a renamed file, or a library the game links against.

Usage: similar.py A B                  two files: a score from 0 to 100
       similar.py FILE DIR             every file in DIR that resembles FILE, best first
       similar.py DIR_A DIR_B          each file of DIR_A with its closest file in DIR_B
       options: --min N (default 20)  hide scores below N;  --json

compare_trees.py says whether two files are the same. This tool says how much they share.
It cuts each file into pieces where the content itself says so (a rolling hash over a few
bytes picks the cut points), and hashes each piece. An insertion or a change then moves or
changes only the pieces around it, and the rest still match. The score is the share of bytes
in matching pieces.

Uses:
- which release a copy is closest to, and how much changed between two patches;
- a file that was renamed or moved between versions (DIR_A DIR_B shows it);
- the parts of an executable that are a known library (compare with the library's own build;
  then confirm with the code, re-trace);
- which saves or levels differ only a little, to find the bytes that hold one setting.

The idea is context-triggered piecewise hashing, as in ssdeep. This is GameFrame's own
implementation, and its scores are not ssdeep's. Standard library only.
"""
import hashlib
import json
import os
import sys

WINDOW = 7


def pieces(data, block):
    """Content-defined pieces: cut where a rolling sum of the last WINDOW bytes hits block - 1
    modulo block. Returns {digest: bytes covered}."""
    out = {}
    start, h = 0, 0
    for i, b in enumerate(data):
        h = (h * 31 + b) & 0xFFFFFFFF
        if i >= WINDOW:
            h = (h - data[i - WINDOW] * pow31) & 0xFFFFFFFF
        if h % block == block - 1 or i == len(data) - 1:
            d = hashlib.blake2b(data[start:i + 1], digest_size=8).digest()
            out[d] = out.get(d, 0) + i + 1 - start
            start = i + 1
    return out


pow31 = pow(31, WINDOW, 1 << 32)
BLOCKS = (32, 128, 512, 2048, 8192)


class Print:
    def __init__(self, path):
        self.path = path
        with open(path, 'rb') as f:
            self.data = f.read(64 << 20)
        self.size = len(self.data)
        self.by_block = {}

    def at(self, block):
        if block not in self.by_block:
            self.by_block[block] = pieces(self.data, block)
        return self.by_block[block]


def block_for(size):
    """About 200 pieces in the smaller file: small enough to see a change, big enough to be fast."""
    for b in BLOCKS:
        if size // b <= 200:
            return b
    return BLOCKS[-1]


def score(a, b):
    if not a.size or not b.size:
        return 100 if a.size == b.size else 0
    if a.data == b.data:
        return 100
    block = block_for(min(a.size, b.size))
    pa, pb = a.at(block), b.at(block)
    shared = sum(min(n, pb[d]) for d, n in pa.items() if d in pb)
    return round(100 * shared / max(a.size, b.size))


def files(d):
    for base, _, names in os.walk(d):
        for n in sorted(names):
            yield os.path.join(base, n)


def main(argv):
    if len(argv) < 2 or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0 if argv and argv[0] in ('-h', '--help') else 2
    floor = int(argv[argv.index('--min') + 1]) if '--min' in argv else 20
    as_json = '--json' in argv
    a, b = [x for i, x in enumerate(argv) if not x.startswith('--') and (i == 0 or argv[i - 1] != '--min')][:2]
    rows = []
    if os.path.isfile(a) and os.path.isfile(b):
        rows.append({'a': a, 'b': b, 'score': score(Print(a), Print(b))})
        floor = 0
    elif os.path.isfile(a):
        pa = Print(a)
        for p in files(b):
            rows.append({'a': a, 'b': os.path.relpath(p, b), 'score': score(pa, Print(p))})
        rows.sort(key=lambda r: -r['score'])
    else:
        right = [Print(p) for p in files(b)]
        for p in files(a):
            pa = Print(p)
            best = max(right, key=lambda q: score(pa, q), default=None)
            s = score(pa, best) if best else 0
            ra, rb = os.path.relpath(p, a), os.path.relpath(best.path, b) if best else ''
            note = 'same' if s == 100 and ra.upper() == rb.upper() else \
                'renamed or moved' if s >= 90 and ra.upper() != rb.upper() else \
                'changed' if s else 'no match'
            rows.append({'a': ra, 'b': rb, 'score': s, 'note': note})
    rows = [r for r in rows if r['score'] >= floor or r.get('note') == 'no match']
    if as_json:
        print(json.dumps(rows, indent=2))
        return 0
    for r in rows:
        print(f'{r["score"]:3d}  {r["a"]}  ->  {r["b"]}' + (f'  [{r["note"]}]' if r.get('note') else ''))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
