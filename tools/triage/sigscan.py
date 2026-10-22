#!/usr/bin/env python3
"""Byte signatures with wildcards: make them from functions you have named, and find them in
another build, a memory dump or a game that uses the same library.

Usage: sigscan.py scan SIGS BINARY [--base ADDR] [--labels OUT.tsv]
       sigscan.py make BINARY ADDR[:LEN] [ADDR[:LEN] ...] [--names NAMES.tsv] [--len N] > SIGS
       sigscan.py make BINARY --names NAMES.tsv [--len N] > SIGS

  SIGS        a text file, one signature per line: "name: 55 8B EC ?? ?? 56 57" (?? is any
              byte; lines starting with # are comments). tools/dos/omf.py writes these from a
              compiler's libraries; `make` writes them from a binary.
  BINARY      a Windows PE (code sections, virtual addresses), a DOS MZ program (its load image;
              addresses as image offsets, and as Ghidra's seg:off from 1000:0000),
              or any other file (offsets, plus --base)
  --labels    write the matches as a labels file (address, name, evidence) for ghidra-run labels
  make        ADDR is a virtual address (PE), a seg:off or image offset (MZ), or a file offset;
              LEN the bytes to take (default --len, 32). --names takes the addresses and names
              from a labels file (labels/names.tsv), for every named function at once

A signature holds the first bytes of a function, with the bytes that change from one build to
the next replaced by ??: call and jump distances, absolute addresses of globals, and anything
the relocation table lists. What stays is the function's own instructions. So a signature
made from build A finds the same function in build B, without a Ghidra export of B
(match_functions.py does the same from two exports). A compiler's runtime library, turned
into signatures, names the runtime's functions inside a game, so they can be left unread.

The idea follows the library signatures of Cifuentes's dcc decompiler and the FLIRT
signatures of IDA. This is GameFrame's own, simple form. A match is a lead: two different
functions can begin with the same bytes, so the tool says when a signature matches more than
once. Needs capstone (.venv) for `make`.
"""
import os
import re
import struct
import sys


