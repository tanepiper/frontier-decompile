#!/usr/bin/env python3
"""Name the algorithms in a binary by their constants: random number generators, checksums,
hashes, ciphers, compressors and well-known maths tricks.

Usage: constants.py FILE [FILE ...] [--json] [--all]

Many algorithms carry numbers that appear nowhere else: the multiplier of a random number
generator, the first entries of a CRC table, the start values of a hash. Finding one in a
game's executable says which algorithm the game uses, and the address leads to the function.
That is often the fastest way to find the random number generator (re-trace, the randomiser
agent) or the decompressor of a data file (re-format).

Each 32-bit constant is searched little-endian and big-endian. For 16-bit code, where a
32-bit value is loaded as two 16-bit halves, the halves are also searched close together
(marked "split"). Tables are matched by their first few entries. A hit is a lead: open the
function at that address and confirm the algorithm before you port it. Short or common values
(such as 5381 or 65521) are left out unless --all is given. For a Windows executable the
virtual address is printed next to the file offset. Standard library only.
"""
import json
import struct
import sys

# name, family, value, bits. Values are the published constants of each algorithm.
SCALARS = [
    ('MSVC rand() multiplier 214013', 'random', 214013, 32),
    ('MSVC rand() increment 2531011', 'random', 2531011, 32),
    ('ANSI C / glibc rand() multiplier 1103515245', 'random', 1103515245, 32),
    ('Borland C rand() multiplier 22695477', 'random', 22695477, 32),
    ('Turbo Pascal / Delphi Random multiplier 134775813 (0x08088405)', 'random', 134775813, 32),
    ('Numerical Recipes LCG multiplier 1664525', 'random', 1664525, 32),
    ('Numerical Recipes LCG increment 1013904223', 'random', 1013904223, 32),
    ('Super-Duper / VAX LCG multiplier 69069', 'random', 69069, 32),
    ('Park-Miller minimal standard 16807', 'random', 16807, 32),
    ('Park-Miller revised 48271', 'random', 48271, 32),
    ('Java Random multiplier 0x5DEECE66D (low word)', 'random', 0xDEECE66D, 32),
    ('Mersenne Twister matrix 0x9908B0DF', 'random', 0x9908B0DF, 32),
    ('Mersenne Twister tempering 0x9D2C5680', 'random', 0x9D2C5680, 32),
    ('Mersenne Twister tempering 0xEFC60000', 'random', 0xEFC60000, 32),
    ('Mersenne Twister seeding 1812433253', 'random', 1812433253, 32),
    ('Golden ratio 0x9E3779B9 (TEA, XTEA, hash mixing)', 'cipher or hash', 0x9E3779B9, 32),
    ('xxHash prime 0x9E3779B1', 'hash', 0x9E3779B1, 32),
    ('xxHash prime 0x85EBCA77', 'hash', 0x85EBCA77, 32),
    ('MD5 / SHA-1 start value 0x67452301', 'hash', 0x67452301, 32),
    ('MD5 / SHA-1 start value 0xEFCDAB89', 'hash', 0xEFCDAB89, 32),
    ('MD5 first round constant 0xD76AA478', 'hash', 0xD76AA478, 32),
    ('SHA-1 start value 0xC3D2E1F0', 'hash', 0xC3D2E1F0, 32),
    ('SHA-1 round constant 0x5A827999', 'hash', 0x5A827999, 32),
    ('SHA-256 start value 0x6A09E667', 'hash', 0x6A09E667, 32),
    ('SHA-256 round constant 0x428A2F98', 'hash', 0x428A2F98, 32),
    ('FNV-1 32-bit offset basis 0x811C9DC5', 'hash', 0x811C9DC5, 32),
    ('FNV-1 32-bit prime 0x01000193', 'hash', 0x01000193, 32),
    ('MurmurHash2 multiplier 0x5BD1E995', 'hash', 0x5BD1E995, 32),
    ('MurmurHash3 constant 0xCC9E2D51', 'hash', 0xCC9E2D51, 32),
    ('MurmurHash3 constant 0x1B873593', 'hash', 0x1B873593, 32),
    ('CRC-32 reversed polynomial 0xEDB88320', 'checksum', 0xEDB88320, 32),
    ('CRC-32 polynomial 0x04C11DB7', 'checksum', 0x04C11DB7, 32),
    ('CRC-32C reversed polynomial 0x82F63B78', 'checksum', 0x82F63B78, 32),
    ('Blowfish P-array start 0x243F6A88 (also the digits of pi)', 'cipher', 0x243F6A88, 32),
    ('Fast inverse square root 0x5F3759DF', 'maths', 0x5F3759DF, 32),
    ('LZSS (Okumura) window start 4078 = N - F', 'compression', 0xFEE, 16),
    ('Adler-32 modulus 65521', 'checksum', 65521, 32),
    ('djb2 hash start 5381', 'hash', 5381, 32),
]
COMMON = {'Adler-32 modulus 65521', 'djb2 hash start 5381', 'Park-Miller minimal standard 16807',
          'Super-Duper / VAX LCG multiplier 69069', 'LZSS (Okumura) window start 4078 = N - F'}
