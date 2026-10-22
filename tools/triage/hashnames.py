#!/usr/bin/env python3
"""Name hashed strings: find which hash function a game uses for its names, and which name each
hash constant stands for.

Usage: hashnames.py PATH [--names FILE] [--names-from BINARY ...] [--aligned] [--value 0xHEX ...] [--all]
       hashnames.py --hash STRING          every hash of one string, to compare by hand

Many engines keep names as 32-bit hashes: class ids, resource names in an archive, event
types, animation channels, settings. The binary then holds the hash where the name would be,
as an immediate in the code (`cmp eax, 0x8f3a12c4`) or in a table. If the names are still in
the program somewhere (debug messages, a registration call, a file of names), hashing every
string with every common function finds the matches. The function with far more matches than
chance is the game's hash function, and each match names one constant.

PATH is a binary, a data file, or a folder whose files are all searched. Some games compute
their hashes when they start, so the hashes are only in the data files: search the data folder,
and take the names from the program with --names-from. The candidates are the printable strings
in PATH and in each --names-from BINARY (ASCII and UTF-16), and the lines of --names FILE (a list
from a modding wiki, a file listing, another game on the engine). With --names-from, the
strings of PATH itself are left out: data files are full of text that is not a name. With
--aligned, only values at offsets divisible by 4 are read, as in most structured data. Each is
tried as it is, in lower case and in upper case. The functions are CRC-32 (and CRC-32 without
its final inversion), FNV-1 and FNV-1a (32-bit), djb2, djb2 with xor, sdbm, the Java string
hash (x31), the ELF hash and ROR-13 (used to find imports by hash). Strings are also hashed as UTF-16LE for CRC-32 and FNV-1a.

The first table counts how often each function's hashes occur, and compares the count with
chance. A real hash often occurs many times (a type id in the header of every resource). Only a
function well above chance is listed in full (--all lists every function). With --value, only
the given constants are searched for, which needs no luck: each is named or reported unknown.
A hash not found here may use a seed, a different case rule (only the file name, slashes
turned round) or a function not listed; constants.py names the function from its constants in
the code. Standard library only.
"""
import os
import re
import struct
import sys
import zlib
from array import array

M32 = 0xFFFFFFFF


def crc32(b):
    return zlib.crc32(b) & M32


def crc32_noinv(b):
    return (~zlib.crc32(b)) & M32


def fnv1(b):
    h = 0x811C9DC5
    for c in b:
        h = ((h * 0x01000193) & M32) ^ c
    return h


def fnv1a(b):
    h = 0x811C9DC5
    for c in b:
        h = ((h ^ c) * 0x01000193) & M32
    return h


def djb2(b):
    h = 5381
    for c in b:
        h = (h * 33 + c) & M32
    return h


def djb2x(b):
    h = 5381
    for c in b:
        h = ((h * 33) ^ c) & M32
    return h


def sdbm(b):
    h = 0
    for c in b:
        h = (c + (h << 6) + (h << 16) - h) & M32
    return h


def java(b):
    h = 0
    for c in b:
        h = (h * 31 + c) & M32
    return h


def elf(b):
    h = 0
    for c in b:
        h = ((h << 4) + c) & M32
        g = h & 0xF0000000
        if g:
            h ^= g >> 24
        h &= ~g & M32
    return h


def ror13(b):
    """Rotate right by 13, then add each byte: the usual hash for import names found at run time."""
    h = 0
    for c in b:
        h = (((h >> 13) | (h << 19)) + c) & M32
    return h


FUNCTIONS = [('crc32', crc32, False), ('crc32-noinv', crc32_noinv, False), ('fnv1-32', fnv1, False),
             ('fnv1a-32', fnv1a, False), ('djb2', djb2, False), ('djb2-xor', djb2x, False), ('sdbm', sdbm, False),
             ('java-x31', java, False), ('elf', elf, False), ('ror13', ror13, False), ('crc32-utf16', crc32, True), ('fnv1a-utf16', fnv1a, True)]
CASES = [('as is', lambda t: t), ('lower', str.lower), ('upper', str.upper)]


def strings(data, minimum=3):
    found = set()
    for m in re.finditer(rb'[\x20-\x7e]{%d,}' % minimum, data):
        found.add(m.group().decode('ascii'))
    for m in re.finditer(rb'(?:[\x20-\x7e]\x00){%d,}' % minimum, data):
        found.add(m.group().decode('utf-16-le'))
    return found