class Image:
    """The bytes to search and how to name an offset in them."""

    def __init__(self, path, base=0):
        with open(path, 'rb') as f:
            data = f.read()
        self.path, self.kind, self.relocs = path, 'raw', set()
        self.regions = [(data, base, 32)]      # (bytes, address of the first byte, bits)
        if data[:2] == b'MZ' and len(data) > 0x40:
            pe = struct.unpack_from('<I', data, 0x3C)[0]
            if data[pe:pe + 4] == b'PE\0\0':
                self._pe(data, pe)
                return
            self._mz(data)

    def _pe(self, data, pe):
        self.kind = 'pe'
        count = struct.unpack_from('<H', data, pe + 6)[0]
        opt_size = struct.unpack_from('<H', data, pe + 20)[0]
        opt = pe + 24
        is64 = struct.unpack_from('<H', data, opt)[0] == 0x20B
        image = struct.unpack_from('<Q', data, opt + 24)[0] if is64 else struct.unpack_from('<I', data, opt + 28)[0]
        self.image_base = image
        self.regions = []
        self.sections = []
        for i in range(count):
            s = opt + opt_size + 40 * i
            va, rsize, raw = struct.unpack_from('<III', data, s + 12)
            flags = struct.unpack_from('<I', data, s + 36)[0]
            self.sections.append((image + va, raw, rsize))
            if flags & 0x20000000:
                self.regions.append((data[raw:raw + rsize], image + va, 64 if is64 else 32))
        # The base relocation table: each entry is an address the loader patches.
        dirs = opt + (112 if is64 else 96)
        rva, size = struct.unpack_from('<II', data, dirs + 5 * 8)
        at = self._rva_to_off(image + rva) if rva else None
        end = at + size if at is not None else 0
        while at is not None and at + 8 <= end:
            page, block = struct.unpack_from('<II', data, at)
            if block < 8:
                break
            for k in range((block - 8) // 2):
                e = struct.unpack_from('<H', data, at + 8 + 2 * k)[0]
                kind, off = e >> 12, e & 0xFFF
                if kind in (3, 10):           # HIGHLOW (4 bytes), DIR64 (8 bytes)
                    for j in range(4 if kind == 3 else 8):
                        self.relocs.add(image + page + off + j)
            at += block

    def _rva_to_off(self, va):
        for start, raw, rsize in self.sections:
            if start <= va < start + rsize:
                return raw + va - start
        return None

    def _mz(self, data):
        self.kind = 'mz'
        header = struct.unpack_from('<H', data, 8)[0] * 16
        pages, last = struct.unpack_from('<HH', data, 4)[::-1][::-1]
        size = pages * 512 - (512 - last if last else 0)
        image = data[header:max(header, size)] or data[header:]
        count, table = struct.unpack_from('<H', data, 6)[0], struct.unpack_from('<H', data, 0x18)[0]
        for k in range(count):
            off, seg = struct.unpack_from('<HH', data, table + 4 * k)
            at = seg * 16 + off
            self.relocs.update((at, at + 1))
        self.regions = [(image, 0, 16)]

    def label_addr(self, addr):
        """Ghidra's address: an MZ image loads at segment 1000, so an offset in the first
        64 KB is 1000:offset. Further on, the segment is normalised (check it in Ghidra)."""
        if self.kind == 'mz':
            if addr < 0x10000:
                return f'1000:{addr:04x}'
            return f'{0x1000 + addr // 16:04x}:{addr % 16:04x}'
        return f'{addr:x}'

    def name(self, addr):
        if self.kind == 'mz':
            return f'{self.label_addr(addr)} (image offset 0x{addr:x})'
        return f'0x{addr:x}'

    def find_region(self, addr):
        for data, start, bits in self.regions:
            if start <= addr < start + len(data):
                return data, start, bits
        return None


def parse_sigs(path):
    sigs = []
    for line in open(path, encoding='utf-8'):
        line = line.strip()
        if not line or line.startswith('#') or ':' not in line:
            continue
        name, _, body = line.rpartition(':')
        pattern = [None if b == '??' else int(b, 16) for b in body.split()]
        sigs.append((name.strip(), pattern))
    return sigs


def anchor(pattern):
    """The longest run of fixed bytes, and where it starts in the pattern."""
    best, start, run = (0, 0), None, 0
    for i, b in enumerate(pattern + [None]):
        if b is None:
            if run > best[0]:
                best = (run, start)
            run, start = 0, None
        else:
            start = i if start is None else start
            run += 1
    return best[1], bytes(pattern[best[1]:best[1] + best[0]])


def scan(sigs, img):
    hits = []
    for name, pattern in sigs:
        at_in_pat, fixed = anchor(pattern)
        if len(fixed) < 3:
            continue
        found = []
        for data, start, _ in img.regions:
            i = data.find(fixed)
            while i >= 0:
                s = i - at_in_pat
                if s >= 0 and s + len(pattern) <= len(data) and all(
                        b is None or data[s + k] == b for k, b in enumerate(pattern)):
                    found.append(start + s)
                i = data.find(fixed, i + 1)
        if found:
            hits.append((name, found))
    return hits


def make(img, addr, length):
    import capstone
    from capstone import x86
    region = img.find_region(addr)
    if not region:
        raise SystemExit(f'{addr:#x} is not in the code of {img.path}')
    data, start, bits = region
    code = data[addr - start:addr - start + length]
    md = capstone.Cs(capstone.CS_ARCH_X86, {16: capstone.CS_MODE_16, 32: capstone.CS_MODE_32,
                                            64: capstone.CS_MODE_64}[bits])
    md.detail = True
    mask = [False] * len(code)
    low = min(s for _, s, _ in img.regions)
    high = max(s + len(d) for d, s, _ in img.regions) + 0x1000000
    for ins in md.disasm(code, addr):
        o = ins.address - addr
        branch = ins.group(x86.X86_GRP_JUMP) or ins.group(x86.X86_GRP_CALL)
        if ins.imm_size and (branch and len(ins.bytes) > 2 or
                             (ins.imm_size >= 4 and any(op.type == x86.X86_OP_IMM and low <= op.imm < high
                                                        for op in ins.operands))):
            for k in range(ins.imm_offset, ins.imm_offset + ins.imm_size):
                if o + k < len(mask):
                    mask[o + k] = True
        if ins.disp_size >= 4 or (bits == 16 and ins.disp_size == 2 and any(
                op.type == x86.X86_OP_MEM and op.mem.base == 0 for op in ins.operands)):
            for k in range(ins.disp_offset, ins.disp_offset + ins.disp_size):
                if o + k < len(mask):
                    mask[o + k] = True
        if ins.mnemonic in ('ret', 'retf', 'jmp') and not branch or ins.mnemonic in ('ret', 'retf'):
            code, mask = code[:o + len(ins.bytes)], mask[:o + len(ins.bytes)]
            break
    for k in range(len(code)):
        if addr + k in img.relocs:
            mask[k] = True
    return ' '.join('??' if m else f'{b:02X}' for b, m in zip(code, mask))


def parse_addr(text, img):
    if ':' in text and img.kind == 'mz':
        seg, off = (int(x, 16) for x in text.split(':'))
        return (seg - 0x1000) * 16 + off
    return int(text, 16) if not text.startswith('0x') and re.fullmatch(r'[0-9a-fA-F]{6,}', text) else int(text, 0)


def main(argv):
    if len(argv) < 3 or argv[0] not in ('scan', 'make'):
        print(__doc__)
        return 0 if argv and argv[0] in ('-h', '--help') else 2
    opt = lambda name, default=None: argv[argv.index(name) + 1] if name in argv else default
    taken = {i + 1 for i, a in enumerate(argv) if a in ('--base', '--labels', '--names', '--len')}
    args = [a for i, a in enumerate(argv[1:], 1) if not a.startswith('--') and i not in taken]
    if argv[0] == 'scan':
        img = Image(args[1], int(opt('--base', '0'), 0))
        hits = scan(parse_sigs(args[0]), img)
        for name, found in hits:
            where = ', '.join(img.name(a) for a in found[:4]) + (' ...' if len(found) > 4 else '')
            print(f'{name:32s} {where}' + (f'   [{len(found)} matches: check which]' if len(found) > 1 else ''))
        if opt('--labels'):
            with open(opt('--labels'), 'w', encoding='utf-8') as f:
                for name, found in hits:
                    if len(found) == 1:
                        f.write(f'{img.label_addr(found[0])}\t{name}\tsignature from {os.path.basename(args[0])}\n')
            print(f'labels for the single matches written to {opt("--labels")}')
        print(f'{len(hits)} signatures matched')
        return 0
    img = Image(args[0])
    length = int(opt('--len', '32'))
    todo = []
    if opt('--names'):
        for line in open(opt('--names'), encoding='utf-8'):
            parts = line.rstrip('\n').split('\t')
            if len(parts) >= 2 and parts[0] and not parts[0].startswith('#'):
                todo.append((parse_addr(parts[0], img), parts[1], length))
    for a in args[1:]:
        addr, _, n = a.partition(':') if img.kind != 'mz' or a.count(':') != 1 else (a, '', '')
        todo.append((parse_addr(addr, img), f'sub_{addr}', int(n) if n else length))
    print(f'# signatures made from {os.path.basename(args[0])} by tools/triage/sigscan.py')
    for addr, name, n in todo:
        try:
            print(f'{name}: {make(img, addr, n)}')
        except SystemExit as e:
            print(f'# {name}: {e}')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
