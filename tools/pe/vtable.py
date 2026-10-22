#!/usr/bin/env python3
"""List the slots of a table of function pointers (a C++ vtable) with the functions they hold.

Usage: vtable.py BINARY TABLE_ADDRESS [SLOTS] [--names functions.tsv]

Reads pointers from TABLE_ADDRESS until one does not point into an executable section (or
SLOTS are read), and prints each slot's offset, target and name. Names come from a Ghidra
functions.tsv (scripts/ghidra-export.sh writes one) and from labels/names.tsv when present.
A class's vtable is found in its constructor: the first store into a new object is usually
`*this = &vtable`. Slot offsets are what call sites use: `(*(code **)(*obj + 0x18))(obj)` calls
slot +0x18.
"""
import os
import struct
import sys

import pefile


def load_names(path):
    names = {}
    if path and os.path.exists(path):
        for line in open(path, encoding='utf-8', errors='replace'):
            cols = line.rstrip('\n').split('\t')
            if len(cols) >= 3 and not line.startswith('#'):
                try:
                    names[int(cols[0], 16)] = cols[2] if path.endswith('functions.tsv') else cols[1]
                except ValueError:
                    pass
    return names


def main(argv):
    names_path = None
    if '--names' in argv:
        i = argv.index('--names')
        names_path = argv[i + 1]
        argv = argv[:i] + argv[i + 2:]
    path, table, *rest = argv
    slots = int(rest[0]) if rest else 256
    pe = pefile.PE(path, fast_load=True)
    base = pe.OPTIONAL_HEADER.ImageBase
    wide = pe.FILE_HEADER.Machine == 0x8664
    size, fmt = (8, '<Q') if wide else (4, '<I')
    names = load_names(names_path)
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    names.update(load_names(os.path.join(root, 'labels', 'names.tsv')))
    execs = [(base + s.VirtualAddress, base + s.VirtualAddress + s.Misc_VirtualSize)
             for s in pe.sections if s.Characteristics & 0x20000000]
    va = int(table.split('_')[-1], 16)
    for i in range(slots):
        target = struct.unpack(fmt, pe.get_data(va - base + size * i, size))[0]
        if not any(lo <= target < hi for lo, hi in execs):
            break
        print(f'+{size * i:#05x}  {target:#x}  {names.get(target, "")}')


if __name__ == '__main__':
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(2)
    main(sys.argv[1:])