def table(names, keep_small=False):
    """{function name: {hash: (case, name)}} for every candidate name."""
    out = {}
    for fname, fn, wide in FUNCTIONS:
        d = {}
        for case, conv in CASES:
            for name in names:
                t = conv(name)
                h = fn(t.encode('utf-16-le') if wide else t.encode('latin-1', 'replace'))
                # Short names give small, far from random values under the polynomial hashes,
                # and small numbers are common in any file: leave both out.
                if (keep_small or (h >= 0x01000000 and len(t) >= 4)) and h not in d:
                    d[h] = (case, t)
        out[fname] = d
    return out


def values_in(data, aligned=False):
    """Every 32-bit little-endian value at every byte offset, as four arrays (one per alignment),
    or at offsets divisible by 4 only."""
    out = []
    for shift in (0,) if aligned else range(4):
        part = data[shift:]
        part = part[:len(part) // 4 * 4]
        a = array('I')
        a.frombytes(part)
        if sys.byteorder == 'big':
            a.byteswap()
        out.append(a)
    return out


def offsets(data, value, limit=4):
    pat = struct.pack('<I', value)
    found, i = [], data.find(pat)
    while i >= 0 and len(found) < limit:
        found.append(i)
        i = data.find(pat, i + 1)
    return found


def main(argv):
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0
    if argv[0] == '--hash':
        text = argv[1]
        for fname, fn, wide in FUNCTIONS:
            for case, conv in CASES:
                t = conv(text)
                h = fn(t.encode('utf-16-le') if wide else t.encode('latin-1', 'replace'))
                print(f'{fname:12} {case:6} 0x{h:08x}  {t}')
        return 0
    files = []
    if os.path.isdir(argv[0]):
        for root, _, fnames in os.walk(argv[0]):
            for n in sorted(fnames):
                with open(os.path.join(root, n), 'rb') as f:
                    files.append((os.path.relpath(os.path.join(root, n), argv[0]), f.read()))
    else:
        with open(argv[0], 'rb') as f:
            files.append((os.path.basename(argv[0]), f.read()))
    names = set()
    if '--names-from' not in argv:
        for _, data in files:
            names |= strings(data)
    for i, a in enumerate(argv):
        if a == '--names-from':
            with open(argv[i + 1], 'rb') as f:
                names |= strings(f.read())
    if '--names' in argv:
        with open(argv[argv.index('--names') + 1], encoding='utf-8', errors='replace') as f:
            names |= {line.strip() for line in f if line.strip()}
    wanted = [int(argv[i + 1], 16) for i, a in enumerate(argv) if a == '--value']
    print(f'{len(names)} candidate names, {len(FUNCTIONS)} hash functions, 3 cases each')
    hashes = table(names, keep_small=bool(wanted))

    if wanted:
        for v in wanted:
            hits = [(fname, d[v]) for fname, d in hashes.items() if v in d]
            if hits:
                for fname, (case, name) in hits:
                    print(f'0x{v:08x}  {fname} ({case}): {name}')
            else:
                print(f'0x{v:08x}  unknown: no candidate name hashes to it')
        return 0

    every = set()
    for d in hashes.values():
        every |= d.keys()
    scanned, count = 0, {}
    for _, data in files:
        for a in values_in(data, '--aligned' in argv):
            scanned += len(a)
            for v in a:
                if v in every:
                    count[v] = count.get(v, 0) + 1
    results = []
    for fname, d in hashes.items():
        seen = {v for v in count if v in d}
        n = sum(count[v] for v in seen)
        chance = scanned * len(d) / 2 ** 32
        results.append((n, fname, chance, seen))
    results.sort(key=lambda r: -r[0])
    print(f'\n{"function":14} {"matches":>8} {"by chance":>10} {"names":>6}')
    for n, fname, chance, seen in results:
        print(f'{fname:14} {n:8} {chance:10.1f} {len(seen):6}')
    shown = 0
    for n, fname, chance, seen in results:
        if not ('--all' in argv or n >= 3 * chance + 5):
            continue
        shown += 1
        print(f'\n{fname}: {n} matches of {len(seen)} names (chance {chance:.1f})')
        for v in sorted(seen, key=lambda v: (-count[v], hashes[fname][v][1].lower())):
            case, name = hashes[fname][v]
            where = next(f'{path} +0x{offsets(data, v, 1)[0]:x}' for path, data in files if offsets(data, v, 1))
            print(f'  0x{v:08x}  {name!r:40} {case:6} {count[v]:6}x  first at {where}')
    if not shown:
        print('\nNo function is clearly above chance. Try --names with a list of likely names, or --value '
              'with constants from the lookup code.')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
