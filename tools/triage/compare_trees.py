#!/usr/bin/env python3
"""Compare two copies of a game file by file: are two releases (a CD, a GOG or Steam copy, a
patch) the same program and the same data?

Usage: compare_trees.py DIR_A DIR_B [--all] [--hashes] [--similar]

Files are matched by path, ignoring case (DOS and Windows names are case-insensitive), and
compared by SHA-256. Prints the files that differ (with both sizes), the files only in one
copy, and a summary. `--all` also lists identical files; `--hashes` prints "sha256  path" for
every file of DIR_A (publishable: hashes are not game material, and they pin down which
version a finding was made on).

Use it before choosing which copy to reverse engineer: identical code means findings carry
over; a different executable means a different version, with its own addresses.
It does not look inside archives or installers (extract them first with
scripts/extract-installer.sh).

`--similar` also scores how much of each differing file is shared (0 to 100 %). It pairs each
file found only in A with the most similar file only in B, which finds renamed or moved
files. The score cuts each file into pieces at points chosen by its content (a rolling hash),
the method of fuzzy hashes such as ssdeep. A change in one place then changes only the pieces
around it, even when bytes were inserted. Files over 64 MB are not scored.
"""
import random
import hashlib
import os
import sys


def tree(root):
    out = {}
    for base, _, files in os.walk(root):
        for f in files:
            p = os.path.join(base, f)
            out[os.path.relpath(p, root).replace(os.sep, '/').upper()] = p
    return out


def sha(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


LIMIT = 64 << 20
_GEAR = random.Random(0x6761).getrandbits
GEAR = [_GEAR(32) for _ in range(256)]


def pieces(path, mask=(1 << 11) - 1):
    """The hashes of the file's content-defined pieces (about 2 KB each)."""
    data = open(path, 'rb').read()
    out, start, h = set(), 0, 0
    for i, byte in enumerate(data):
        h = ((h << 1) + GEAR[byte]) & 0xFFFFFFFF
        if h & mask == 0 and i - start >= 64:
            out.add(hashlib.blake2b(data[start:i + 1], digest_size=8).digest())
            start = i + 1
    if start < len(data):
        out.add(hashlib.blake2b(data[start:], digest_size=8).digest())
    return out


def similarity(p, q, cache):
    if max(os.path.getsize(p), os.path.getsize(q)) > LIMIT:
        return None
    for x in (p, q):
        if x not in cache:
            cache[x] = pieces(x)
    a, b = cache[p], cache[q]
    return 100 * len(a & b) // max(1, len(a | b))


def main(argv):
    if len(argv) < 2 or argv[0] in ('-h', '--help'):
        print(__doc__)
        sys.exit(2)
    a, b = tree(argv[0]), tree(argv[1])
    if '--hashes' in argv:
        for k in sorted(a):
            print(f'{sha(a[k])}  {os.path.relpath(a[k], argv[0])}')
        return
    same = diff = 0
    similar, cache = '--similar' in argv, {}
    for k in sorted(set(a) & set(b)):
        if os.path.getsize(a[k]) == os.path.getsize(b[k]) and sha(a[k]) == sha(b[k]):
            same += 1
            if '--all' in argv:
                print(f'same       {k}')
        else:
            diff += 1
            score = similarity(a[k], b[k], cache) if similar else None
            shared = f', {score} % shared' if score is not None else ''
            print(f'DIFFERENT  {k}  ({os.path.getsize(a[k]):,} vs {os.path.getsize(b[k]):,} bytes{shared})')
    only_a, only_b = sorted(set(a) - set(b)), sorted(set(b) - set(a))
    for k in only_a:
        match = ''
        if similar and only_b:
            scored = [(similarity(a[k], b[o], cache) or 0, o) for o in only_b]
            best, other = max(scored)
            if best >= 30:
                match = f'  (like {other} in B, {best} % shared)'
        print(f'only A     {k}{match}')
    for k in only_b:
        print(f'only B     {k}')
    print(f'{same} identical, {diff} different, {len(only_a)} only in A, {len(only_b)} only in B')


if __name__ == '__main__':
    main(sys.argv[1:])
