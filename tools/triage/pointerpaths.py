#!/usr/bin/env python3
"""Find a way to the address of a value that moves: a chain of pointers from a fixed address
(a global variable of the program) through the game's objects to the value.

Usage: pointerpaths.py DUMP@BASE:TARGET [DUMP@BASE:TARGET ...] --static LO-HI
                       [--depth N] [--offset N] [--size 4|8|2] [--be] [--max N]

  DUMP@BASE:TARGET  a memory dump, the address of its first byte, and the address of the value
                    in that run (found with memscan.py), for example derived/mem/run1.bin@0x400000:0x1234560
  --static LO-HI    the address range that does not move between runs: the program's data
                    segment or its image, for example 0x400000-0x4fffff
  --depth N         the longest chain tried (default 4 pointers)
  --offset N        the largest offset from a pointer to the field it leads to (default 0x400)
  --size N          bytes in a pointer: 4 (default), 8 for 64-bit programs, 2 for 16-bit near pointers
  --be              big-endian pointers (68000, PowerPC, MIPS consoles)

A value inside an object on the heap lives at a different address in every run, so its address
alone cannot be used by a test or an object model. A chain such as "the pointer at 0x4a1f30,
then +0x18, then +0x4" finds it in every run. This tool searches backwards from the value: the
pointers that point at it or just before it, then the pointers to those, until it reaches the
static range. It then follows each chain in the other dumps, taken in other runs where the
value sat at another address, and keeps only the chains that lead to the value in all of them.

The chain is also the shape of the game's data: the first pointer is often a global (the
world, the player list), and each offset a field of a structure. Write each step into the
labels and the structure notes (docs/observe/memory.md). Standard library only.
"""
import bisect
import struct
import sys


class Dump:
    def __init__(self, spec, size, order):
        path, _, rest = spec.rpartition('@')
        base, _, target = rest.partition(':')
        with open(path, 'rb') as f:
            self.data = f.read()
        self.path, self.base, self.target = path, int(base, 0), int(target, 0)
        self.size, self.fmt = size, order + {2: 'H', 4: 'I', 8: 'Q'}[size]

    def read(self, addr):
        at = addr - self.base
        if at < 0 or at + self.size > len(self.data):
            return None
        return struct.unpack_from(self.fmt, self.data, at)[0]

    def pointers(self):
        """(value, address) for every aligned word, sorted by value."""
        n = len(self.data) // self.size
        vals = struct.unpack(self.fmt[0] + self.fmt[1] * n, self.data[:n * self.size])
        return sorted((v, self.base + i * self.size) for i, v in enumerate(vals) if v)


def search(dump, lo, hi, depth, max_offset, cap=20000):
    """Chains as (static address, [offset after each pointer]), shortest first."""
    table = dump.pointers()
    values = [v for v, _ in table]
    found = []
    frontier = {dump.target: []}           # address -> offsets from there to the value
    for _ in range(depth):
        nxt = {}
        for addr, tail in frontier.items():
            i = bisect.bisect_left(values, addr - max_offset)
            while i < len(values) and values[i] <= addr:
                v, where = table[i]
                chain = [addr - v] + tail
                if lo <= where <= hi:
                    found.append((where, chain))
                elif where not in nxt:
                    nxt[where] = chain
                i += 1
        frontier = dict(list(nxt.items())[:cap])
        if not frontier:
            break
    return found


def follow(dump, start, offsets):
    addr = start
    for off in offsets:
        v = dump.read(addr)
        if v is None:
            return None
        addr = v + off
    return addr


def main(argv):
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__)
        return 0 if argv else 2
    opt = lambda name, default=None: argv[argv.index(name) + 1] if name in argv else default
    taken = {i + 1 for i, a in enumerate(argv) if a in ('--static', '--depth', '--offset', '--size', '--max')}
    specs = [a for i, a in enumerate(argv) if not a.startswith('--') and i not in taken]
    size = int(opt('--size', '4'))
    order = '>' if '--be' in argv else '<'
    lo, hi = (int(x, 0) for x in opt('--static', '0-0').split('-'))
    dumps = [Dump(s, size, order) for s in specs]
    chains = search(dumps[0], lo, hi, int(opt('--depth', '4')), int(opt('--offset', '0x400'), 0))
    print(f'{dumps[0].path}: {len(chains)} chains from the static range to 0x{dumps[0].target:x}')
    for d in dumps[1:]:
        chains = [(s, offs) for s, offs in chains if follow(d, s, offs) == d.target]
        print(f'{d.path}: {len(chains)} chains still lead to 0x{d.target:x}')
    chains.sort(key=lambda c: (len(c[1]), sum(c[1])))
    for start, offs in chains[:int(opt('--max', '20'))]:
        print(f'[0x{start:x}]' + ''.join(f' -> +0x{o:x}' for o in offs))
    if not chains:
        print('no chain: allow a deeper chain (--depth) or larger offsets (--offset), or check the static range')
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
