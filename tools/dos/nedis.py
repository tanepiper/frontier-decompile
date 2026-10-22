#!/usr/bin/env python3
"""Disassemble 16-bit code with its relocations applied and its names shown.

Usage: nedis.py BINARY SEG:OFF|LABEL [LENGTH] [--labels FILE]

Ghidra's listing aside, a plain disassembler shows a 16-bit program's calls to other segments
and to imports as `lcall 0:ffff`: the file holds placeholders that the loader fills in. This
tool loads the program the way tools/emu/dos_emu/ does (relocations applied, Ghidra's
selectors, imports as named stubs) and disassembles the result with capstone. Call and jump
targets, and data offsets in DS, are replaced by names from a labels file (labels/names.tsv
by default) where one matches: `lcall Reset_1040_0a12`, `mov al, byte ptr [PlayDemoNum]`.

LENGTH is in bytes (default 0x100). Addresses are Ghidra's (1000:7f40). Needs capstone and
unicorn (GameFrame's .venv). It does not follow jumps or find function ends.
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, '..', 'emu'))
import capstone  # noqa: E402
from dos_emu import DosEmu  # noqa: E402


def load_labels(path):
    labels = {}
    if os.path.exists(path):
        for line in open(path, encoding='utf-8'):
            if line.startswith('#') or '\t' not in line:
                continue
            addr, name = line.split('\t')[:2]
            if ':' in addr:
                seg, off = addr.split(':')
                labels[(int(seg, 16), int(off, 16))] = name
    return labels


def main(argv):
    if len(argv) < 2 or argv[0] in ('-h', '--help'):
        print(__doc__)
        sys.exit(2)
    labels_path = 'labels/names.tsv'
    if '--labels' in argv:
        i = argv.index('--labels')
        labels_path = argv[i + 1]
        argv = argv[:i] + argv[i + 2:]
    path, where = argv[0], argv[1]
    length = int(argv[2], 0) if len(argv) > 2 else 0x100
    emu = DosEmu(path)
    labels = load_labels(labels_path)
    by_name = {v.lower(): k for k, v in labels.items()}
    if where.lower() in by_name:
        seg, off = by_name[where.lower()]
    else:
        seg, off = (int(x, 16) for x in where.replace('_', ':').split(':')[-2:])
    code = emu.read((seg, off), length)
    ds = emu.regs['ds']
    imports = {o: k for o, k in emu.imports.items()}
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_16)

    def name_far(s, o):
        if s == emu.stub_seg and o in imports:
            return imports[o]
        return labels.get((s, o), f'{s:04x}:{o:04x}')

    for ins in md.disasm(code, off):
        text = ins.op_str
        m = re.fullmatch(r'(0x[0-9a-f]+|\d+), (0x[0-9a-f]+|\d+)', text)
        if ins.mnemonic in ('lcall', 'ljmp') and m:
            text = name_far(int(m.group(1), 0), int(m.group(2), 0))
        elif ins.mnemonic in ('call', 'jmp') and re.fullmatch(r'0x[0-9a-f]+', text):
            target = int(text, 16)
            if (seg, target) in labels:
                text = f'{labels[(seg, target)]} ({text})'
        elif '[' in text and 'bp' not in text and 'ss:' not in text and 'cs:' not in text:
            def sub(mm):
                v = int(mm.group(1), 0)
                return f'[{labels[(ds, v)]}]' if (ds, v) in labels else mm.group(0)
            # Only absolute operands ([0x12de]) name a DS global: in [si + 0x10] the constant
            # is a field or element offset from a register, not an address.
            text = re.sub(r'\[(0x[0-9a-f]+|\d+)\]', sub, text)
        print(f'{seg:04x}:{ins.address:04x}  {ins.mnemonic:<7} {text}')


if __name__ == '__main__':
    main(sys.argv[1:])
