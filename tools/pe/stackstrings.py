#!/usr/bin/env python3
"""Find text that the code builds on the stack a few bytes at a time, which a list of strings
in the file does not show.

Usage: stackstrings.py BINARY [--min N] [--json]
       stackstrings.py RAW --raw --bits 16|32|64 --base ADDR [--min N]

A compiler that initialises a local `char name[] = "SAVE0.DAT"` often writes it with a row of
`mov` instructions holding the bytes as immediate values: `mov dword [ebp-0x10], 0x45564153`.
The text then appears in no string list and in no Ghidra string table. Some protections and
some games also build file names, keys and messages this way on purpose. This tool
disassembles the executable sections (capstone) and joins the immediate values that are
stored to neighbouring stack offsets into text, ordered by offset.

Each result gives the address of the first store, the stack offset and the text. Open the
function at that address: the text is a local variable, and the code that uses it is close by.
`--min` sets the shortest text reported (default 4). Windows PE files are read with their
sections. Any other code (a DOS program's load image, a memory dump) can be given with --raw,
its CPU mode and its base address. Needs capstone (.venv).
"""
import json
import struct
import sys

import capstone
from capstone import x86

PRINTABLE = set(range(32, 127)) | {9, 10, 13}


def code_regions(path, raw, bits, base):
    with open(path, 'rb') as f:
        data = f.read()
    if raw:
        return [(data, base, bits)]
    if data[:2] != b'MZ':
        raise SystemExit('not a PE file: use --raw --bits N --base ADDR')
    pe = struct.unpack_from('<I', data, 0x3C)[0]
    machine, count = struct.unpack_from('<HH', data, pe + 4)
    opt_size = struct.unpack_from('<H', data, pe + 20)[0]
    opt = pe + 24
    is64 = struct.unpack_from('<H', data, opt)[0] == 0x20B
    image = struct.unpack_from('<Q', data, opt + 24)[0] if is64 else struct.unpack_from('<I', data, opt + 28)[0]
    out = []
    for i in range(count):
        s = opt + opt_size + 40 * i
        va, rsize, raw_at = struct.unpack_from('<III', data, s + 12)
        flags = struct.unpack_from('<I', data, s + 36)[0]
        if flags & 0x20000000:          # IMAGE_SCN_MEM_EXECUTE
            out.append((data[raw_at:raw_at + rsize], image + va, 64 if is64 else 32))
    return out


def stores(code, base, bits):
    """Yield (address, base register, displacement, bytes) for each store of an immediate to
    memory addressed from a frame or stack register."""
    mode = {16: capstone.CS_MODE_16, 32: capstone.CS_MODE_32, 64: capstone.CS_MODE_64}[bits]
    md = capstone.Cs(capstone.CS_ARCH_X86, mode)
    md.detail = True
    md.skipdata = True
    frame_regs = {x86.X86_REG_EBP, x86.X86_REG_ESP, x86.X86_REG_RBP, x86.X86_REG_RSP, x86.X86_REG_BP,
                  x86.X86_REG_SP}
    for ins in md.disasm(code, base):
        if ins.id != x86.X86_INS_MOV or len(ins.operands) != 2:
            yield None
            continue
        dst, src = ins.operands
        if dst.type != x86.X86_OP_MEM or src.type != x86.X86_OP_IMM or dst.mem.base not in frame_regs \
                or dst.mem.index != 0:
            yield None
            continue
        size = dst.size
        value = src.imm & ((1 << (8 * size)) - 1)
        yield ins.address, dst.mem.base, dst.mem.disp, value.to_bytes(size, 'little')


def texts(regions, minimum):
    out = []
    for code, base, bits in regions:
        # A run is a sequence of stores with at most three other instructions between them. Each store
        # writes its bytes into a map of stack offsets, so overlapping stores (a compiler may
        # write the last four bytes of a string with an overlapping store) give the right text.
        run, first = [], None

        def flush():
            for reg in {r for r, _, _ in run}:
                mem = {}
                for r, disp, data in run:
                    if r == reg:
                        for k, b in enumerate(data):
                            mem[disp + k] = b
                offsets = sorted(mem)
                start = prev = None
                for o in offsets + [None]:
                    if o is not None and prev is not None and o == prev + 1:
                        prev = o
                        continue
                    if start is not None:
                        text = bytes(mem[i] for i in range(start, prev + 1)).split(b'\0')[0]
                        if len(text) >= minimum and all(c in PRINTABLE for c in text):
                            out.append({'address': first, 'offset': start, 'text': text.decode('latin-1')})
                    start = prev = o

        gap = 0
        for s in stores(code, base, bits):
            if s is None:
                gap += 1
                if gap > 3:                # a few other instructions may sit between stores
                    if len(run) >= 2:
                        flush()
                    run, first = [], None
                continue
            gap = 0
            addr, reg, disp, data = s
            first = first or addr
            run.append((reg, disp, data))
        if len(run) >= 2:
            flush()
    return out


def main(argv):
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0 if argv else 2
    opt = lambda name, default=None: argv[argv.index(name) + 1] if name in argv else default
    regions = code_regions(argv[0], '--raw' in argv, int(opt('--bits', '32')), int(opt('--base', '0'), 0))
    rows = texts(regions, int(opt('--min', '4')))
    if '--json' in argv:
        print(json.dumps(rows, indent=2))
        return 0
    for r in rows:
        print(f'0x{r["address"]:x}  stack {r["offset"]:+#x}  {r["text"]!r}')
    if not rows:
        print('no stack strings found')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
