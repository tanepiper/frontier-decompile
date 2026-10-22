#!/usr/bin/env python3
"""Mark the fields of a data file with unique values, then find where the game puts each one in
memory. It maps a save or data file onto the game's structures.

Usage: markers.py plant FILE OUT [--every N] [--size 2|4] [--from A] [--to B] [--be]
       markers.py find  OUT.markers DUMP[@BASE] [DUMP@BASE ...]

  plant   copy FILE to OUT with a marker in each chosen field, and write OUT.markers listing
          each marker and its offset. --every N (default 4) marks every N bytes between
          --from and --to (default: the whole file); --size is the marker's width.
  find    list where each marker appears in one or more memory dumps taken after the game
          has loaded OUT: the file offset, the marker, and each address it was found at.

A tracer in a medical scan shows where the blood goes. A marker does the same for a value
the game reads from a file: a number that occurs nowhere else, written into the field. After
the game loads the marked file, the marker turns up in memory at the address where the game
keeps that field, often inside an object whose other fields came from the same record. A
memory breakpoint on that address (docs/observe/debuggers.md) then finds the code that uses
the field. Each marker encodes its own offset (0xC0DE0000 + the field's number for 4 bytes,
0xA000 + the number for 2 bytes), so a match names its field.

A game may check the file (a checksum, a version, a count that must be small) and refuse it, or
crash on a marker in a field such as a count or an offset. Plant markers in a few fields at a
time, keep the header untouched (--from), and fix the checksum if the game checks one. Work on
copies in derived/: marked files and dumps are game material. Standard library only.
"""
import struct
import sys


def plant(src, out, every, size, lo, hi, order):
    with open(src, 'rb') as f:
        data = bytearray(f.read())
    hi = min(hi if hi is not None else len(data), len(data))
    fmt = order + ('I' if size == 4 else 'H')
    base = 0xC0DE0000 if size == 4 else 0xA000
    limit = 0xFFFF if size == 4 else 0x0FFF
    rows = []
    for n, at in enumerate(range(lo, hi - size + 1, every)):
        if n > limit:
            break
        value = base + n
        struct.pack_into(fmt, data, at, value)
        rows.append((at, value))
    with open(out, 'wb') as f:
        f.write(data)
    with open(out + '.markers', 'w') as f:
        f.write(f'# markers planted by tools/formats/markers.py in {out}: offset, value, size {size}, {fmt}\n')
        for at, value in rows:
            f.write(f'0x{at:x}\t0x{value:x}\t{fmt}\n')
    print(f'{len(rows)} markers written into {out}; the list is {out}.markers')


def find(listing, dumps):
    rows = []
    for line in open(listing):
        if line.startswith('#') or not line.strip():
            continue
        off, value, fmt = line.split()
        rows.append((int(off, 0), int(value, 0), fmt))
    found_any = 0
    for off, value, fmt in rows:
        needle = struct.pack(fmt, value)
        places = []
        for spec in dumps:
            path, _, base = spec.partition('@')
            with open(path, 'rb') as f:
                data = f.read()
            at = data.find(needle)
            while at >= 0:
                places.append(f'0x{int(base or "0", 0) + at:x}' + (f' ({path})' if len(dumps) > 1 else ''))
                at = data.find(needle, at + 1)
        if places:
            found_any += 1
            print(f'file 0x{off:06x}  marker 0x{value:x}  ->  ' + ', '.join(places[:6])
                  + (' ...' if len(places) > 6 else ''))
    print(f'{found_any} of {len(rows)} markers found in memory')


def main(argv):
    if len(argv) < 3 or argv[0] not in ('plant', 'find'):
        print(__doc__)
        return 0 if argv and argv[0] in ('-h', '--help') else 2
    opt = lambda name, default=None: argv[argv.index(name) + 1] if name in argv else default
    if argv[0] == 'plant':
        plant(argv[1], argv[2], int(opt('--every', '4'), 0), int(opt('--size', '4')),
              int(opt('--from', '0'), 0), int(opt('--to'), 0) if opt('--to') else None,
              '>' if '--be' in argv else '<')
    else:
        find(argv[1], argv[2:])
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
