#!/usr/bin/env python3
"""Compare two copies of a game file by file: are two releases (a CD, a GOG or Steam copy, a
patch) the same program and the same data?

Usage: compare_trees.py DIR_A DIR_B [--all] [--hashes]

Files are matched by path, ignoring case (DOS and Windows names are case-insensitive), and
compared by SHA-256. Prints the files that differ (with both sizes), the files only in one
copy, and a summary. `--all` also lists identical files; `--hashes` prints "sha256  path" for
every file of DIR_A (publishable: hashes are not game material, and they pin down which
version a finding was made on).

Use it before choosing which copy to reverse engineer: identical code means findings carry
over; a different executable means a different version, with its own addresses.
It does not look inside archives or installers (extract them first with
scripts/extract-installer.sh).
"""
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
    for k in sorted(set(a) & set(b)):
        if os.path.getsize(a[k]) == os.path.getsize(b[k]) and sha(a[k]) == sha(b[k]):
            same += 1
            if '--all' in argv:
                print(f'same       {k}')
        else:
            diff += 1
            print(f'DIFFERENT  {k}  ({os.path.getsize(a[k]):,} vs {os.path.getsize(b[k]):,} bytes)')
    only_a, only_b = sorted(set(a) - set(b)), sorted(set(b) - set(a))
    for k in only_a:
        print(f'only A     {k}')
    for k in only_b:
        print(f'only B     {k}')
    print(f'{same} identical, {diff} different, {len(only_a)} only in A, {len(only_b)} only in B')


if __name__ == '__main__':
    main(sys.argv[1:])
