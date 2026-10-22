#!/usr/bin/env python3
"""Read typed values from a DOS (MZ) or 16-bit NE executable at Ghidra's seg:off addresses.

Usage: readseg.py BINARY TYPE ADDRESS [ADDRESS ...] [--count N]

The 16-bit counterpart of tools/pe/readdata.py, for the DAT_1008_0004 globals in pseudocode and
for Pascal "typed constants" (initialised variables in the data segment). TYPE is:

  u8 i8 u16 i16 u32 i32     integers (Pascal byte, shortint, word, integer, longint)
  ptr                       a far pointer, offset word then segment word, printed seg:off
  real48                    Turbo Pascal's 6-byte Real (not IEEE: exponent byte first)
  f32 f64 f80               single, double, extended (x87 80-bit)
  pstr                      a Pascal string: a length byte, then the text (string[N])
  cstr                      a zero-terminated string
  hex                       16 * count raw bytes

ADDRESS is as Ghidra prints it: 1008:0004, DAT_1008_0004 or FUN_1000_0028 (NE selectors are
0x1000 + 8 * (segment - 1); MZ images are loaded at segment 0x1000). `--count N` reads N
consecutive values (a table). Values are as stored in the file, before the program runs:
relocated words (selectors, far pointers to segments) still hold the file's placeholder.
It does not read memory the program fills at run time (use dos_emu or DOSBox-X for that).
"""
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ne import load, parse_address  # noqa: E402

FORMATS = {'u8': '<B', 'i8': '<b', 'u16': '<H', 'i16': '<h', 'u32': '<I', 'i32': '<i',
           'f32': '<f', 'f64': '<d'}


def real48(b):
    """Turbo Pascal Real: byte 0 exponent (bias 129, 0 = zero), bytes 1-5 mantissa, sign in bit 7 of byte 5."""
    if b[0] == 0:
        return 0.0
    mant = int.from_bytes(b[1:6], 'little') & ((1 << 39) - 1)
    sign = -1.0 if b[5] & 0x80 else 1.0
    return sign * (1 + mant / (1 << 39)) * 2.0 ** (b[0] - 129)


def f80(b):
    mant = int.from_bytes(b[:8], 'little')
    exp = int.from_bytes(b[8:10], 'little')
    sign = -1.0 if exp & 0x8000 else 1.0
    exp &= 0x7FFF
    if exp == 0 and mant == 0:
        return 0.0
    return sign * mant / (1 << 63) * 2.0 ** (exp - 16383)


def main(argv):
    count = 1
    if '--count' in argv:
        i = argv.index('--count')
        count = int(argv[i + 1])
        argv = argv[:i] + argv[i + 2:]
    path, kind, *addrs = argv
    exe = load(path)
    for a in addrs:
        seg, off = parse_address(a)
        at = exe.file_offset(seg, off)
        if at is None:
            print(f'{seg:04x}:{off:04x}  not initialised in the file (a variable set at run time, or an '
                  f'address outside every segment)')
            continue
        d = exe.data
        if kind in ('pstr', 'cstr'):
            values = []
            for _ in range(count):
                if kind == 'pstr':
                    text = d[at + 1:at + 1 + d[at]]
                    at += 1 + d[at]
                else:
                    text = d[at:d.index(b'\0', at)]
                    at += len(text) + 1
                values.append(repr(text.decode('latin-1')))
            print(f'{seg:04x}:{off:04x}  {", ".join(values)}')
            continue
        if kind == 'hex':
            print(f'{seg:04x}:{off:04x}  {d[at:at + 16 * count].hex(" ")}')
            continue
        size = {'ptr': 4, 'real48': 6, 'f80': 10}.get(kind) or struct.calcsize(FORMATS[kind])
        values = []
        for i in range(count):
            b = d[at + size * i:at + size * (i + 1)]
            if kind == 'ptr':
                o, s = struct.unpack('<HH', b)
                values.append(f'{s:04x}:{o:04x}')
            elif kind == 'real48':
                values.append(repr(real48(b)))
            elif kind == 'f80':
                values.append(repr(f80(b)))
            else:
                values.append(repr(struct.unpack(FORMATS[kind], b)[0]))
        print(f'{seg:04x}:{off:04x}  {", ".join(values)}')


if __name__ == '__main__':
    if len(sys.argv) < 4:
        print(__doc__)
        sys.exit(2)
    main(sys.argv[1:])