SCALARS64 = [
    ('FNV-1 64-bit offset basis', 'hash', 0xCBF29CE484222325),
    ('FNV-1 64-bit prime', 'hash', 0x100000001B3),
    ('Java Random multiplier 0x5DEECE66D', 'random', 0x5DEECE66D),
    ('SplitMix64 / golden ratio 0x9E3779B97F4A7C15', 'random', 0x9E3779B97F4A7C15),
    ('xorshift* multiplier 0x2545F4914F6CDD1D', 'random', 0x2545F4914F6CDD1D),
    ('PCG multiplier 6364136223846793005', 'random', 6364136223846793005),
]
TABLES = [
    ('CRC-32 table (reflected, 0xEDB88320)', 'checksum', '<I', [0x00000000, 0x77073096, 0xEE0E612C, 0x990951BA]),
    ('CRC-32 table (normal, 0x04C11DB7)', 'checksum', '<I', [0x00000000, 0x04C11DB7, 0x09823B6E, 0x0D4326D9]),
    ('CRC-16-CCITT table (0x1021)', 'checksum', '<H', [0x0000, 0x1021, 0x2042, 0x3063, 0x4084]),
    ('CRC-16 table (reflected 0xA001, ARC/IBM)', 'checksum', '<H', [0x0000, 0xC0C1, 0xC181, 0x0140, 0xC301]),
    ('AES S-box', 'cipher', 'B', [0x63, 0x7C, 0x77, 0x7B, 0xF2, 0x6B, 0x6F, 0xC5]),
    ('AES inverse S-box', 'cipher', 'B', [0x52, 0x09, 0x6A, 0xD5, 0x30, 0x36, 0xA5, 0x38]),
    ('Deflate length base table (zlib, inflate)', 'compression', '<H', [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17]),
    ('Deflate distance base table (zlib, inflate)', 'compression', '<H', [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49]),
    ('Deflate code length order', 'compression', 'B', [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15]),
    ('Base64 alphabet', 'encoding', 'text', b'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'),
    ('Sine table, 16.16 fixed point, from 0', 'maths', '<i', [0, 1608, 3215, 4821]),
]
STRINGS = [b'inflate ', b'deflate ', b'Jean-loup Gailly', b'Mark Adler', b'LZO', b'UPX!', b'PKWARE',
           b'Mersenne', b'MT19937', b'zlib', b'libpng', b'Ogg Vorbis', b'Bink', b'Smacker', b'Miles Sound',
           b'Copyright (c) 1995-2', b'LZMA', b'lzma', b'bzip2', b'Lua 5.', b'$LuaVersion', b'Python ']


def find_all(data, needle):
    at = data.find(needle)
    while at >= 0:
        yield at
        at = data.find(needle, at + 1)


def pe_mapper(data):
    """File offset -> virtual address for a PE file, or None."""
    if data[:2] != b'MZ' or len(data) < 0x40:
        return None
    pe = struct.unpack_from('<I', data, 0x3C)[0]
    if data[pe:pe + 4] != b'PE\0\0':
        return None
    count = struct.unpack_from('<H', data, pe + 6)[0]
    opt_size = struct.unpack_from('<H', data, pe + 20)[0]
    opt = pe + 24
    magic = struct.unpack_from('<H', data, opt)[0]
    base = struct.unpack_from('<Q', data, opt + 24)[0] if magic == 0x20B else struct.unpack_from('<I', data, opt + 28)[0]
    sections = []
    for i in range(count):
        s = opt + opt_size + 40 * i
        name = data[s:s + 8].rstrip(b'\0').decode('latin-1')
        va, rsize, raw = struct.unpack_from('<III', data, s + 12)
        sections.append((raw, rsize, va, name))

    def to_va(off):
        for raw, rsize, va, name in sections:
            if raw <= off < raw + rsize:
                return base + va + off - raw, name
        return None, None
    return to_va


def scan(path, show_all):
    with open(path, 'rb') as f:
        data = f.read()
    hits = []
    for name, family, value, bits in SCALARS:
        if name in COMMON and not show_all:
            continue
        size = bits // 8
        forms = [('le', value.to_bytes(size, 'little')), ('be', value.to_bytes(size, 'big'))]
        for form, needle in forms:
            for at in find_all(data, needle):
                hits.append((at, name, family, form))
        if bits == 32 and value > 0xFFFF:
            lo, hi = (value & 0xFFFF).to_bytes(2, 'little'), (value >> 16).to_bytes(2, 'little')
            for at in find_all(data, lo):
                window = data[max(0, at - 12):at + 14]
                if hi in window and value.to_bytes(4, 'little') not in window:
                    hits.append((at, name, family, 'split'))
    for name, family, value in SCALARS64:
        for at in find_all(data, value.to_bytes(8, 'little')):
            hits.append((at, name, family, 'le'))
    for name, family, fmt, values in TABLES:
        needle = values if fmt == 'text' else b''.join(struct.pack(fmt, v) for v in values)
        for at in find_all(data, needle):
            hits.append((at, name, family, 'table'))
    for s in STRINGS:
        for at in list(find_all(data, s))[:3]:
            end = data.find(b'\0', at, at + 80)
            text = data[at:end if end > 0 else at + 60].decode('latin-1', 'replace')
            hits.append((at, f'text "{text}"', 'library name', 'string'))
    hits.sort()
    to_va = pe_mapper(data)
    rows = []
    for at, name, family, form in hits:
        va, section = to_va(at) if to_va else (None, None)
        rows.append({'file': path, 'offset': at, 'va': va, 'section': section,
                     'family': family, 'what': name, 'form': form})
    return rows


def main(argv):
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0 if argv else 2
    rows = []
    for p in [a for a in argv if not a.startswith('--')]:
        rows += scan(p, '--all' in argv)
    if '--json' in argv:
        print(json.dumps(rows, indent=2))
        return 0
    for r in rows:
        where = f'0x{r["va"]:x} {r["section"]}' if r['va'] is not None else ''
        print(f'{r["file"]}  0x{r["offset"]:08x}  {where:22s} {r["family"]:14s} {r["what"]}'
              + (f'  [{r["form"]}]' if r['form'] not in ('le', 'table', 'string') else ''))
    if not rows:
        print('no known constants found (try --all, or the unpacked code: see docs/media/protection.md)')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
