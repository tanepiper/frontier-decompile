#!/usr/bin/env python3
"""Find where a running game keeps a value, from memory dumps taken while you watch the value
change: the gold, a unit's health, the turn counter, the random seed.

Usage: memscan.py DUMP:RULE [DUMP:RULE ...] [--type T] [--base ADDR] [--max N]
       memscan.py DUMP --pointers ADDR [--base ADDR] [--type u16|u32|u64]

  DUMP:RULE    a dump file and what the value was when it was taken. RULE is one of
                 =N        the value was N (500, -3, 0x1f4, 2.5)
                 changed   different from the previous dump
                 same      equal to the previous dump
                 up, down  larger or smaller than in the previous dump
                 any       no rule (a starting dump)
  --type T     u8 i8 u16 i16 u32 i32 u64 f32 f64 (default u16 for a dump under 1 MB, else
               u32); add "be" for big-endian, as in u16be
  --base ADDR  the address of the dump's first byte, so results print as addresses
  --pointers   list every place in DUMP that holds ADDR: pointers to a value found with a scan,
               which lead to the structure around it

Start with =N when you know the value: the first dump is then searched as bytes, which is
fast. Starting with `any` checks every offset, which takes seconds per megabyte.

Each dump must cover the same memory: the same segment, the same heap range, or the whole
emulated memory. Take the first one, change the value in the game (spend gold, take damage),
take another, and so on. Every offset whose value fits every rule is printed. Three or four
dumps usually leave a handful. The tool scans every alignment, so a value inside a packed
structure is found too.

Then confirm the address. Put a memory breakpoint on it (DOSBox-X: BPM; tools/dos/dbx_control.py),
or find the code that refers to it in the Ghidra export (decomp_query.py). The instruction
that writes it is the rule to trace. Dumps come from DOSBox-X's debugger (MEMDUMPBIN), an
emulator's save state, tools/emu, or a debugger on the original. They are game material:
keep them in derived/. Standard library only.
"""
import struct
import sys

TYPES = {'u8': 'B', 'i8': 'b', 'u16': 'H', 'i16': 'h', 'u32': 'I', 'i32': 'i', 'u64': 'Q',
         'i64': 'q', 'f32': 'f', 'f64': 'd'}


def parse_rule(rule, is_float):
    if rule.startswith('='):
        text = rule[1:]
        return ('eq', float(text) if is_float else int(text, 0))
    if rule in ('changed', 'same', 'up', 'down', 'any'):
        return (rule, None)
    raise SystemExit(f'unknown rule {rule!r}: use =N, changed, same, up, down or any')


def fits(rule, arg, now, before, is_float):
    if rule == 'any':
        return True
    if rule == 'eq':
        return abs(now - arg) <= 1e-4 * max(1.0, abs(arg)) if is_float else now == arg
    if before is None:
        raise SystemExit(f'the rule {rule!r} needs a dump before it')
    return {'changed': now != before, 'same': now == before,
            'up': now > before, 'down': now < before}[rule]


def type_fmt(name):
    big = name.endswith('be')
    base = name[:-2] if big else name
    if base not in TYPES:
        raise SystemExit(f'unknown type {name}')
    return ('>' if big else '<') + TYPES[base], base.startswith('f')


def main(argv):
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0 if argv else 2
    opt = lambda name, default=None: argv[argv.index(name) + 1] if name in argv else default
    base = int(opt('--base', '0'), 0)
    limit = int(opt('--max', '50'))
    taken = {i + 1 for i, a in enumerate(argv) if a in ('--type', '--base', '--max', '--pointers')}
    specs = [a for i, a in enumerate(argv) if not a.startswith('--') and i not in taken]

    if '--pointers' in argv:
        with open(specs[0], 'rb') as f:
            data = f.read()
        fmt, _ = type_fmt(opt('--type', 'u32'))
        target = int(opt('--pointers'), 0)
        needle, hits = struct.pack(fmt, target), []
        at = data.find(needle)
        while at >= 0:
            hits.append(at)
            at = data.find(needle, at + 1)
        for i in hits[:limit]:
            print(f'0x{base + i:08x}')
        print(f'{len(hits)} places hold 0x{target:x}')
        return 0

    dumps = []
    for spec in specs:
        path, _, rule = spec.rpartition(':')
        if not path:
            path, rule = spec, 'any'
        with open(path, 'rb') as f:
            dumps.append((path, rule, f.read()))
    sizes = {len(d) for _, _, d in dumps}
    if len(sizes) != 1:
        raise SystemExit(f'the dumps differ in size {sorted(sizes)}: they must cover the same memory')
    tname = opt('--type', 'u16' if sizes.pop() < 1 << 20 else 'u32')
    fmt, is_float = type_fmt(tname)

    size = struct.calcsize(fmt)
    alive = None
    before = None
    for path, rule, data in dumps:
        kind, arg = parse_rule(rule, is_float)
        if alive is None and kind == 'eq' and not is_float:
            # A known first value: find its bytes, which is fast even in a large dump.
            needle, alive, at = struct.pack(fmt, arg), [], data.find(struct.pack(fmt, arg))
            while at >= 0:
                alive.append(at)
                at = data.find(needle, at + 1)
        else:
            if alive is None:
                alive = range(len(data) - size + 1)
            get = lambda d, i: struct.unpack_from(fmt, d, i)[0]
            alive = [i for i in alive
                     if fits(kind, arg, get(data, i), get(before, i) if before is not None else None, is_float)]
        print(f'{path}: {rule:10s} {len(alive):10,d} candidates')
        before = data
    if not alive:
        print('nothing fits every rule: check the type (--type), the byte order, and that the dumps cover the same memory')
        return 1
    print()
    for i in alive[:limit]:
        trail = ' -> '.join(str(struct.unpack_from(fmt, d, i)[0]) for _, _, d in dumps)
        print(f'0x{base + i:08x}  {tname}  {trail}')
    if len(alive) > limit:
        print(f'... {len(alive) - limit} more: take another dump with a new value')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
