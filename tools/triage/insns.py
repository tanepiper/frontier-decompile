#!/usr/bin/env python3
"""List every instruction of a kind in a program: every port read or write, every interrupt,
every timer read, every division, every far call. Then break on each one, or read them all.

Usage: insns.py BINARY PATTERN [PATTERN ...] [--functions functions.tsv] [--json]
       insns.py RAW PATTERN ... --raw --bits 16|32|64 --base ADDR

  PATTERN     a regular expression matched against "mnemonic operands", as capstone writes
              them: '^in ', '^out ', '^int 0x21', '^rdtsc', '^f?div', 'cs:', '^lcall'
  --functions a Ghidra export's functions.tsv, to name the function each instruction is in
  BINARY      a Windows PE (its code sections, virtual addresses) or a DOS MZ program (its
              load image, addresses as Ghidra's 1000:offset); --raw for any other code

Some questions are answered by a short list. A game that reads the joystick does it with
`in al, dx` at port 0x201; one that programs the timer writes port 0x40 and 0x43; one that
talks to a Sound Blaster writes ports 0x220 to 0x22F; one that asks the BIOS for the time
calls interrupt 0x1A; one that measures the CPU uses rdtsc. A game's floating point is a
handful of fdiv and fsqrt instructions. List them, set a breakpoint on each, and the one
that stops when the thing happens is the one to trace. Groups that matter:

  '^(in|out|ins|outs)'      hardware ports: the docs/observe/inputs.md tables name them
  '^int '                   BIOS and DOS services; the AH value set just before says which
  '^(rdtsc|cpuid)'          CPU timing and identification (also anti-debugging checks)
  '^(f|)i?div'              divisions, where rounding matters to a port

The disassembly is a linear sweep of the code sections, so data inside code can show as stray
instructions: check each hit in Ghidra. Needs capstone (.venv).
"""
import bisect
import json
import os
import re
import sys

import capstone

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from sigscan import Image  # noqa: E402


def functions(path):
    rows = []
    for line in open(path, encoding='utf-8', errors='replace'):
        parts = line.rstrip('\n').split('\t')
        if len(parts) >= 3:
            a = parts[0]
            addr = (int(a.split(':')[0], 16) - 0x1000) * 16 + int(a.split(':')[1], 16) if ':' in a else int(a, 16)
            rows.append((addr, parts[2]))
    rows.sort()
    return rows


def main(argv):
    if len(argv) < 2 or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0 if argv and argv[0] in ('-h', '--help') else 2
    opt = lambda name, default=None: argv[argv.index(name) + 1] if name in argv else default
    taken = {i + 1 for i, a in enumerate(argv) if a in ('--functions', '--bits', '--base')}
    args = [a for i, a in enumerate(argv) if not a.startswith('--') and i not in taken]
    img = Image(args[0], int(opt('--base', '0'), 0))
    if '--raw' in argv:
        img.regions = [(img.regions[0][0], int(opt('--base', '0'), 0), int(opt('--bits', '16')))]
        img.kind = 'raw'
    patterns = [re.compile(p) for p in args[1:]]
    funcs = functions(opt('--functions')) if opt('--functions') else []
    starts = [a for a, _ in funcs]
    rows = []
    for data, start, bits in img.regions:
        md = capstone.Cs(capstone.CS_ARCH_X86, {16: capstone.CS_MODE_16, 32: capstone.CS_MODE_32,
                                                64: capstone.CS_MODE_64}[bits])
        md.skipdata = True
        for ins in md.disasm(data, start):
            text = f'{ins.mnemonic} {ins.op_str}'.strip()
            if any(p.search(text) for p in patterns):
                i = bisect.bisect_right(starts, ins.address) - 1
                rows.append({'address': img.label_addr(ins.address), 'instruction': text,
                             'function': funcs[i][1] if i >= 0 else ''})
    if '--json' in argv:
        print(json.dumps(rows, indent=2))
        return 0
    for r in rows:
        print(f'{r["address"]:>14s}  {r["instruction"]:36s} {r["function"]}')
    print(f'{len(rows)} instructions')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
