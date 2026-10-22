#!/usr/bin/env python3
"""Read typed values from a Windows binary at Ghidra addresses (the DAT_ globals in pseudocode).

Usage: readdata.py BINARY TYPE ADDRESS [ADDRESS ...] [--count N]

TYPE is u8, i8, u16, i16, u32, i32, u64, i64, f32, f64, ptr, cstr, wstr or hex. ADDRESS is a
virtual address at the binary's preferred base, as Ghidra prints it (0x140008000 or DAT_140008000).
`--count N` reads N consecutive values (a table). Ghidra often shows a float constant only as a
reference to data: read it here rather than trusting the decompiler's typing.
"""
import struct
import sys

import pefile

FORMATS = {'u8': '<B', 'i8': '<b', 'u16': '<H', 'i16': '<h', 'u32': '<I', 'i32': '<i',
           'u64': '<Q', 'i64': '<q', 'f32': '<f', 'f64': '<d'}


def parse_addr(text):
    return int(text.split('_')[-1], 16)


def main(argv):
    count = 1
    if '--count' in argv:
        i = argv.index('--count')
        count = int(argv[i + 1])
        argv = argv[:i] + argv[i + 2:]
    path, kind, *addrs = argv
    pe = pefile.PE(path, fast_load=True)
    base = pe.OPTIONAL_HEADER.ImageBase
    ptr = '<Q' if pe.FILE_HEADER.Machine == 0x8664 else '<I'
    for a in addrs:
        va = parse_addr(a)
        if kind in ('cstr', 'wstr'):
            raw = pe.get_data(va - base, 4096)
            text = raw.split(b'\0\0' if kind == 'wstr' else b'\0', 1)[0]
            print(f'{va:#x}  {text.decode("utf-16-le" if kind == "wstr" else "latin-1", errors="replace")!r}')
            continue
        if kind == 'hex':
            print(f'{va:#x}  {pe.get_data(va - base, 16 * count).hex(" ")}')
            continue
        fmt = ptr if kind == 'ptr' else FORMATS[kind]
        size = struct.calcsize(fmt)
        values = [struct.unpack(fmt, pe.get_data(va - base + size * i, size))[0] for i in range(count)]
        shown = [f'{v:#x}' if kind == 'ptr' else repr(v) for v in values]
        print(f'{va:#x}  {", ".join(shown)}')


if __name__ == '__main__':
    if len(sys.argv) < 4:
        print(__doc__)
        sys.exit(2)
    main(sys.argv[1:])
